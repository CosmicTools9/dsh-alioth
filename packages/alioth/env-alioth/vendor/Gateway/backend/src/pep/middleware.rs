//! PEP (Policy Enforcement Point) Middleware for NGAC authorization.
//!
//! This middleware is responsible for:
//! 1. Extracting user identity from JWT tokens
//! 2. Calling SSO PDP (Policy Decision Point) for permission checks
//! 3. Enforcing access control decisions (Permit/Deny)
//! 4. Recording audit events for compliance
//! 5. Inserting RequestContext into request extensions for downstream handlers
//!
//! # Architecture
//!
//! ```text
//! Request → JWT Extraction → PDP Check → Decision → Audit Log → Insert Context → Response
//!                              ↓
//!                        SSO Service
//! ```
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use crate::pep::NgacEnforcer;
//! use actix_web::App;
//!
//! #[actix_web::main]
//! async fn main() -> std::io::Result<()> {
//!     let enforcer = NgacEnforcer::new(pool, jwt_public_key, jwt_public_key_prev, "http://localhost:9002".to_string());
//!     
//!     App::new()
//!         .wrap(enforcer)
//!         .service(my_protected_route)
//! }
//! ```

use super::cache::{ProbeOutcome, VersionProbe};
use crate::epp::{record_audit_event, Decision as AuditDecision};
use crate::pep::jwks::{JwksError, SsoJwksClient};
use actix_web::{
    body::EitherBody,
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage, HttpResponse,
};
use common::context::RequestContext;
use common::telemetry::{error, info, warn};
use futures::future::{ok, LocalBoxFuture, Ready};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use ngac_contract::{HttpNgacClient, ResourceRegistry};

/// 按 namespace 构建资源注册表（与 main.rs 受保护路由一致）：WZ 的 service 路由为
/// 2 段风格（/service/invoice-sync/list）且实体直接挂 service 根，必须加载
/// with_wz_defaults（含 invoice-sync/receipt-sync 服务根别名键），否则 resolve
/// 返回 None → map_resource 回退把 /service/invoice-sync/list 解析成 list:0 → 403。
fn ngac_resource_registry() -> ResourceRegistry {
    let reg = ResourceRegistry::new().with_alioth_defaults();
    if std::env::var("NAMESPACE")
        .unwrap_or_default()
        .eq_ignore_ascii_case("WZ")
    {
        reg.with_wz_defaults()
    } else {
        reg
    }
}
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

/// 测试用 EC P-256 公钥（PKCS#8 PEM），供 `new_without_pool()` 与集成测试使用。
const TEST_SSO_JWT_PUBLIC_KEY: &[u8] = b"-----BEGIN PUBLIC KEY-----
\
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAExHBYWD4VZBXSBjQIgcMUKbtHGEV3
\
NK6CQd0RxdS3yLGgsXJ1XqdLJwuPvErVIsSI3ywGfDHPPrqmuN53XjRBZg==
\
-----END PUBLIC KEY-----";

/// PDP (Policy Decision Point) error types.
///
/// Errors that can occur during PDP communication, including token validation,
/// network failures, and invalid responses.
#[derive(Debug)]
pub enum PdpError {
    /// JWT token decoding or validation failed
    TokenDecodeError(String),
    /// Network error during PDP communication
    NetworkError(String),
    /// Invalid response format from PDP
    InvalidResponse(String),
    /// PDP service unavailable
    PdpUnavailable(String),
}

impl std::fmt::Display for PdpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PdpError::TokenDecodeError(reason) => write!(f, "Token decode failed: {}", reason),
            PdpError::NetworkError(reason) => write!(f, "Network error: {}", reason),
            PdpError::InvalidResponse(reason) => write!(f, "Invalid PDP response: {}", reason),
            PdpError::PdpUnavailable(reason) => write!(f, "PDP unavailable: {}", reason),
        }
    }
}

impl std::error::Error for PdpError {}

/// JWT Claims structure for authentication.
///
/// Extracted from the Authorization header's Bearer token.
/// Contains user identity and token metadata.
///
/// # Fields
///
/// * `sub` - Subject (user ID) from JWT
/// * `exp` - Expiration timestamp (Unix seconds)
/// * `iat` - Issued-at timestamp (Unix seconds)
/// * `email` - User email address (optional, for audit logging)
///
/// # Example
///
/// ```json
/// {
///   "sub": "550e8400-e29b-41d4-a716-446655440000",
///   "exp": 1234567890,
///   "iat": 1234567800,
///   "email": "user@example.com"
/// }
/// ```
#[derive(Debug, Deserialize, Clone)]
pub struct Claims {
    /// Subject (user ID) from JWT
    pub sub: String,
    /// Expiration time (Unix timestamp in seconds)
    pub exp: usize,
    /// Issued at (Unix timestamp in seconds)
    pub iat: usize,
    /// User email address for audit logging (defaults to empty string)
    #[serde(default)]
    pub email: String,
    /// Username for display
    #[serde(default)]
    pub username: String,
    /// Session token linking this token to an SSO session.
    /// When present, the PEP validates session revocation status.
    #[serde(default)]
    pub sid: String,
    /// Issuer（必须与 SSO `oidc_issuer` 一致），用于防令牌错投。
    #[serde(default)]
    pub iss: String,
    /// Audience（第一方场景 == issuer），验证方必须校验命中自身预期值。
    #[serde(default)]
    pub aud: String,
    /// OAuth scope（空格分隔）；client_credentials 服务令牌携带其子集。
    #[serde(default)]
    pub scope: String,
    /// 服务令牌主体：api_clients.fk_service_user（auth_users 服务用户 id）。
    /// 自然人令牌为 0。服务令牌以此作为 NGAC PDP 决策主体（openapi-external-access）。
    #[serde(default)]
    #[serde(with = "common::serde_zuid")]
    pub svc_user_id: i64,
}

/// NGAC (Next Generation Access Control) Policy Enforcer.
///
/// Actix-web middleware that enforces access control by:
/// 1. Extracting JWT from Authorization header
/// 2. Validating token and extracting user claims
/// 3. Sending PDP check request to SSO service
/// 4. Enforcing PDP decision (permit/deny)
/// 5. Recording audit events
///
/// # Examples
///
/// ## Create with database pool (for audit logging)
///
/// ```rust,ignore
/// let enforcer = NgacEnforcer::new(
///     database_pool,
///     jwt_public_key,
///     jwt_public_key_prev,
///     "http://localhost:9002".to_string(),
/// );
/// ```
///
/// ## Create without database pool (no audit logging)
///
/// ```rust,ignore
/// let enforcer = NgacEnforcer::new_without_pool();
/// ```
pub struct NgacEnforcer {
    /// Database pool for audit logging
    pool: Option<PgPool>,
    /// SSO JWT ES256 public key (PEM) for token validation.
    /// 作为 JWKS 动态获取的回退；可留空（"去私钥"）完全依赖 JWKS。
    jwt_public_key: Vec<u8>,
    /// 轮换窗口内的历史 ES256 公钥（PEM，可选）——静态回退多 key（prev）。
    jwt_public_key_prev: Vec<u8>,
    /// SSO JWKS 客户端：动态获取验证公钥，替代静态公钥分发。
    jwks_client: Option<Arc<SsoJwksClient>>,
    /// 策略版本探针（remove-ngac-pep-decision-cache）：列缓存失效信号
    version_probe: Arc<VersionProbe>,
    /// 列级授权缓存（user+resource_type → columns，TTL 60s）
    column_cache: std::sync::Arc<super::cache::ColumnCache>,
    /// NGAC HTTP client for remote SSO calls
    ngac_client: Option<HttpNgacClient>,
    /// 预期的 SSO 令牌 issuer（用于校验 `iss`）。
    issuer: String,
    /// 预期的 SSO 令牌 audience（用于校验 `aud`）。
    audience: String,
    /// Resource registry mapping URL patterns to isahl tables.
    /// When None, falls back to the old map_resource behavior.
    resource_registry: Option<ngac_contract::ResourceRegistry>,
    /// Paths that completely bypass JWT authentication and NGAC PDP.
    /// Used for auth endpoints (login, register, etc.) that must be
    /// accessible without any credentials.
    public_noauth_paths: HashSet<String>,
}
impl NgacEnforcer {
    pub fn new(
        pool: PgPool,
        jwt_public_key: Vec<u8>,
        jwt_public_key_prev: Vec<u8>,
        sso_service_url: String,
    ) -> Self {
        // NGAC_SPEC §4.3：sso_service_url 为空（standalone 交付形态）→ 无 PDP/JWKS
        // 客户端（ngac_client=None），持有有效 JWT 即放行；此前构造空 URL 客户端使
        // decide/list 必网络失败 → fail-close 全量 403，standalone fail-open 失效。
        let has_pdp_endpoint = !sso_service_url.trim().is_empty();
        Self {
            pool: Some(pool),
            jwt_public_key,
            jwt_public_key_prev,
            jwks_client: if has_pdp_endpoint {
                Some(Arc::new(SsoJwksClient::new(sso_service_url.clone())))
            } else {
                None
            },

            version_probe: Arc::new(VersionProbe::new()),
            column_cache: std::sync::Arc::new(super::cache::ColumnCache::with_defaults()),
            ngac_client: if has_pdp_endpoint {
                Some(HttpNgacClient::new(sso_service_url))
            } else {
                None
            },
            issuer: "http://localhost:9002".to_string(),
            audience: "http://localhost:9002".to_string(),
            public_noauth_paths: HashSet::new(),
            resource_registry: Some(ngac_resource_registry()),
        }
    }

    /// Create NgacEnforcer without database pool and NGAC client.
    /// Useful for integration tests where only JWT validation is tested.
    pub fn new_without_pool() -> Self {
        Self {
            pool: None,
            jwt_public_key: TEST_SSO_JWT_PUBLIC_KEY.to_vec(),
            jwt_public_key_prev: Vec::new(),
            jwks_client: None,

            version_probe: Arc::new(VersionProbe::new()),
            column_cache: std::sync::Arc::new(super::cache::ColumnCache::with_defaults()),
            ngac_client: None,
            issuer: "http://localhost:9002".to_string(),
            audience: "http://localhost:9002".to_string(),
            public_noauth_paths: HashSet::new(),
            resource_registry: Some(ngac_resource_registry()),
        }
    }

    /// Configure a set of paths that bypass the NGAC PDP check while still
    /// requiring a valid JWT. Paths must match `req.path()` exactly
    /// (e.g. "/api/global/overview").
    /// 配置 JWT `iss`/`aud` 绑定（须与 SSO `oidc_issuer` 一致）。
    /// 缺省为 `http://localhost:9002`，生产应通过 Config::sso_jwt_issuer 注入。
    pub fn with_token_binding(mut self, issuer: String, audience: String) -> Self {
        self.issuer = issuer;
        self.audience = audience;
        self
    }
    /// Configure a custom resource registry.
    /// Overrides the default alioth resource mappings.
    pub fn with_resource_registry(mut self, registry: ngac_contract::ResourceRegistry) -> Self {
        self.resource_registry = Some(registry);
        self
    }
    /// Configure a set of paths that completely bypass JWT authentication
    /// and NGAC PDP. Use for auth endpoints like login, register, etc.
    /// Paths are matched by prefix (e.g. "/api/auth" matches "/api/auth/login").
    pub fn with_public_noauth_paths(mut self, paths: HashSet<String>) -> Self {
        self.public_noauth_paths = paths;
        self
    }
}

/// PDP 检查请求 / 响应类型已从 ngac-contract crate 导入。
/// 本地保留类型别名以便测试代码兼容。
pub type PdpCheckRequest = ngac_contract::PdpCheckRequest;
pub type PdpCheckResponse = ngac_contract::PdpCheckResponse;

impl<S, B> Transform<S, ServiceRequest> for NgacEnforcer
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = NgacEnforcerService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(NgacEnforcerService {
            service: Rc::new(service),
            pool: self.pool.clone(),
            jwt_public_key: self.jwt_public_key.clone(),
            jwt_public_key_prev: self.jwt_public_key_prev.clone(),
            jwks_client: self.jwks_client.clone(),

            version_probe: self.version_probe.clone(),
            column_cache: self.column_cache.clone(),
            ngac_client: self.ngac_client.clone(),
            issuer: self.issuer.clone(),
            audience: self.audience.clone(),
            public_noauth_paths: self.public_noauth_paths.clone(),
            resource_registry: self.resource_registry.clone(),
        })
    }
}

/// Decode JWT token and extract claims.
///
/// 优先通过 SSO JWKS（按 token header `kid` 选密钥）验证；JWKS 不可用或缺失对应
/// 密钥时，回退到静态配置的公钥（若存在）。`jwks_client` 为 None 时直接使用静态公钥。
/// 除 `exp` 外，强制校验 `iss` 与 `aud`：jsonwebtoken 10 的 `set_issuer`/`set_audience`
/// 为条件校验（claim 缺失即跳过），故 decode 成功后显式比对——`iss`/`aud` 必须存在
/// 且等于预期绑定值，缺失或不匹配 → 401（防令牌跨部署/服务错投被接受）。
async fn decode_jwt_token(
    jwks_client: Option<&SsoJwksClient>,
    fallback_keys: &[&[u8]],
    token: &str,
    issuer: &str,
    audience: &str,
) -> Result<Claims, PdpError> {
    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = true;
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[audience]);

    let decoding_key = match jwks_client {
        Some(client) => {
            let kid = SsoJwksClient::token_kid(token);
            match client.decoding_key(&kid).await {
                Ok(key) => Some(key),
                // 仅 SSO 不可用（网络/HTTP/解析失败）回退静态公钥（可用性）；
                // kid 未命中 / 密钥不支持是吊销或配置信号，fail-closed 拒绝
                // （M69 审计 P3：回退会掩盖已吊销 key 的令牌）。
                Err(JwksError::FetchFailed(e)) => {
                    log::debug!("JWKS 不可用，回退静态公钥: {e}");
                    None
                }
                Err(e) => {
                    return Err(PdpError::TokenDecodeError(format!(
                        "JWKS 验证失败（fail-closed）: {e}"
                    )))
                }
            }
        }
        None => None,
    };

    let claims = match decoding_key {
        // JWKS 命中 → 单钥验证（kid 已选钥）
        Some(key) => {
            decode::<Claims>(token, &key, &validation)
                .map_err(|e| PdpError::TokenDecodeError(e.to_string()))?
                .claims
        }
        // 无 JWKS / JWKS 不可用 → 静态回退（active + prev 依次尝试，轮换窗口旧钥可验）
        None => verify_token_with_keys(fallback_keys, token, &validation)?,
    };

    // 显式强制 iss/aud（jsonwebtoken 10 的 set_* 仅条件校验）：声明缺失或不匹配
    // → 视为无效令牌（401），杜绝「缺 aud/iss 声明跳过绑定校验」的错投路径。
    if claims.iss.is_empty() || claims.iss != issuer {
        return Err(PdpError::TokenDecodeError(format!(
            "iss claim missing or mismatch: expected '{}'",
            issuer
        )));
    }
    if claims.aud.is_empty() || claims.aud != audience {
        return Err(PdpError::TokenDecodeError(format!(
            "aud claim missing or mismatch: expected '{}'",
            audience
        )));
    }

    Ok(claims)
}

/// 静态回退多 key 验签：active + prev 依次尝试，任一把成功即接受；
/// 全部失败返回第一个错误（保持单钥错误语义），空列表返回明确错误。
fn verify_token_with_keys(
    keys: &[&[u8]],
    token: &str,
    validation: &Validation,
) -> Result<Claims, PdpError> {
    let mut first_err: Option<PdpError> = None;
    for key in keys {
        let decoding_key = match DecodingKey::from_ec_pem(key) {
            Ok(dk) => dk,
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(PdpError::TokenDecodeError(e.to_string()));
                }
                continue;
            }
        };
        match decode::<Claims>(token, &decoding_key, validation) {
            Ok(td) => return Ok(td.claims),
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(PdpError::TokenDecodeError(e.to_string()));
                }
            }
        }
    }
    Err(first_err.unwrap_or_else(|| {
        PdpError::TokenDecodeError("no static fallback public key configured".to_string())
    }))
}

/// 校验 SSO 会话是否仍处于活跃状态。
///
/// 仅当 JWT 携带 `sid`（SSO 会话令牌）时执行；旧版未绑定会话的 token
/// 跳过检查（向后兼容）。会话状态从 `isahl_auth.sso_sessions` 读取，
/// 已吊销/过期/不存在的会话视为失效，返回 false。
/// `expires_at` 为 not null 列：本地直接校验过期，不依赖 SSO 定时任务
/// 翻转 status（M69 审计 P3：过期窗口内旧 token 仍被接受）。
async fn is_session_active(pool: &PgPool, sid: &str) -> bool {
    let result = sqlx::query_as::<_, (String,)>(
        "SELECT status FROM isahl_auth.sso_sessions \
         WHERE session_token = $1 AND expires_at > NOW() LIMIT 1",
    )
    .bind(sid)
    .fetch_optional(pool)
    .await;

    match result {
        Ok(Some((status,))) => status == "active",
        // 会话不存在或查询失败：保守地视为失效，拒绝请求。
        Ok(None) | Err(_) => false,
    }
}

/// 远程调用 SSO NGAC PDP 决策端点
/// 通过 HTTP 调用 SSO 的 /api/ngac/decide 接口，消除编译时耦合
async fn call_pdp_remote(
    ngac_client: &HttpNgacClient,
    auth_token: &str,
    user_id: i64,
    resource: &str,
    action: &str,
) -> Result<bool, PdpError> {
    let request = ngac_contract::PdpCheckRequest {
        user_id,
        resource: resource.to_string(),
        action: action.to_string(),
    };

    let response = ngac_client
        .decide(&request, auth_token)
        .await
        .map_err(|e| {
            common::telemetry::error!("NGAC remote decision failed: {}", e);
            PdpError::NetworkError(e.to_string())
        })?;

    Ok(response.permitted)
}

/// Map HTTP method to action string
fn method_to_action(method: &str) -> &'static str {
    match method {
        "GET" => "read",
        "POST" => "create",
        "PUT" | "PATCH" => "update",
        "DELETE" => "delete",
        _ => "read",
    }
}

/// Map HTTP request path to NGAC resource string in `type:id` format.
///
/// All resources are normalized to two segments: `{type}:{id}`.
/// Collection-level operations (list/create) use `id = 0`.
///
/// Examples:
/// - /api/schemas -> schema:0
/// - /api/schemas/123 -> schema:123
/// - /api/schemas/123/export -> schema:123
/// - /api/collections -> collection:0
/// - /api/collections/abc -> collection:abc
/// - /api/product -> product:0
/// - /api/product/42 -> product:42
fn map_resource(path: &str) -> String {
    let stripped = path.trim_start_matches("/api/").trim_start_matches('/');
    let parts: Vec<&str> = stripped.split('/').collect();
    if parts.is_empty() || parts[0].is_empty() {
        return "api:0".to_string();
    }

    // /api/service/{svc}/{entity}[/{id}]：entity 段为资源类型（registry 未注册的
    // service 实体也按实体名推导，而非整路径 fallback 出 service_xxx 伪类型）。
    // 与 scope 推导（PEP scope 校验）一致，保证 PDP 决策与 scope 校验同实体。
    if parts.len() >= 3 && parts[0] == "service" {
        let entity = parts[2].replace('-', "_");
        let id = parts
            .get(3)
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        return format!("{}:{}", entity, id);
    }

    // OpenAPI 管理/统计面恒定资源（refactor-openapi-admin-ngac-pdp，迁移 029）：
    // - usage 系列（统计聚合，全 GET）→ openapi_analytics:0
    // - scope-catalog / outbound（出向管理 CRUD）→ openapi_admin:0
    // 文档路径（openapi.json/openapi/openapi/）在 is_openapi_doc_path 提前豁免，
    // 不到这里。
    if parts[0] == "openapi" {
        match parts.get(1).copied() {
            Some("usage") => return "openapi_analytics:0".to_string(),
            Some("scope-catalog") | Some("outbound") => {
                return "openapi_admin:0".to_string();
            }
            _ => {}
        }
    }

    let resource_type = parts[0].replace("-", "_");
    if parts.len() == 1 || parts[1].is_empty() {
        return format!("{}:0", resource_type);
    }

    // SSO PDP requires the resource ID to be an integer. If the second path
    // segment is numeric, use it as the resource ID (sub-paths like /export
    // are ignored because NGAC policies are object-centric). If it is not
    // numeric, treat it as a sub-resource qualifier so that
    // /api/schedule/overview becomes schedule_overview:0 instead of the
    // invalid schedule:overview.
    match parts[1].parse::<i64>() {
        Ok(id) => format!("{}:{}", resource_type, id),
        Err(_) => format!("{}_{}:0", resource_type, parts[1].replace("-", "_")),
    }
}
pub struct NgacEnforcerService<S> {
    service: Rc<S>,
    pool: Option<PgPool>,
    jwt_public_key: Vec<u8>,
    /// 轮换窗口内的历史 ES256 公钥（PEM，可选）——静态回退多 key（prev）
    jwt_public_key_prev: Vec<u8>,
    jwks_client: Option<Arc<SsoJwksClient>>,

    /// 策略版本探针（per-worker 缓存失效信号，fix-ngac-decision-consistency D4）
    version_probe: Arc<VersionProbe>,
    column_cache: std::sync::Arc<super::cache::ColumnCache>,
    ngac_client: Option<HttpNgacClient>,
    issuer: String,
    audience: String,
    /// Resource registry for URL → isahl table resolution.
    /// When None, falls back to the old map_resource logic.
    resource_registry: Option<ngac_contract::ResourceRegistry>,
    /// Paths that completely bypass JWT authentication and NGAC PDP.
    public_noauth_paths: HashSet<String>,
}

impl<S, B> Service<ServiceRequest> for NgacEnforcerService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_web::dev::forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let svc = self.service.clone();
        let pool = self.pool.clone();
        let jwt_public_key = self.jwt_public_key.clone();
        let jwt_public_key_prev = self.jwt_public_key_prev.clone();
        let jwks_client = self.jwks_client.clone();

        let version_probe = self.version_probe.clone();
        let ngac_client = self.ngac_client.clone();
        let issuer = self.issuer.clone();
        let audience = self.audience.clone();
        let public_noauth_paths = self.public_noauth_paths.clone();
        let resource_registry = self.resource_registry.clone();
        let column_cache = self.column_cache.clone();
        Box::pin(async move {
            let mut req = req;
            // 静态回退 key 列表（active + prev，过滤空）——闭包内构造，借用不逃逸
            let fallback_keys: Vec<&[u8]> = {
                let mut keys = Vec::with_capacity(2);
                if !jwt_public_key.is_empty() {
                    keys.push(jwt_public_key.as_slice());
                }
                if !jwt_public_key_prev.is_empty() {
                    keys.push(jwt_public_key_prev.as_slice());
                }
                keys
            };
            // 入站伪造防护：无条件剥离客户端自声明的安全 header，
            // 权威值由本中间件在授权判定后以 insert 重建。
            req.headers_mut().remove("x-visible-ids");
            req.headers_mut().remove("x-authorized-columns");
            // E2E test mode: bypass auth for automated API testing
            // 编译期门控：仅启用 `e2e-test-mode` feature 时该分支才存在。
            // 生产/release 常规构建不启用 → 认证绕过代码不被编译（SECURITY_SPEC §1）。
            #[cfg(feature = "e2e-test-mode")]
            if std::env::var("E2E_TEST_MODE").unwrap_or_default() == "true" {
                let request_context =
                    RequestContext::with_username(1, "e2e-admin@alioth.test", "e2e-admin");
                req.extensions_mut().insert(request_context);
                req.extensions_mut().insert(1i64);
                // 无 PDP 形态：注入显式全量列授权（与 standalone/FAIL_OPEN 交付语义一致），
                // 使「header 缺失」恒为异常信号（crud 对缺失 fail-closed）
                req.headers_mut().insert(
                    actix_web::http::header::HeaderName::from_static("x-authorized-columns"),
                    actix_web::http::header::HeaderValue::from_static("*"),
                );
                let response = svc.call(req).await?;
                return Ok(response.map_into_left_body());
            }

            // 完全公开路径：不需要 JWT token，跳过所有认证
            // 用于 auth/login、auth/register 等需要在登录前访问的端点
            // 段边界匹配（SECURITY_SPEC §3.4 豁免最小化）：`/api/auth` 只豁免
            // 自身与 `/api/auth/...`，不得匹配 `/api/authx` 等非豁免前缀。
            let is_noauth_path = public_noauth_paths.iter().any(|p| {
                let path = req.path();
                path == p || path.starts_with(&format!("{}/", p))
            });
            if is_noauth_path {
                // 服务令牌不得经 noauth 豁免（access-control public-whitelist-removed：
                // 「服务令牌（sub=client:*）MUST NOT 存在任何路径豁免特判」）。
                // 仅当请求携带已认证服务令牌时落入正常 JWT/PDP 流程（无 PDP → 500）；
                // 无凭证、无效凭证、自然人令牌直通——登录/回调端点必须无凭证可达。
                let bearer = req
                    .headers()
                    .get("Authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.strip_prefix("Bearer "))
                    .map(|t| t.to_string());
                let cookie_token = req
                    .cookie("access_token")
                    .map(|c| c.value().trim().to_string());
                let is_service_token = match bearer.or(cookie_token) {
                    Some(token) if !token.is_empty() => matches!(
                        decode_jwt_token(
                            jwks_client.as_deref(),
                            &fallback_keys,
                            &token,
                            &issuer,
                            &audience,
                        )
                        .await,
                        Ok(c) if c.sub.starts_with("client:")
                    ),
                    _ => false,
                };
                if !is_service_token {
                    info!("No-auth path access permitted: {}", req.path());
                    let response = svc.call(req).await?;
                    return Ok(response.map_into_left_body());
                }
                // 服务令牌命中 noauth 路径 → 继续下方 JWT 校验 + PDP 流程
            }

            // 从 Authorization header 或 httpOnly Cookie 获取 token
            // SSO 登录通过 Cookie 下发 access_token，前端跨域/跨端口请求时
            // 也依赖 Cookie 自动携带，因此 PEP 需要同时支持两种来源。
            let token = match req.headers().get("Authorization") {
                Some(value) => {
                    if let Ok(value_str) = value.to_str() {
                        if let Some(token) = value_str.strip_prefix("Bearer ") {
                            token.to_string()
                        } else {
                            warn!("Invalid authorization header format");
                            return Ok(req.into_response(
                                HttpResponse::Unauthorized()
                                    .json(serde_json::json!({
                                        "error": "Invalid authorization header format"
                                    }))
                                    .map_into_right_body(),
                            ));
                        }
                    } else {
                        return Ok(req.into_response(
                            HttpResponse::Unauthorized()
                                .json(serde_json::json!({
                                    "error": "Invalid authorization header"
                                }))
                                .map_into_right_body(),
                        ));
                    }
                }
                None => {
                    // 没有 Authorization header，尝试从 Cookie 获取
                    match req.cookie("access_token") {
                        Some(cookie) => {
                            let value = cookie.value().trim();
                            if value.is_empty() {
                                return Ok(req.into_response(
                                    HttpResponse::Unauthorized()
                                        .json(serde_json::json!({
                                            "error": "Missing authorization header"
                                        }))
                                        .map_into_right_body(),
                                ));
                            }
                            value.to_string()
                        }
                        None => {
                            return Ok(req.into_response(
                                HttpResponse::Unauthorized()
                                    .json(serde_json::json!({
                                        "error": "Missing authorization header"
                                    }))
                                    .map_into_right_body(),
                            ));
                        }
                    }
                }
            };

            // Decode JWT to get user_id and email
            // 优先 JWKS 动态公钥，回退静态公钥（见 decode_jwt_token）
            let claims = match decode_jwt_token(
                jwks_client.as_deref(),
                &fallback_keys,
                &token,
                &issuer,
                &audience,
            )
            .await
            {
                Ok(c) => c,
                Err(e) => {
                    error!("Token decode failed: {}", e);
                    return Ok(req.into_response(
                        HttpResponse::Unauthorized()
                            .json(serde_json::json!({
                                "error": "Invalid token"
                            }))
                            .map_into_right_body(),
                    ));
                }
            };

            // 会话吊销检查：若 token 绑定 SSO 会话且会话已失效（注销/过期），
            // 即使 JWT 签名有效也拒绝访问，使 logout 即时生效。
            if !claims.sid.is_empty() {
                let session_ok = match &pool {
                    Some(p) => is_session_active(p, &claims.sid).await,
                    None => true, // 无 DB 池（如测试）跳过检查
                };
                if !session_ok {
                    warn!(
                        "Access denied: SSO session '{}' is revoked/expired",
                        claims.sid
                    );
                    return Ok(req.into_response(
                        HttpResponse::Unauthorized()
                            .json(serde_json::json!({
                                "error": "Session revoked"
                            }))
                            .map_into_right_body(),
                    ));
                }
            }

            let user_id_str = claims.sub.clone();
            let user_email = claims.email.clone();

            // 服务令牌识别（openapi-external-access）：`sub=client:*` + svc_user_id
            // → 以服务用户（api_clients.fk_service_user）作为 NGAC PDP 主体。
            // 自然人令牌 sub 为纯数字，svc_user_id=0，走原路径。
            let is_service_token = user_id_str.starts_with("client:") && claims.svc_user_id > 0;
            let user_id_i64 = if is_service_token {
                // Gap C（fail-closed）：服务令牌必须对应 enabled 且未过期的 client。
                // 吊销/禁用/过期 → 401（旧 JWT 在 TTL 内也不可用）。
                if let Some(ref db_pool) = pool {
                    let client_id = user_id_str.strip_prefix("client:").unwrap_or(&user_id_str);
                    let client_ok: Option<(bool,)> = sqlx::query_as(
                        "SELECT enabled FROM isahl_auth.api_clients \
                         WHERE client_id = $1 AND deleted_at IS NULL \
                           AND (expires_at IS NULL OR expires_at > NOW())",
                    )
                    .bind(client_id)
                    .fetch_optional(db_pool)
                    .await
                    .unwrap_or(None);
                    match client_ok {
                        Some((true,)) => {}
                        _ => {
                            warn!(
                                "Access denied: service client '{}' disabled/expired/missing",
                                client_id
                            );
                            return Ok(req.into_response(
                                HttpResponse::Unauthorized()
                                    .json(serde_json::json!({
                                        "error": "Invalid client",
                                        "reason": "Service client disabled or expired"
                                    }))
                                    .map_into_right_body(),
                            ));
                        }
                    }
                }
                claims.svc_user_id
            } else {
                user_id_str.parse::<i64>().unwrap_or(0)
            };
            // Create RequestContext (kept in local var; inserted after visible_ids lookup)
            let mut request_context = RequestContext::with_username(
                user_id_i64,
                user_email.clone(),
                if is_service_token {
                    user_id_str.clone()
                } else {
                    claims.username.clone()
                },
            );
            req.extensions_mut().insert(user_id_i64);
            // Insert RequestContext（non-public 路径稍后设置 visible_ids）
            let ctx = request_context.clone();
            req.extensions_mut().insert(ctx);
            // require a valid JWT (enforced above). This lets authenticated
            // users access global UI data without defining per-user NGAC
            // policies for aggregate endpoints（remove-public-whitelist：全部走 PDP）。

            // OpenAPI 文档端点（/api/openapi.json、/api/openapi/ 根 = Swagger UI）：
            // 元数据，持有有效 JWT（服务令牌或自然人）即放行，不查 PDP。
            // 仅限文档本身——管理子路径（/api/openapi/outbound-clients 等）MUST
            // 走 NGAC 授权（gateway-openapi-outbound-unify），不得命中本豁免。
            let is_openapi_doc_path = req.path() == "/api/openapi.json"
                || req.path() == "/api/openapi"
                || req.path() == "/api/openapi/";
            if is_openapi_doc_path {
                info!(
                    "OpenAPI doc access permitted for user={} path={}",
                    user_id_str,
                    req.path()
                );
                let response = svc.call(req).await?;
                return Ok(response.map_into_left_body());
            }

            // Resolve resource using registry (falls back to map_resource for unknown routes)
            let resolved = resource_registry
                .as_ref()
                .and_then(|r| r.resolve(req.path()));
            let resource = resolved
                .as_ref()
                .map(|r| r.resource.clone())
                .unwrap_or_else(|| map_resource(req.path()));
            let action = method_to_action(req.method().as_str());

            // scope 强制校验（openapi-external-access，remove-public-whitelist 收紧）：
            // 服务令牌的 scope MUST 非空且覆盖端点所需 scope，否则 403 SCOPE_INSUFFICIENT。
            // 端点所需 scope 由资源类型推导：`read:{type}` / `create:{type}` /
            // `update:{type}` / `delete:{type}`。scope 空 = fail-closed（不再「未受限」）。
            if is_service_token {
                // fail-closed：scope 空 → 直接 403（remove-public-whitelist）
                if claims.scope.trim().is_empty() {
                    warn!("Scope insufficient: user={} scope=EMPTY", user_id_str);
                    return Ok(req.into_response(
                        HttpResponse::Forbidden()
                            .json(serde_json::json!({
                                "error": "SCOPE_INSUFFICIENT",
                                "reason": "Service token has empty scope; explicit scope required"
                            }))
                            .map_into_right_body(),
                    ));
                }
                // 非空已满足第一步；覆盖校验在下方
                let resolved_for_scope =
                    resolved.as_ref().map(|r| r.type_name.clone()).or_else(|| {
                        // registry 未注册的 service 实体路径（如 /api/service/measurement/unit）：
                        // 从 entity 段（/service/{svc}/{entity}[/{id}]）推导 scope 实体名，
                        // 而非整路径 fallback（避免 service_measurement 伪类型）。
                        let stripped = req.path().trim_start_matches("/api/");
                        let parts: Vec<&str> = stripped.split('/').collect();
                        if parts.len() >= 3 && parts[0] == "service" {
                            Some(parts[2].replace('-', "_"))
                        } else {
                            map_resource(req.path())
                                .split(':')
                                .next()
                                .map(|s| s.to_string())
                        }
                    });
                if let Some(rt) = resolved_for_scope {
                    let required = format!("{}:{}", action, rt);
                    let has_scope = claims
                        .scope
                        .split_whitespace()
                        .any(|s| s == required || s == format!("*:{}", rt) || s == "*");
                    if !has_scope {
                        warn!(
                            "Scope insufficient: user={} required={} has={}",
                            user_id_str, required, claims.scope
                        );
                        return Ok(req.into_response(
                            HttpResponse::Forbidden()
                                .json(serde_json::json!({
                                    "error": "SCOPE_INSUFFICIENT",
                                    "reason": format!(
                                        "Required scope '{}' not granted (has: '{}')",
                                        required, claims.scope
                                    )
                                }))
                                .map_into_right_body(),
                        ));
                    }
                }
            }

            info!(
                "PEP checking access for user={} resource={} action={}",
                user_id_str, resource, action
            );

            // NGAC_FAIL_OPEN 判定提前到 PDP list 之前：fail-open 下列表端点同样
            // 跳过 PDP（与 decide 路径一致，消除 standalone/fail-open 下列表恒 403），
            // 按 §2.7 降级为不注入 visible_ids（None → crud 无 RLS 约束）。
            let fail_open = std::env::var("NGAC_FAIL_OPEN")
                .unwrap_or_default()
                .eq_ignore_ascii_case("true");

            // For list endpoints (resource_id == 0), query PDP for visible_ids
            // and inject into RequestContext for RLS filtering downstream.
            // 仅 read 动作注入（NGAC_SPEC §5.5.2：visible_ids 为行级读过滤）；
            // create/update/delete 集合操作走 decide 判定，不被 PDP list 耦合。
            if !fail_open {
                if let Some(resolved) = resolved.as_ref() {
                    if resolved.resource_id == 0 && action == "read" {
                        if let Some(ngac_client_ref) = &ngac_client {
                            let list_req = ngac_contract::PdpListRequest {
                                user_id: user_id_i64,
                                resource_type: resolved.type_name.clone(),
                                action: action.to_string(),
                            };
                            match ngac_client_ref.list(&list_req, &token).await {
                                Ok(list_resp) if list_resp.permitted => {
                                    // visible_ids=None = admin 全量（NGAC_SPEC §6.2）：
                                    // 不注入 header / 不设置 RLS——crud 侧缺失 = 无约束（全表）。
                                    // Some([]) = 显式空授权 → 注入 `none`（恒假谓词零行）。
                                    // Some(ids) = 行级过滤集。
                                    if let Some(visible) = list_resp.visible_ids {
                                        request_context.set_visible_resource_ids(
                                            resolved.type_name.clone(),
                                            visible.clone(),
                                        );
                                        // 空集必须注入显式 `none` 标记——crud 解析为 Some([]) →
                                        // 恒假谓词零行；header 缺失在 crud 侧为无约束，不能作为空权限信号
                                        let header_val = if visible.is_empty() {
                                            "none".to_string()
                                        } else {
                                            visible
                                                .iter()
                                                .map(|i| i.to_string())
                                                .collect::<Vec<_>>()
                                                .join(",")
                                        };
                                        req.headers_mut().insert(
                                            actix_web::http::header::HeaderName::from_static(
                                                "x-visible-ids",
                                            ),
                                            actix_web::http::header::HeaderValue::from_str(
                                                &header_val,
                                            )
                                            .unwrap_or_else(|_| {
                                                actix_web::http::header::HeaderValue::from_static(
                                                    "none",
                                                )
                                            }),
                                        );
                                    }
                                }
                                _ => {
                                    // PDP list 调用失败或 permitted=false → fail-close：
                                    // 缺失 header 在 crud 侧 = 无约束（全表），不得静默放行
                                    warn!(
                                    "NGAC list check failed/denied for user={} type={} action={}",
                                    user_id_str, resolved.type_name, action
                                );
                                    return Ok(req.into_response(
                                        HttpResponse::Forbidden()
                                            .json(serde_json::json!({
                                                "error": "Access denied",
                                                "reason": "List permission check failed or denied"
                                            }))
                                            .map_into_right_body(),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            // Insert context AFTER visible_ids enrichment so downstream sees RLS
            req.extensions_mut().insert(request_context);

            // 版本探针（remove-ngac-pep-decision-cache）：决策不缓存——权限立即
            // 生效为硬性要求，每次请求现查 PDP；探针仅服务列级缓存（ColumnCache，
            // association 派生、版本化 ≤2s）失效。探针失败 → 本请求列授权绕过
            // 缓存直查（不服务陈旧条目）。无 PDP 客户端（standalone/FAIL_OPEN/e2e）不探针。
            let column_cache_bypass = match &ngac_client {
                Some(client) if !fail_open => matches!(
                    version_probe.ensure_fresh(client, &column_cache).await,
                    ProbeOutcome::Unavailable
                ),
                _ => false,
            };

            {
                // Cache miss — need PDP decision

                // 开发环境：NGAC_FAIL_OPEN=true 时完全跳过 PDP 调用，消除 SSO 依赖
                if fail_open {
                    info!(
                        "NGAC_FAIL_OPEN: skipping PDP for user={} resource={}",
                        user_id_str, resource
                    );
                    // 无 PDP 形态：注入显式全量列授权（与 standalone/e2e 交付语义一致），
                    // 使「header 缺失」恒为异常信号（crud 对缺失 fail-closed）
                    req.headers_mut().insert(
                        actix_web::http::header::HeaderName::from_static("x-authorized-columns"),
                        actix_web::http::header::HeaderValue::from_static("*"),
                    );
                    let response = svc.call(req).await?;
                    return Ok(response.map_into_left_body());
                }

                // 正常 PDP 路径
                let ngac_client = match &ngac_client {
                    Some(client) => client,
                    None => {
                        error!("No NGAC client configured for PDP decision");
                        return Ok(req.into_response(
                            HttpResponse::InternalServerError()
                                .json(serde_json::json!({
                                    "error": "Internal server error",
                                    "reason": "PDP decision service unavailable"
                                }))
                                .map_into_right_body(),
                        ));
                    }
                };

                match call_pdp_remote(ngac_client, &token, user_id_i64, &resource, action).await {
                    Ok(permitted) => {
                        if permitted {
                            info!("Access permitted for user={}", user_id_str);
                            if let Some(ref db_pool) = pool {
                                let _ = record_audit_event(
                                    db_pool,
                                    user_id_i64,
                                    &user_email,
                                    &resource,
                                    action,
                                    &AuditDecision::Permit,
                                )
                                .await;
                            }
                            // 列级授权注入（所有 read 动作：列表 resource_id==0 与详情
                            // resource_id!=0 统一注入；crud 引擎按 SENSITIVE_COLUMNS
                            // 与该 header 求交裁剪敏感列（fail-closed：none/缺失 = 空集）。
                            if let Some(resolved) = resolved.as_ref() {
                                if action == "read" {
                                    Self::inject_authorized_columns(
                                        &column_cache,
                                        Some(ngac_client),
                                        user_id_i64,
                                        &resolved.type_name,
                                        is_service_token,
                                        &token,
                                        &mut req,
                                        column_cache_bypass,
                                    )
                                    .await;
                                }
                            }
                            let response = svc.call(req).await?;
                            Ok(response.map_into_left_body())
                        } else {
                            warn!("Access denied for user={}", user_id_str);
                            if let Some(ref db_pool) = pool {
                                let _ = record_audit_event(
                                    db_pool,
                                    user_id_i64,
                                    &user_email,
                                    &resource,
                                    action,
                                    &AuditDecision::Deny,
                                )
                                .await;
                            }
                            Ok(req.into_response(
                                HttpResponse::Forbidden()
                                    .json(serde_json::json!({
                                        "error": "Access denied",
                                        "reason": "Permission denied by policies"
                                    }))
                                    .map_into_right_body(),
                            ))
                        }
                    }
                    Err(e) => {
                        error!("PDP check failed: {}", e);
                        // Already checked fail_open above; this is fail-close (production)
                        warn!("PDP unavailable, denying access (fail-close)");
                        Ok(req.into_response(
                            HttpResponse::Forbidden()
                                .json(serde_json::json!({
                                    "error": "Access denied",
                                    "reason": "Policy decision service unavailable"
                                }))
                                .map_into_right_body(),
                        ))
                    }
                }
            }
        })
    }
}

impl<S> NgacEnforcerService<S> {
    /// 列级授权注入：调 SSO /api/ngac/pdp/columns，将授权列集合写入 x-authorized-columns header。
    ///
    /// 缓存（ColumnCache TTL 60s）避免每请求调 PDP。`["*"]`（通配/无列级策略）→ 注入显式 `*`
    /// （crud 侧视为全量，防与 fail-closed 叠加回归）；空集合/columns 失败 → 注入 `none`
    /// （fail-closed，敏感列全裁）；无 PDP 客户端（standalone/FAIL_OPEN/e2e）→ 注入 `*`。
    /// `bypass_cache`：版本探针失败时跳过缓存读写（不服务陈旧条目、不把故障期
    /// 的 fail-closed 空授权写入缓存毒化后续请求，fix-ngac-decision-consistency D4）。
    async fn inject_authorized_columns(
        column_cache: &std::sync::Arc<super::cache::ColumnCache>,
        ngac_client: Option<&HttpNgacClient>,
        user_id: i64,
        resource_type: &str,
        is_service_token: bool,
        auth_token: &str,
        req: &mut ServiceRequest,
        bypass_cache: bool,
    ) {
        let cache_key =
            super::cache::ColumnCache::make_key(user_id, resource_type, is_service_token);
        let cached = if bypass_cache {
            None
        } else {
            column_cache.get(&cache_key)
        };
        let cols: Vec<String> = if let Some(cols) = cached {
            cols
        } else {
            let cols = match ngac_client {
                None => {
                    // 无 PDP 客户端（standalone / NGAC_FAIL_OPEN / e2e-test-mode）
                    // → 注入 `*`（PDP 未介入时维持全量，与 NGAC_SPEC §5.5.2 降级语义一致）
                    vec!["*".to_string()]
                }
                Some(client) => {
                    let request = ngac_contract::PdpColumnsRequest {
                        user_id,
                        resource_type: resource_type.to_string(),
                    };
                    match client.columns(&request, auth_token).await {
                        Ok(resp) if resp.permitted => resp.columns,
                        Ok(_) => {
                            // 不允许 → fail-closed：空授权（敏感列全裁）
                            vec![]
                        }
                        Err(e) => {
                            // PDP columns 调用失败 → fail-closed `none`（敏感列全裁）+ warn，
                            // 不阻断主请求（与 NGAC_SPEC §5.5.2：columns 失败注入 none）
                            common::telemetry::warn!(
                                "NGAC columns check failed for user={} type={}: {} — fail-closed none",
                                user_id,
                                resource_type,
                                e
                            );
                            vec![]
                        }
                    }
                }
            };
            if !bypass_cache {
                column_cache.set(cache_key, cols.clone());
            }
            cols
        };

        // `["*"]` 通配 → 注入显式 `*`（crud 侧视为全量——禁止「不注入」折叠，
        // 否则与列控 fail-closed 叠加会把 read:* 用户敏感列全裁）
        // 空授权 → 注入 "none" 标记（crud 识别为 fail-closed 全裁）；
        // 空 header 值非法（http crate panic），故用显式标记
        let header_val = if cols.is_empty() {
            "none".to_string()
        } else {
            cols.join(",")
        };
        req.headers_mut().insert(
            actix_web::http::header::HeaderName::from_static("x-authorized-columns"),
            actix_web::http::header::HeaderValue::from_str(&header_val)
                .unwrap_or_else(|_| actix_web::http::header::HeaderValue::from_static("none")),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ngac_enforcer_new() {
        let enforcer = NgacEnforcer::new_without_pool();
        assert!(enforcer.pool.is_none());
    }

    #[test]
    fn test_pdp_check_request_serialize() {
        let request = PdpCheckRequest {
            user_id: 42,
            resource: "document:123".to_string(),
            action: "read".to_string(),
        };

        let json = serde_json::to_string(&request).unwrap();
        // serde_zuid 迁移：user_id 序列化为字符串（双向兼容，SSO 端同构 deserialize）
        assert!(json.contains("\"user_id\":\"42\""));
        assert!(json.contains("\"resource\":\"document:123\""));
        assert!(json.contains("\"action\":\"read\""));
    }

    #[test]
    fn test_pdp_check_response_deserialize() {
        let json = r#"{
            "permitted": true,
            "reason": "Access granted"
        }"#;

        let response: PdpCheckResponse = serde_json::from_str(json).unwrap();
        assert!(response.permitted);
        assert_eq!(response.reason, "Access granted");
    }

    #[test]
    fn test_pdp_check_response_deserialize_deny() {
        let json = r#"{
            "permitted": false,
            "reason": "Access denied by policy"
        }"#;

        let response: PdpCheckResponse = serde_json::from_str(json).unwrap();
        assert!(!response.permitted);
        assert_eq!(response.reason, "Access denied by policy");
    }

    #[test]
    fn test_pdp_check_request_missing_fields() {
        let json = r#"{
            "user_id": 42
        }"#;

        let result: Result<PdpCheckRequest, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_method_to_action_mapping() {
        assert_eq!(method_to_action("GET"), "read");
        assert_eq!(method_to_action("POST"), "create");
        assert_eq!(method_to_action("PUT"), "update");
        assert_eq!(method_to_action("PATCH"), "update");
        assert_eq!(method_to_action("DELETE"), "delete");
        assert_eq!(method_to_action("OPTIONS"), "read"); // Default
    }

    #[test]
    fn test_claims_deserialize() {
        let json = r#"{
            "sub": "550e8400-e29b-41d4-a716-446655440000",
            "exp": 1234567890,
            "iat": 1234567800
        }"#;

        let claims: Claims = serde_json::from_str(json).unwrap();
        assert_eq!(claims.sub, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(claims.exp, 1234567890);
        assert_eq!(claims.iat, 1234567800);
    }

    #[test]
    fn test_pdp_error_display() {
        let err = PdpError::TokenDecodeError("Invalid token".to_string());
        assert!(err.to_string().contains("Token decode failed"));

        let err = PdpError::NetworkError("Connection refused".to_string());
        assert!(err.to_string().contains("Network error"));

        let err = PdpError::InvalidResponse("JSON parse error".to_string());
        assert!(err.to_string().contains("Invalid PDP response"));

        let err = PdpError::PdpUnavailable("Service down".to_string());
        assert!(err.to_string().contains("PDP unavailable"));
    }

    #[test]
    fn test_map_resource_schemas_list() {
        let resource = map_resource("/api/schemas");
        assert_eq!(resource, "schemas:0");
    }

    #[test]
    fn test_map_resource_schema_get() {
        let resource = map_resource("/api/schemas/123e4567-e89b-12d3-a456-426614174000");
        assert_eq!(resource, "schemas_123e4567_e89b_12d3_a456_426614174000:0");
    }

    #[test]
    fn test_map_resource_schema_export() {
        let resource = map_resource("/api/schemas/123e4567-e89b-12d3-a456-426614174000/export");
        assert_eq!(resource, "schemas_123e4567_e89b_12d3_a456_426614174000:0");
    }

    #[test]
    fn test_map_resource_collections() {
        let resource = map_resource("/api/collections");
        assert_eq!(resource, "collections:0");
    }

    #[test]
    fn test_map_resource_collection_item() {
        let resource = map_resource("/api/collections/abc-123");
        assert_eq!(resource, "collections_abc_123:0");
    }

    #[test]
    fn test_map_resource_product() {
        let resource = map_resource("/api/product");
        assert_eq!(resource, "product:0");
    }

    #[test]
    fn test_map_resource_product_item() {
        let resource = map_resource("/api/product/42");
        assert_eq!(resource, "product:42");
    }

    #[test]
    fn test_map_resource_datasource() {
        let resource = map_resource("/api/data-sources");
        assert_eq!(resource, "data_sources:0");
    }

    #[test]
    fn test_map_resource_non_schema() {
        let resource = map_resource("/api/other/path");
        assert_eq!(resource, "other_path:0");
    }

    #[test]
    fn test_map_resource_workspace_overview() {
        let resource = map_resource("/api/schedule/overview");
        assert_eq!(resource, "schedule_overview:0");

        let resource = map_resource("/api/global/overview");
        assert_eq!(resource, "global_overview:0");
    }

    #[test]
    fn test_verify_token_with_keys_no_keys() {
        let validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        let err = verify_token_with_keys(&[], "not-a-token", &validation);
        assert!(matches!(err, Err(PdpError::TokenDecodeError(_))));
    }

    #[test]
    fn test_verify_token_with_keys_bad_pem() {
        let validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        let bad = b"-----BEGIN PUBLIC KEY-----\nnot-a-key\n-----END PUBLIC KEY-----";
        let err = verify_token_with_keys(&[bad], "not-a-token", &validation);
        assert!(matches!(err, Err(PdpError::TokenDecodeError(_))));
    }

    #[test]
    fn test_verify_token_with_keys_second_key_used() {
        // 第一把钥解析失败但第二把是合法公钥：验证不因第一把坏钥短路（走第二把）
        let validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        let bad = b"-----BEGIN PUBLIC KEY-----\nnot-a-key\n-----END PUBLIC KEY-----";
        let err =
            verify_token_with_keys(&[bad, TEST_SSO_JWT_PUBLIC_KEY], "not-a-token", &validation);
        // token 无效（非坏钥导致），错误应为 decode 类而非坏钥短路
        assert!(matches!(err, Err(PdpError::TokenDecodeError(_))));
    }
}
