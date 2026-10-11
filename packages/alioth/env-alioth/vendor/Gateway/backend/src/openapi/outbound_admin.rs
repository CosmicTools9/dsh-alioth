//! 出向调用方管理端点（`gateway-openapi-outbound`）——Gateway 通用能力。
//!
//! `GET/POST /api/openapi/outbound-clients` + `PUT/DELETE /{id}`：
//! 跨 namespace 的出向对接配置管理（替代业务服务上的管理端点）。
//! 凭据明文仅创建/轮换时返回一次；轮换触发 version+1。经 NGAC 授权（管理员 OA）。
//!
//! 载荷为 **provider 中立**形态（`outbound-provider-abstraction`）：`provider`（派发键）/
//! `auth_type` / `base_url` / `config`（provider 私有配置 jsonb）——MUST NOT 出现任何
//! 单 provider 专属顶层字段。凭据密文仅由本面写入（明文经 `crypto::encrypt` 现加密入库）。

use actix_web::{web, HttpResponse};
use outbound_client::crypto;
use outbound_client::provider::AUTH_TYPE_NONE;
use outbound_client::repository::OutboundRepository;
use serde_json::Value;
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
    /// provider 派发键（如 `fssc` / `wechat-pay`）
    pub provider: String,
    /// 认证方案（缺省 `none` = 无认证）
    #[serde(default = "default_auth_type")]
    pub auth_type: String,
    #[serde(default)]
    pub base_url: String,
    /// provider 私有配置（jsonb；缺省 `{}`）
    #[serde(default)]
    pub config: Value,
    /// 明文凭据——服务端加密后入库，响应仅本次返回明文
    #[serde(default)]
    pub app_secret: Option<String>,
}

fn default_auth_type() -> String {
    AUTH_TYPE_NONE.to_string()
}

/// 非空 slug：`[a-z0-9-]+`，≤64 字符（code / provider / auth_type 共用形状判据；
/// auth_type 为 provider 自定义方案标识，非封闭枚举——仅 `none` 为规范值）。
fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// base_url 形状校验：空 = 合法（provider 私有配置可补）；非空 MUST 解析为 http(s)。
fn validate_base_url(base_url: &str) -> Result<(), common::AliothError> {
    let t = base_url.trim();
    if t.is_empty() {
        return Ok(());
    }
    let parsed = url::Url::parse(t)
        .map_err(|_| common::AliothError::BadRequest(format!("base_url 无法解析为 URL: {t}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(common::AliothError::BadRequest(format!(
            "base_url 仅支持 http(s) scheme: {t}"
        )));
    }
    Ok(())
}
/// 载荷校验（provider 中立面唯一判据；fail-closed）
fn validate_payload(
    code: &str,
    provider: &str,
    auth_type: &str,
    config: &Value,
) -> Result<(), common::AliothError> {
    if !is_slug(code.trim()) {
        return Err(common::AliothError::BadRequest(
            "code 必须为非空 slug（[a-z0-9-]，≤64）".into(),
        ));
    }
    if !is_slug(provider.trim()) {
        return Err(common::AliothError::BadRequest(
            "provider 必须为非空 slug（[a-z0-9-]，≤64；注册表派发键）".into(),
        ));
    }
    if !is_slug(auth_type.trim()) {
        return Err(common::AliothError::BadRequest(
            "auth_type 必须为非空 slug（[a-z0-9-]；provider 自定义认证方案标识，无认证填 'none'）"
                .into(),
        ));
    }
    if !config.is_object() && !config.is_null() {
        return Err(common::AliothError::BadRequest(
            "config 必须为 JSON 对象（provider 私有配置）".into(),
        ));
    }
    Ok(())
}

/// 载荷校验（含 base_url 形状；create/update 共用入口）。
fn validate_payload_with_url(
    code: &str,
    provider: &str,
    auth_type: &str,
    base_url: &str,
    config: &Value,
) -> Result<(), common::AliothError> {
    validate_payload(code, provider, auth_type, config)?;
    validate_base_url(base_url)
}

/// 凭据要求：非 `none` 认证方案 MUST 提供明文（`none` 方案 MUST NOT 携带凭据）。
fn credential_for(
    auth_type: &str,
    app_secret: Option<&str>,
) -> Result<Option<String>, common::AliothError> {
    let secret = app_secret.map(str::trim).filter(|s| !s.is_empty());
    match (auth_type == AUTH_TYPE_NONE, secret) {
        (false, None) => Err(common::AliothError::BadRequest(format!(
            "auth_type='{auth_type}' 需要提供 app_secret"
        ))),
        (true, Some(_)) => Err(common::AliothError::BadRequest(
            "auth_type='none' 不应携带 app_secret".into(),
        )),
        (_, s) => Ok(s.map(str::to_string)),
    }
}

// 保存期成员资格判据单源 = `outbound_client::provider::{membership_verdict, validate_registered}`
// （与 `resolve()`/`auth_headers()` 同面；规约 `openapi-external-access ::
// outbound-provider-catalog-and-membership-check`）。本面只调用，MUST NOT 复制分支语义。

pub async fn create_outbound_client(
    pool: web::Data<PgPool>,
    body: web::Json<CreateOutboundClientRequest>,
) -> Result<HttpResponse, common::AliothError> {
    let b = body.into_inner();
    let config = if b.config.is_null() {
        Value::Object(Default::default())
    } else {
        b.config.clone()
    };
    validate_payload_with_url(&b.code, &b.provider, &b.auth_type, &b.base_url, &config)?;
    // 形状通过后判成员资格（两层判据各自独立：形状 → 成员；失败信息不混）
    outbound_client::provider::validate_registered(&b.provider, &b.auth_type)?;
    let plaintext = credential_for(&b.auth_type, b.app_secret.as_deref())?;
    let secret_enc = match &plaintext {
        Some(p) => Some(crypto::encrypt(p)?),
        None => None,
    };
    let repo = OutboundRepository::new(pool.get_ref().clone());
    let id = repo
        .create_client(
            b.code.trim(),
            b.provider.trim(),
            b.auth_type.trim(),
            b.base_url.trim(),
            &config,
            secret_enc.as_deref(),
        )
        .await?;
    Ok(HttpResponse::Created().json(serde_json::json!({
        "id": id.to_string(),
        "code": b.code,
        "provider": b.provider,
        "authType": b.auth_type,
        "hasSecret": secret_enc.is_some(),
        "secret": plaintext,
        "warn": "密钥明文仅本次返回，请妥善保存",
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateOutboundClientRequest {
    pub provider: Option<String>,
    pub auth_type: Option<String>,
    pub base_url: Option<String>,
    /// provider 私有配置（提供时**整体替换**）
    pub config: Option<Value>,
    /// 提供时触发凭据轮换（version+1）
    pub app_secret: Option<String>,
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
    if let Some(cfg) = &b.config {
        if !cfg.is_object() {
            return Err(common::AliothError::BadRequest(
                "config 必须为 JSON 对象（provider 私有配置）".into(),
            ));
        }
    }
    if let Some(p) = &b.provider {
        if !is_slug(p.trim()) {
            return Err(common::AliothError::BadRequest(
                "provider 必须为非空 slug（[a-z0-9-]，≤64）".into(),
            ));
        }
    }
    if let Some(a) = &b.auth_type {
        if !is_slug(a.trim()) {
            return Err(common::AliothError::BadRequest(
                "auth_type 必须为非空 slug（[a-z0-9-]；无认证填 'none'）".into(),
            ));
        }
    }
    if let Some(u) = &b.base_url {
        validate_base_url(u)?;
    }
    let repo = OutboundRepository::new(pool.get_ref().clone());
    // 成员资格：生效值 = 载荷值 ?? 本行值（部分更新）——行不存在/已软删 ⇒ 交 update_client 报 404
    if let Some((cur_provider, cur_auth_type)) = repo.provider_auth_for_id(id).await? {
        let provider = b.provider.as_deref().unwrap_or(&cur_provider);
        let auth_type = b.auth_type.as_deref().unwrap_or(&cur_auth_type);
        outbound_client::provider::validate_registered(provider, auth_type)?;
    }
    let secret_enc = match b.app_secret.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => Some(crypto::encrypt(s)?),
        Some(_) => {
            return Err(common::AliothError::BadRequest(
                "app_secret 不得为空串（轮换请提供新明文）".into(),
            ))
        }
        None => None,
    };
    repo.update_client(
        id,
        b.provider.as_deref().map(str::trim),
        b.auth_type.as_deref().map(str::trim),
        b.base_url.as_deref().map(str::trim),
        b.config.as_ref(),
        secret_enc.as_deref(),
        b.enabled,
    )
    .await?;
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "id": id.to_string(),
        "updated": true,
        "rotated": secret_enc.is_some(),
    })))
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

#[cfg(test)]
mod validation_tests {
    use super::*;

    #[test]
    fn slug_shapes() {
        for s in ["fssc", "wechat-pay", "ierp-v2", "none", "0x-9"] {
            assert!(is_slug(s), "应合法: {s}");
        }
        for s in [
            "",
            "Magic Auth",
            "Basic",
            "a b",
            "scheme://x",
            &"x".repeat(65),
        ] {
            assert!(!is_slug(s), "应非法: {s:?}");
        }
    }

    #[test]
    fn base_url_shapes() {
        assert!(validate_base_url("").is_ok(), "空 = 合法（可后补）");
        assert!(validate_base_url("https://api.example.com/v1").is_ok());
        assert!(validate_base_url("http://10.0.0.1:8080").is_ok());
        assert!(validate_base_url("not a url").is_err());
        assert!(
            validate_base_url("ftp://example.com").is_err(),
            "仅 http(s)"
        );
        assert!(validate_base_url("example.com").is_err(), "缺 scheme");
    }

    #[test]
    fn payload_rejects_non_slug_auth_type() {
        let cfg = serde_json::json!({});
        let err = validate_payload_with_url("c1", "fssc", "Magic Auth", "https://x.y", &cfg);
        assert!(err.is_err(), "auth_type 形状外值必须 400");
    }

    #[test]
    fn payload_rejects_bad_base_url() {
        let cfg = serde_json::json!({});
        let err = validate_payload_with_url("c1", "fssc", "none", "not a url", &cfg);
        assert!(err.is_err(), "base_url 非法必须 400");
    }

    #[test]
    fn payload_accepts_well_formed() {
        let cfg = serde_json::json!({ "timeout_ms": 3000 });
        assert!(
            validate_payload_with_url("c1", "fssc", "basic", "https://api.example.com", &cfg)
                .is_ok()
        );
    }

    #[test]
    fn credential_coupling() {
        assert!(credential_for("none", None).is_ok());
        assert!(
            credential_for("none", Some("s")).is_err(),
            "none 不得带凭据"
        );
        assert!(credential_for("basic", None).is_err(), "非 none 必须带凭据");
        assert!(credential_for("basic", Some("s")).is_ok());
    }

    // （保存期成员资格单测已随判据单源迁至 `outbound-client/src/provider.rs` tests：
    //   change fix-openapi-review-findings §5——管理面与运行期同源消费）
}
