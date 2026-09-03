//! 转交/加签 Handler — P1 实现
//!
//! 注册 `POST /approval-instances/{id}/transfer` 和 `POST /approval-instances/{id}/cc`。
//! 记录审批意见明细（zc_id_deta-opinion）并更新实例审批人（transfer）。
//!
//! ## NGAC 说明
//! 当前复用已注册的 "approve" action；待 ngac_access_right 注册 "transfer"/"cc" 后替换。
//!
//! ## 路由
//! - `POST /approval-instances/{id}/transfer` — 转交
//! - `POST /approval-instances/{id}/cc`       — 加签
//!
//! ## 数据存储
//! `ak_forwarding` 列存储转交目标用户 ID，`ak_addition` 列存储加签目标用户 ID。
//! 意见文本存入 `comments` 字段。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context;
use common::error::AliothError as ApiError;
use common::messaging::MessagingService;
use common::permissions::require_resource_access;
use common::ApiResponse;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;

use super::notify;

#[derive(Debug, Deserialize)]
pub struct TargetBody {
    #[serde(with = "common::serde_zuid")]
    pub target_id: i64,
    pub opinion: Option<String>,
}

const TRANSFER_NOTICE: &str = "审批转交";
const CC_NOTICE: &str = "审批加签";

// snapshot_comments 已被 DDL 改造替代——直接使用 ak_forwarding/ak_addition 定列

/// POST /approval-instances/{id}/transfer
pub async fn transfer(
    pool: web::Data<PgPool>,
    messaging: web::Data<Arc<dyn MessagingService>>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<TargetBody>,
) -> Result<HttpResponse, ApiError> {
    let instance_id = path.into_inner();
    let user_id = context::require_auth(&req)?;
    require_resource_access(
        pool.get_ref(),
        user_id,
        "approval-instances",
        instance_id,
        "transfer",
    )
    .await?;
    let opinion = body.opinion.as_deref().unwrap_or("");
    let target_id = body.target_id;

    // 校验实例存在（转交/加签意见挂实例 id，非事件 id——fk_index 契约）
    sqlx::query_scalar::<_, i64>(
        r#"SELECT id FROM isahl."zc_id_oper-approve"
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_optional(&**pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("ApprovalInstance {} not found", instance_id)))?;

    // 转交：将当前审批人改派给目标人
    sqlx::query(
        r#"UPDATE isahl."zc_id_oper-approve"
           SET fk_operator = $1, updated_at = NOW()
           WHERE id = $2"#,
    )
    .bind(target_id)
    .bind(instance_id)
    .execute(&**pool)
    .await?;

    // 记录转交动作（使用 ak_forwarding 列存储目标审批人）
    let date_anchor = super::approve_reject::today_date_anchor(&pool).await?;
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_deta-opinion"
           (id, notice, opinion, fk_list, fk_biller, ak_forwarding, qk_date, created_at)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, NOW())"#,
    )
    .bind(TRANSFER_NOTICE)
    .bind(opinion)
    .bind(instance_id)
    .bind(user_id)
    .bind(vec![target_id]) // ak_forwarding 为 bigint[]
    .bind(date_anchor)
    .execute(&**pool)
    .await?;

    // P2-8：向转交目标投递系统通知（失败仅 warn 不阻断）
    let title = notify::instance_title(pool.get_ref(), instance_id)
        .await
        .unwrap_or_else(|| format!("审批实例 #{}", instance_id));
    notify::notify_user(
        messaging.get_ref(),
        target_id,
        "审批转交通知",
        &format!("「{}」已转交由你审批，请及时处理。", title),
    )
    .await;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "id": instance_id,
            "status": "transferred",
            "to_user": target_id,
        }))),
    )
}

/// POST /approval-instances/{id}/cc
pub async fn cc(
    pool: web::Data<PgPool>,
    messaging: web::Data<Arc<dyn MessagingService>>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<TargetBody>,
) -> Result<HttpResponse, ApiError> {
    let instance_id = path.into_inner();
    let user_id = context::require_auth(&req)?;
    require_resource_access(
        pool.get_ref(),
        user_id,
        "approval-instances",
        instance_id,
        "cc",
    )
    .await?;
    let opinion = body.opinion.as_deref().unwrap_or("");
    let target_id = body.target_id;

    // 校验实例存在（加签意见挂实例 id，非事件 id——fk_index 契约）
    sqlx::query_scalar::<_, i64>(
        r#"SELECT id FROM isahl."zc_id_oper-approve"
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_optional(&**pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("ApprovalInstance {} not found", instance_id)))?;

    // 加签：记录加签人（使用 ak_addition 列存储目标用户）
    let date_anchor = super::approve_reject::today_date_anchor(&pool).await?;
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_deta-opinion"
           (id, notice, opinion, fk_list, fk_biller, ak_addition, qk_date, created_at)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, NOW())"#,
    )
    .bind(CC_NOTICE)
    .bind(opinion)
    .bind(instance_id)
    .bind(user_id)
    .bind(vec![target_id]) // ak_addition 为 bigint[]
    .bind(date_anchor)
    .execute(&**pool)
    .await?;

    // P2-8：向抄送目标投递系统通知（失败仅 warn 不阻断）
    let title = notify::instance_title(pool.get_ref(), instance_id)
        .await
        .unwrap_or_else(|| format!("审批实例 #{}", instance_id));
    notify::notify_user(
        messaging.get_ref(),
        target_id,
        "审批抄送通知",
        &format!("「{}」抄送给你知悉。", title),
    )
    .await;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "id": instance_id,
            "status": "cc",
            "cc_user": target_id,
        }))),
    )
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/approval-instances/{id}/transfer").route(web::post().to(transfer)))
        .service(web::resource("/approval-instances/{id}/cc").route(web::post().to(cc)));
}
