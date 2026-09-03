//! 审批通过/驳回 Handler — 自定义操作端点
//!
//! 注册 `POST /approval-instances/{id}/approve` 和 `POST /approval-instances/{id}/reject`。
//! 创建审批意见记录（zc_id_deta-opinion）并更新实例状态。
//!
//! 状态 ID 通过子查询从 zc_id_stus-operation 按 notice 获取，不硬编码。
//!
//! ## 路由
//! - `POST /approval-instances/{id}/approve` — 通过
//! - `POST /approval-instances/{id}/reject`  — 驳回

use crate::advance::advance_flow;
use actix_web::{web, HttpRequest, HttpResponse};
use common::context;
use common::error::AliothError as ApiError;
use common::event_bus::{DomainEvent, DomainEventBus};
use common::permissions::require_resource_access;
use common::ApiResponse;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct ApproveRejectBody {
    pub opinion: Option<String>,
}

/// 审批完成事件发布（PAGE-适航-07 闭环发布端）。
/// 审批通过/驳回时发布 `ApprovalCompleted`，payload 携带实例上下文：
/// - entity_id: 审批事件 ID（实例经 rr_event 桥反查）
/// - entity_type: 若审批事件 comments 为 JSON 且含 `entityType` 则透传（如
///   `airworthiness_certificate`），否则 "approval-instance"
/// - result / comment
///
/// airworthiness 订阅据此驱动证件状态流转（approved→Approved / rejected→Draft）。
/// 无消费者时 publish 为空操作（InMemoryEventBus 无订阅者即返回），零回归。
pub async fn publish_approval_completed(
    bus: &Arc<dyn DomainEventBus>,
    pool: &PgPool,
    instance_id: i64,
    result: &str,
    comment: Option<&str>,
) {
    // 审批事件 ID + comments（实体上下文）
    let ctx: Option<(Option<i64>, Option<String>)> = sqlx::query_as(
        r#"SELECT oe.ref_right, oa.comments FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe
             ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           WHERE oa.id = $1 AND oa.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let (event_id, comments_json) = ctx.unwrap_or((None, None));

    let entity_type = comments_json
        .as_deref()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok())
        .and_then(|v| {
            v.get("entityType")
                .and_then(|x| x.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| "approval-instance".to_string());

    let payload = serde_json::json!({
        "entity_type": entity_type,
        "entity_id": event_id.unwrap_or(instance_id),
        "result": result,
        "comment": comment,
    });
    let event = match DomainEvent::new("ApprovalCompleted", "commitment", instance_id, payload) {
        Ok(e) => e,
        Err(_) => return,
    };
    let _ = bus.publish("ApprovalCompleted", &event).await;
}

// 意见通知文本（模块私有常量，普通注释）
const APPROVE_NOTICE: &str = "审批通过";
const REJECT_NOTICE: &str = "审批驳回";

/// 时间锚（flow-process-continuity 规约）：审批动作意见/实例节点事件的
/// `qk_date` MUST 写入 `zc_id_scal-date` 日粒度标量引用（同日复用既有行）。
/// 时间线读路径 `COALESCE(sd.date, created_at)` 对存量 NULL 行保持回退。
pub(crate) async fn today_date_anchor(pool: &PgPool) -> Result<i64, ApiError> {
    let date_text = chrono::Utc::now().format("%Y-%m-%d").to_string();
    common::scalar::ScalarService::new(pool.clone())
        .find_or_create_date(&date_text)
        .await
        .map_err(|e| ApiError::Database(format!("qk_date scalar resolve failed: {}", e)))
}

/// POST /approval-instances/{id}/approve
pub async fn approve(
    pool: web::Data<PgPool>,
    bus: web::Data<Arc<dyn DomainEventBus>>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<ApproveRejectBody>,
) -> Result<HttpResponse, ApiError> {
    let instance_id = path.into_inner();
    let user_id = context::require_auth(&req)?;
    require_resource_access(
        pool.get_ref(),
        user_id,
        "approval-instances",
        instance_id,
        "approve",
    )
    .await?;
    let opinion = body.opinion.as_deref().unwrap_or("");
    approve_inner(pool.get_ref(), bus.get_ref(), instance_id, user_id, opinion).await?;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "id": instance_id.to_string(),
            "status": "approved",
        }))),
    )
}

/// 审批意见落库（fk_list = 实例 id，避免跨实例串扰；qk_date 写当日标量时间锚）
async fn insert_opinion(
    pool: &PgPool,
    instance_id: i64,
    notice: &str,
    opinion: &str,
    user_id: i64,
) -> Result<(), ApiError> {
    let date_anchor = today_date_anchor(pool).await?;
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_deta-opinion"
           (id, notice, opinion, fk_list, fk_biller, qk_date, created_at)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, NOW())"#,
    )
    .bind(notice)
    .bind(opinion)
    .bind(instance_id)
    .bind(user_id)
    .bind(Some(date_anchor))
    .execute(pool)
    .await?;
    Ok(())
}

/// 校验实例存在（批量/单条共用）
async fn ensure_instance_exists(pool: &PgPool, instance_id: i64) -> Result<(), ApiError> {
    sqlx::query_scalar::<_, i64>(
        r#"SELECT id FROM isahl."zc_id_oper-approve" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("ApprovalInstance {} not found", instance_id)))?;
    Ok(())
}

/// 终态动作守卫（fix-approval-engine-gap-closure D1.2）：仅无桥或桥为 pending 的
/// 实例可 approve/reject/abstain——终态实例（approved/rejected/withdrawn/
/// cancelled/abstained，含会签竞签取消方）返回 Validation，不写意见、不推进。
/// 与 withdraw.rs current_status 同模式（读生命周期主状态桥，非意见派生）。
async fn assert_pending(pool: &PgPool, instance_id: i64) -> Result<(), ApiError> {
    let code: Option<String> = sqlx::query_scalar(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" ls
           JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
           WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL
           ORDER BY ls.created_at DESC LIMIT 1"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten();
    if let Some(code) = code.filter(|c| c != "pending") {
        return Err(ApiError::Validation {
            field: "instance".into(),
            message: format!("实例已 {code}，不可再操作"),
        });
    }
    Ok(())
}

/// 审批通过全链（fix-approval-action-chain）：意见 → approved 生命周期桥 →
/// 流程推进 → ApprovalCompleted 事件。单条与批量端点共用——批量此前只写意见
/// （不推进/不搭桥/不发事件），是链断裂陷阱。
///
/// NGAC 校验不在此内：单条/批量失败语义不同（单条返回错误响应，批量记 failed 继续）。
pub(crate) async fn approve_inner(
    pool: &PgPool,
    bus: &Arc<dyn DomainEventBus>,
    instance_id: i64,
    user_id: i64,
    opinion: &str,
) -> Result<(), ApiError> {
    ensure_instance_exists(pool, instance_id).await?;
    assert_pending(pool, instance_id).await?;
    insert_opinion(pool, instance_id, APPROVE_NOTICE, opinion, user_id).await?;
    update_lifecycle_status(pool, instance_id, "approved", "已通过", user_id).await?;
    advance_flow(pool, instance_id, user_id, Some(bus)).await?;
    // G4 收口：employee-onboarding 副作用（员工创建/UA/profile/用户激活）已迁出，
    // 由 handlers::employee_onboarding 订阅者消费（见该模块文档）。
    publish_approval_completed(bus, pool, instance_id, "approved", Some(opinion)).await;
    Ok(())
}

/// 审批驳回全链（fix-approval-action-chain P0-3）：意见 → rejected 生命周期桥 →
/// ApprovalCompleted 事件。驳回不推进流程。
/// 此前 reject 缺失 lifecycle 桥写入（approve 写、SLA 自动驳回写、手动驳回不写），
/// 状态双源（意见派生 vs 桥）在手动驳回后漂移——本函数与 approve_inner 对称收口。
pub(crate) async fn reject_inner(
    pool: &PgPool,
    bus: &Arc<dyn DomainEventBus>,
    instance_id: i64,
    user_id: i64,
    opinion: &str,
) -> Result<(), ApiError> {
    ensure_instance_exists(pool, instance_id).await?;
    assert_pending(pool, instance_id).await?;
    insert_opinion(pool, instance_id, REJECT_NOTICE, opinion, user_id).await?;
    update_lifecycle_status(pool, instance_id, "rejected", "已拒绝", user_id).await?;
    // fix-approval-engine-semantics P1-4：任一驳回即节点驳回——会签/或签的
    // 其余在途兄弟实例取消（cancelled 桥）；sequential 无并发兄弟，调用为空操作。
    // 实例经 operation_rr_event 桥取节点事件模板（fk_approve 列已移除）
    if let Some(node_id) = sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT oe.ref_right FROM isahl."zc_id_operation_rr_event" oe
           JOIN isahl."zc_id_oper-approve" oa ON oa.id = oe.ref_left
           WHERE oa.id = $1 AND oa.deleted_at IS NULL AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await?
    .flatten()
    {
        crate::advance::cancel_pending_siblings(pool, node_id, instance_id, user_id).await?;
    }
    publish_approval_completed(bus, pool, instance_id, "rejected", Some(opinion)).await;
    Ok(())
}

/// POST /approval-instances/{id}/reject
pub async fn reject(
    pool: web::Data<PgPool>,
    bus: web::Data<Arc<dyn DomainEventBus>>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<ApproveRejectBody>,
) -> Result<HttpResponse, ApiError> {
    let instance_id = path.into_inner();
    let user_id = context::require_auth(&req)?;
    require_resource_access(
        pool.get_ref(),
        user_id,
        "approval-instances",
        instance_id,
        "approve",
    )
    .await?;
    let opinion = body.opinion.as_deref().unwrap_or("");
    // fix-approval-action-chain P0-3：reject_inner 补齐 rejected 生命周期桥
    // （此前 reject 只写意见+事件，桥缺失导致状态双源漂移）。
    reject_inner(pool.get_ref(), bus.get_ref(), instance_id, user_id, opinion).await?;
    // 2026-09-02 A3：vote 节点终态动作触发终局判定（quorum 未达全员已行动 → 自动驳回）
    crate::advance::vote_terminal_advance(
        pool.get_ref(),
        instance_id,
        user_id,
        Some(bus.get_ref()),
    )
    .await?;
    // 2026-09-02 A5：驳回路由（vote 表决与 stop 配置在 route_reject 内判定）
    crate::advance::route_reject(pool.get_ref(), instance_id, user_id).await?;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "id": instance_id.to_string(),
            "status": "rejected",
        }))),
    )
}

/// POST /approval-instances/{id}/abstain — 弃权（2026-09-02 A3）
///
/// 仅 vote 节点实例可弃权：写弃权意见 + 主状态桥 abstained（终态、不计 quorum），
/// 并触发 vote 终局判定（全员终态未达 quorum → 自动 rejected 终局，消除滞留）。
/// NGAC 复用 "approve" action（与 transfer_cc 同策略——ngac_access_right 未注册独立 action）。
pub async fn abstain(
    pool: web::Data<PgPool>,
    bus: web::Data<Arc<dyn DomainEventBus>>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<ApproveRejectBody>,
) -> Result<HttpResponse, ApiError> {
    let instance_id = path.into_inner();
    let user_id = context::require_auth(&req)?;
    require_resource_access(
        pool.get_ref(),
        user_id,
        "approval-instances",
        instance_id,
        "approve",
    )
    .await?;
    // 仅 vote 节点支持弃权（服务端权威；非 vote 明确报错）
    let cate: Option<String> = sqlx::query_scalar(
        r#"SELECT c.code FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_operation_rr_event oe2
             ON oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL
           JOIN isahl.zc_id_operation o ON o.id = oe2.ref_left AND o.tpl_id IS NULL
           LEFT JOIN isahl."zc_id_cate-proc_op" c
             ON c.id = o."ck_cate-proc_op" AND c.deleted_at IS NULL
           WHERE oa.id = $1 AND oa.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(instance_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten();
    if cate.as_deref() != Some("vote") {
        return Err(ApiError::Validation {
            field: "instance".into(),
            message: "仅投票（vote）节点实例支持弃权".into(),
        });
    }
    assert_pending(pool.get_ref(), instance_id).await?;
    let opinion = body.opinion.as_deref().unwrap_or("");
    insert_opinion(pool.get_ref(), instance_id, "弃权", opinion, user_id).await?;
    update_lifecycle_status(pool.get_ref(), instance_id, "abstained", "已弃权", user_id).await?;
    // 终局判定（与 reject 同策略；弃权事件不单独发布——无订阅者契约）
    crate::advance::vote_terminal_advance(
        pool.get_ref(),
        instance_id,
        user_id,
        Some(bus.get_ref()),
    )
    .await?;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "id": instance_id.to_string(),
            "status": "abstained",
        }))),
    )
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route("/approval-instances/{id}/approve", web::post().to(approve));
    cfg.route("/approval-instances/{id}/abstain", web::post().to(abstain));
    cfg.route("/approval-instances/{id}/reject", web::post().to(reject));
}

/// 更新生命周期主状态（zc_id_lifecycle_r_primary-status）
/// pub(crate)：SLA 超时自动驳回（sla_timeout.rs）复用同一链路
pub(crate) async fn update_lifecycle_status(
    pool: &PgPool,
    instance_id: i64,
    status_code: &str,
    status_notice: &str,
    user_id: i64,
) -> Result<(), ApiError> {
    // 1. 查找或创建状态记录
    let status_id: i64 = match sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT id FROM isahl."zc_id_stus-approve" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(status_code)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten()
    {
        Some(id) => id,
        None => {
            sqlx::query_scalar::<_, i64>(
                r#"INSERT INTO isahl."zc_id_stus-approve" (id, code, notice)
                   VALUES (isahl.gen_next_zuid(), $1, $2) RETURNING id"#,
            )
            .bind(status_code)
            .bind(status_notice)
            .fetch_one(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?
        }
    };

    // 2. UPSERT 生命周期主状态（三态：活跃行原地 UPDATE / 软删行 restore / 无行 INSERT）
    // ADR D-010：迁移前取 old（UPDATE 原地覆盖 ref_right）
    let row = crud::audit_outbox::fetch_primary_status_row(pool, instance_id)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
    let old_status = row.filter(|(_, active)| *active).map(|(s, _)| s);

    // D1 终态不变量（fix-approval-engine-gap-closure D1.1）：终态拒覆写守卫——
    // 旧状态 ∈ {approved,rejected,withdrawn,cancelled,abstained} 且新状态不同 →
    // Validation（不落桥不落审计）；同码幂等放行（重试/补偿路径）。
    if let Some(old_sid) = old_status {
        let old_code: Option<String> = sqlx::query_scalar(
            r#"SELECT code FROM isahl."zc_id_stus-approve"
               WHERE id = $1 AND deleted_at IS NULL LIMIT 1"#,
        )
        .bind(old_sid)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?
        .flatten();
        if let Some(code) = old_code {
            const TERMINAL: &[&str] = &[
                "approved",
                "rejected",
                "withdrawn",
                "cancelled",
                "abstained",
            ];
            if TERMINAL.contains(&code.as_str()) && code != status_code {
                return Err(ApiError::Validation {
                    field: "instance".into(),
                    message: format!("实例已终态({code})，禁止变更为 {status_code}"),
                });
            }
        }
    }

    match row {
        Some((_, true)) => {
            sqlx::query(
                r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
                   SET ref_right = $1, updated_at = NOW()
                   WHERE ref_left = $2 AND deleted_at IS NULL"#,
            )
            .bind(status_id)
            .bind(instance_id)
            .execute(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?;
        }
        Some((_, false)) => {
            sqlx::query(
                r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
                   SET ref_right = $1, deleted_at = NULL, updated_at = NOW()
                   WHERE ref_left = $2 AND deleted_at IS NOT NULL"#,
            )
            .bind(status_id)
            .bind(instance_id)
            .execute(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?;
        }
        None => {
            sqlx::query(
                r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (id, ref_left, ref_right)
                   VALUES (isahl.gen_next_zuid(), $1, $2)"#,
            )
            .bind(instance_id)
            .bind(status_id)
            .execute(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?;
        }
    }

    // ADR D-010：主状态变更审计（pool 直插；失败不回滚业务，warn 留证）
    if let Err(e) = crud::audit_outbox::audit_primary_status(
        pool,
        instance_id,
        old_status,
        status_id,
        Some(user_id),
    )
    .await
    {
        common::telemetry::warn!(
            "audit_primary_status enqueue failed (approval instance {}): {}",
            instance_id,
            e
        );
    }

    Ok(())
}
