//! 审批撤回 Handler（fix-approval-engine-semantics）
//!
//! `POST /approval-instances/{id}/withdraw` — 申请人撤回自己发起、仍在途的审批。
//!
//! 语义：
//! - 权限：require_auth + NGAC `withdraw` 动作 + 实例创建者本人（admin UA 成员豁免）。
//! - 状态守卫：仅 pending（无桥或桥为 pending）可撤回；已终态 → Validation 错误。
//! - 级联：沿 fk_previous 前向链收下游实例 + 每个链成员的同节点 pending 兄弟
//!   （会签/或签并行实例），全部写 withdrawn 桥并逐实例发布
//!   `ApprovalCompleted(result=withdrawn)`——业务订阅者据此回写单据状态
//!   （如 contract pending_approval → draft）。
//! - 历史链无 fk_previous（修复前数据）→ 级联退化为单实例撤回，不报错。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context;
use common::error::AliothError as ApiError;
use common::event_bus::DomainEventBus;
use common::permissions::require_resource_access;
use common::ApiResponse;
use sqlx::PgPool;
use std::collections::HashSet;
use std::sync::Arc;

use super::approve_reject::{publish_approval_completed, update_lifecycle_status};
use super::notify::{instance_title, notify_user};
use common::messaging::MessagingService;

/// 当前生命周期桥状态码（无桥 → None）
async fn current_status(pool: &PgPool, instance_id: i64) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, Option<String>>(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" ls
           JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
           WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL
           ORDER BY ls.created_at DESC LIMIT 1"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))
    .map(|r| r.flatten())
}

/// 是否 admin UA 成员（所有权豁免）
async fn is_admin(pool: &PgPool, user_id: i64) -> bool {
    sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS (
               SELECT 1 FROM isahl_auth.ngac_user_rr_attribute rel
               JOIN isahl_auth.ngac_user_attribute ua
                 ON ua.id = rel.fk_user_attribute AND ua.deleted_at IS NULL
               WHERE rel.fk_user = $1 AND ua.o_name = 'admin' AND rel.deleted_at IS NULL
           )"#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .unwrap_or(false)
}

/// 收集撤回级联集合：fk_previous 前向链 + 每个链成员的同节点 pending 兄弟
async fn collect_cascade(pool: &PgPool, root_id: i64) -> Result<Vec<i64>, ApiError> {
    let mut members: Vec<i64> = Vec::new();
    let mut seen: HashSet<i64> = HashSet::new();
    let mut frontier = vec![root_id];
    let mut depth = 0u32;
    const MAX_DEPTH: u32 = 200;

    while !frontier.is_empty() && depth < MAX_DEPTH {
        depth += 1;
        let mut next = Vec::new();
        for id in frontier {
            if !seen.insert(id) {
                continue;
            }
            members.push(id);
            // 前向链：fk_previous = 当前成员
            let children: Vec<i64> = sqlx::query_scalar(
                r#"SELECT id FROM isahl."zc_id_oper-approve"
                   WHERE fk_previous = $1 AND deleted_at IS NULL"#,
            )
            .bind(id)
            .fetch_all(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?;
            next.extend(children);
        }
        frontier = next;
    }

    // 同节点 pending 兄弟清扫（会签/或签并行实例共享 fk_previous 或同为链首，
    // 前向链不一定覆盖——按节点的在途集合补齐）
    let mut all: HashSet<i64> = members.iter().copied().collect();
    for id in &members {
        if let Some(node_id) = sqlx::query_scalar::<_, Option<i64>>(
            r#"SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
               WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
               ORDER BY oe.created_at LIMIT 1"#,
        )
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?
        .flatten()
        {
            let pending: Vec<i64> = sqlx::query_scalar(
                r#"SELECT oa.id FROM isahl."zc_id_oper-approve" oa
                   JOIN isahl.zc_id_operation_rr_event oe
                     ON oe.ref_left = oa.id AND oe.ref_right = $1 AND oe.deleted_at IS NULL
                   WHERE oa.deleted_at IS NULL
                     AND NOT EXISTS (
                         SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                         JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
                         WHERE ls.ref_left = oa.id AND ls.deleted_at IS NULL
                           AND s.code IN ('approved','rejected','withdrawn','cancelled','abstained')
                     )"#,
            )
            .bind(node_id)
            .fetch_all(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?;
            all.extend(pending);
        }
    }

    let mut out: Vec<i64> = all.into_iter().collect();
    out.sort_unstable();
    Ok(out)
}

/// POST /approval-instances/{id}/withdraw
pub async fn withdraw(
    pool: web::Data<PgPool>,
    bus: web::Data<Arc<dyn DomainEventBus>>,
    messaging: web::Data<Arc<dyn MessagingService>>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let instance_id = path.into_inner();
    let user_id = context::require_auth(&req)?;
    require_resource_access(
        pool.get_ref(),
        user_id,
        "approval-instances",
        instance_id,
        "withdraw",
    )
    .await?;

    // 所有权守卫：仅实例创建者本人（admin 豁免）
    let owner: Option<i64> = sqlx::query_scalar(
        r#"SELECT created_by_id FROM isahl."zc_id_oper-approve"
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .ok_or_else(|| ApiError::NotFound(format!("ApprovalInstance {} not found", instance_id)))?;
    if owner != Some(user_id) && !is_admin(pool.get_ref(), user_id).await {
        return Err(ApiError::Forbidden(format!(
            "only the initiator can withdraw instance {}",
            instance_id
        )));
    }

    // 状态守卫：仅 pending 可撤回
    let status = current_status(pool.get_ref(), instance_id).await?;
    let is_pending = match &status {
        None => true,
        Some(s) => s == "pending" || s.is_empty(),
    };
    if !is_pending {
        return Err(ApiError::Validation {
            field: "status".into(),
            message: format!(
                "instance {} already in terminal status '{}', cannot withdraw",
                instance_id,
                status.unwrap_or_default()
            ),
        });
    }

    // 级联撤回：写 withdrawn 桥 + 逐实例事件
    let cascade = collect_cascade(pool.get_ref(), instance_id).await?;
    let title = instance_title(pool.get_ref(), instance_id)
        .await
        .unwrap_or_else(|| format!("审批实例 {}", instance_id));
    for id in &cascade {
        // 级联成员可能已终态（链上历史实例）——仅撤回仍在途者
        let member_status = current_status(pool.get_ref(), *id).await?;
        let member_pending = match &member_status {
            None => true,
            Some(s) => s == "pending" || s.is_empty(),
        };
        if member_pending {
            update_lifecycle_status(pool.get_ref(), *id, "withdrawn", "已撤回", user_id).await?;
            publish_approval_completed(
                bus.get_ref(),
                pool.get_ref(),
                *id,
                "withdrawn",
                Some("申请人撤回"),
            )
            .await;
        }
    }

    // 通知当前审批人「单据已撤回」（失败仅 warn 不阻断）
    if let Some(op) = super::notify::current_operator(pool.get_ref(), instance_id).await {
        if op != user_id {
            notify_user(
                messaging.get_ref(),
                op,
                "审批撤回通知",
                &format!("您待办的「{}」已被申请人撤回，无需处理。", title),
            )
            .await;
        }
    }

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "id": instance_id.to_string(),
            "status": "withdrawn",
            "withdrawn_count": cascade.len(),
        }))),
    )
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/approval-instances/{id}/withdraw",
        web::post().to(withdraw),
    );
}
