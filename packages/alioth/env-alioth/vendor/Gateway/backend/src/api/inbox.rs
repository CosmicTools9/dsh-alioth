//! Inbox HTTP Handler — Gateway thin adapter
//!
//! 业务逻辑在 framework-inbox crate 中。
//! 本层仅负责：提取 HttpRequest 上下文 → 调用 Framework 服务 → 映射 HTTP 响应。

use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use serde::Deserialize;

use framework_inbox::{InboxActionResponse, InboxService, SendMessageRequest};

/// POST /api/messages — 发送站内信
pub async fn send_message(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    body: web::Json<SendMessageRequest>,
) -> HttpResponse {
    let user_id = match req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id)
    {
        Some(id) => id,
        None => return HttpResponse::Unauthorized().json(InboxActionResponse::fail("未认证用户")),
    };

    let resp = InboxService::send(pool.get_ref(), user_id, body.into_inner()).await;
    if resp.success {
        HttpResponse::Ok().json(resp)
    } else {
        HttpResponse::BadRequest().json(resp)
    }
}

/// POST /api/messages/{id}/reply — 回复站内信
#[derive(Deserialize)]
pub struct ReplyMessageRequest {
    content: String,
}

pub async fn reply_message(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    path: web::Path<i64>,
    body: web::Json<ReplyMessageRequest>,
) -> HttpResponse {
    let parent_id = path.into_inner();
    let user_id = match req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id)
    {
        Some(id) => id,
        None => return HttpResponse::Unauthorized().json(InboxActionResponse::fail("未认证用户")),
    };

    // 读取原消息的 title 和发件人作为 recipients
    let row: Option<(String, i64)> = sqlx::query_as(
        "SELECT notice, created_by_id FROM isahl.zc_id_message WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(parent_id)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);

    let (title, original_sender_id) = match row {
        Some((t, s)) => (t, s),
        None => return HttpResponse::BadRequest().json(InboxActionResponse::fail("原消息不存在")),
    };

    // reply 使用原消息的发件人作为唯一收件人
    let send_req = SendMessageRequest {
        title,
        content: body.content.clone(),
        recipient_ids: vec![original_sender_id],
        previous_id: Some(parent_id),
    };

    let resp = InboxService::send(pool.get_ref(), user_id, send_req).await;
    if resp.success {
        HttpResponse::Ok().json(resp)
    } else {
        HttpResponse::BadRequest().json(resp)
    }
}

/// PATCH /api/messages/{id}/read — 标记消息为已读
pub async fn mark_message_read(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    path: web::Path<i64>,
) -> HttpResponse {
    let msg_id = path.into_inner();
    let user_id = match req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id)
    {
        Some(id) => id,
        None => return HttpResponse::Unauthorized().json(InboxActionResponse::fail("未认证用户")),
    };

    let resp = InboxService::mark_read(pool.get_ref(), msg_id, user_id).await;
    if resp.success {
        HttpResponse::Ok().json(resp)
    } else {
        HttpResponse::BadRequest().json(resp)
    }
}

/// DELETE /api/messages/{id} — 软删除消息（仅创建者）
pub async fn delete_message(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    path: web::Path<i64>,
) -> HttpResponse {
    let msg_id = path.into_inner();
    let user_id = match req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id)
    {
        Some(id) => id,
        None => return HttpResponse::Unauthorized().json(InboxActionResponse::fail("未认证用户")),
    };

    let resp = InboxService::delete(pool.get_ref(), msg_id, user_id).await;
    if resp.success {
        HttpResponse::Ok().json(resp)
    } else {
        HttpResponse::NotFound().json(resp)
    }
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/messages")
            .route("", web::post().to(send_message))
            .route("/{id}/reply", web::post().to(reply_message))
            .route("/{id}/read", web::patch().to(mark_message_read))
            .route("/{id}", web::delete().to(delete_message)),
    );
}
