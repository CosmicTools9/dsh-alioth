//! 出向调用方管理端点（gateway-openapi-outbound-unify）——Gateway 通用能力。
//!
//! `GET/POST /api/openapi/outbound-clients` + `PUT/DELETE /{id}`：
//! 跨 namespace 的出向对接配置管理（替代业务服务上的管理端点）。
//! 凭据明文仅创建时返回一次；轮换 version+1。经 NGAC 授权（管理员 OA）。

use actix_web::{web, HttpResponse};
use outbound_client::crypto;
use outbound_client::repository::OutboundRepository;
use sqlx::PgPool;

/// GET /api/openapi/outbound-clients — 列出出向调用方
pub async fn list_outbound_clients(
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, common::AliothError> {
    let repo = OutboundRepository::new(pool.get_ref().clone());
    let items = repo.list_clients().await?;
    Ok(HttpResponse::Ok().json(items))
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateOutboundClientRequest {
    pub code: String,
    #[serde(default = "default_provider")]
    pub provider: String,
    pub base_url: String,
    pub app_id: String,
    /// 明文 app_secret——服务端加密后入库，响应仅本次返回明文
    pub app_secret: String,
    pub tenant_id: Option<String>,
    pub account_id: Option<String>,
}

fn default_provider() -> String {
    "fssc".to_string()
}

/// POST /api/openapi/outbound-clients — 创建出向调用方
pub async fn create_outbound_client(
    pool: web::Data<PgPool>,
    body: web::Json<CreateOutboundClientRequest>,
) -> Result<HttpResponse, common::AliothError> {
    let b = body.into_inner();
    if b.code.trim().is_empty() || b.app_secret.trim().is_empty() {
        return Err(common::AliothError::BadRequest(
            "code 与 app_secret 必填".into(),
        ));
    }
    let secret_enc = crypto::encrypt(&b.app_secret)?;
    let repo = OutboundRepository::new(pool.get_ref().clone());
    let id = repo
        .create_client(
            &b.code,
            &b.provider,
            &b.base_url,
            &b.app_id,
            &secret_enc,
            b.tenant_id.as_deref().unwrap_or(""),
            b.account_id.as_deref().unwrap_or(""),
        )
        .await?;
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "id": id.to_string(),
        "code": b.code,
        "appSecret": b.app_secret,
        "warn": "密钥明文仅本次返回，请妥善保存",
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateOutboundClientRequest {
    pub base_url: Option<String>,
    pub app_id: Option<String>,
    /// 提供时触发凭据轮换（version+1）
    pub app_secret: Option<String>,
    pub tenant_id: Option<String>,
    pub account_id: Option<String>,
    pub enabled: Option<bool>,
}

/// PUT /api/openapi/outbound-clients/{id} — 更新出向调用方
pub async fn update_outbound_client(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<UpdateOutboundClientRequest>,
) -> Result<HttpResponse, common::AliothError> {
    let id = path.into_inner();
    let b = body.into_inner();
    let secret_enc = match &b.app_secret {
        Some(s) if !s.trim().is_empty() => Some(crypto::encrypt(s)?),
        _ => None,
    };
    let repo = OutboundRepository::new(pool.get_ref().clone());
    repo.update_client(
        id,
        b.base_url.as_deref(),
        b.app_id.as_deref(),
        secret_enc.as_deref(),
        b.tenant_id.as_deref(),
        b.account_id.as_deref(),
        b.enabled,
    )
    .await?;
    Ok(HttpResponse::Ok().json(serde_json::json!({ "id": id.to_string(), "updated": true })))
}

/// DELETE /api/openapi/outbound-clients/{id} — 软删出向调用方
pub async fn delete_outbound_client(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, common::AliothError> {
    let id = path.into_inner();
    let repo = OutboundRepository::new(pool.get_ref().clone());
    repo.delete_client(id).await?;
    Ok(HttpResponse::Ok().json(serde_json::json!({ "id": id.to_string(), "deleted": true })))
}

/// 注册出向调用方管理路由（挂 /api/openapi scope）
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/outbound-clients")
            .route("", web::get().to(list_outbound_clients))
            .route("", web::post().to(create_outbound_client))
            .route("/{id}", web::put().to(update_outbound_client))
            .route("/{id}", web::delete().to(delete_outbound_client)),
    );
}
