//! 审批提醒 Handler — 记录催办动作 + 通知投递
//!
//! `POST /approval-instances/{id}/remind` — 当前审批人催办提醒。
//! fix-approval-action-chain P2-8：催办从只留痕升级为留痕 + 系统通知投递
//! （MessagingService；失败仅 warn 不阻断——通知是增强投递，不是状态契约）。
use actix_web::{web, HttpRequest, HttpResponse};
use common::context;
use common::messaging::MessagingService;
use sqlx::PgPool;
use std::sync::Arc;

use crate::handlers::notify;

/// POST /api/service/approval/approval-instances/{id}/remind
pub async fn remind(
    pool: web::Data<PgPool>,
    messaging: web::Data<Arc<dyn MessagingService>>,
    path: web::Path<i64>,
    req: HttpRequest,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = context::require_auth(&req)?;
    let instance_id = path.into_inner();
    // 催办留痕（services 层收口；分层规约 BACKEND_FRAMEWORK §4.5）
    crate::services::record_remind(pool.get_ref(), instance_id, user_id)
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?;

    // P2-8：向当前审批人投递催办系统通知（fk_operator 回退 fk_subject）
    if let Some(operator) = notify::current_operator(pool.get_ref(), instance_id).await {
        let title = notify::instance_title(pool.get_ref(), instance_id)
            .await
            .unwrap_or_else(|| format!("审批实例 #{}", instance_id));
        notify::notify_user(
            messaging.get_ref(),
            operator,
            "审批催办提醒",
            &format!("「{}」有催办提醒，请及时处理。", title),
        )
        .await;
    }

    Ok(HttpResponse::Ok().json(serde_json::json!({ "success": true })))
}

/// 注册 /approval-instances/{id}/remind 路由（须在 approval_instance CRUD scope 之前）
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route("/approval-instances/{id}/remind", web::post().to(remind));
}
