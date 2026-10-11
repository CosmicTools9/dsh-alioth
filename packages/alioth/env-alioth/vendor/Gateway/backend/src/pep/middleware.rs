//! PEP (Policy Enforcement Point) Middleware for NGAC authorization.
//!
//! This middleware is responsible for:
//! 1. Extracting user identity from JWT tokens
//! 2. Calling SSO PDP (Policy Decision Point) for permission checks
//! 3. Enforcing access control decisions (Permit/Deny)
//! 4. Recording audit events for compliance
//! 5. Inserting RequestContext into request extensions for downstream handlers
//!
//! # 内部 seam（评审候选 5，2026-10-10）
//!
//! 内部关注点各成模块（外 interface `NgacEnforcer` 与组合根装配不变）：
//! `pep::jwt`（JWT 解码/验签）· `pep::session`（会话态校验）· `pep::pdp_client`（PDP 远调）
//! · `pep::audit`（审计投递）· `pep::obligations`（义务执行）· `pep::resource_map`（资源映射）。
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

use super::audit::{resolve_audit_subject, spawn_audit_event, AuditSpawn};
use super::cache::{ProbeOutcome, VersionProbe};
use super::jwt::{decode_jwt_token, TEST_SSO_JWT_PUBLIC_KEY};
use super::pdp_client::call_pdp_remote;
use super::resource_map::{map_resource, method_to_action, platform_resource_registry};
use super::session::is_session_active;
use crate::epp::Decision as AuditDecision;
use crate::pep::jwks::SsoJwksClient;
use actix_web::{
    body::EitherBody,
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage, HttpResponse,
};
use common::context::RequestContext;
use common::telemetry::{debug, error, warn};
use crud::audit_outbox::AuditScope;
use futures::future::{ok, LocalBoxFuture, Ready};
use ngac_contract::HttpNgacClient;

use sqlx::PgPool;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

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
    /// `Arc` 持有：注册表在 worker 生命周期内不变（alioth+wz 合计 145 条），
    /// 请求内只做引用计数拷贝，不再逐请求深拷贝整表（含 ~500 个 String）。
    resource_registry: Option<Arc<ngac_contract::ResourceRegistry>>,
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
        // dev 降级开关显式化（flag-dev-ngac-fail-open）：NGAC_FAIL_OPEN=true 时
        // PEP 完全跳过 PDP 且注入全量列授权 ⇒ **本进程 NGAC 授权不生效**。
        // 启动即告警一次（构造点 = 进程级单例），避免把 dev 语义误读为生产越权
        // （2026-09-15 WZ 验证实证：外部账号可读平台端点的唯一原因是该开关）。
        if std::env::var("NGAC_FAIL_OPEN")
            .unwrap_or_default()
            .eq_ignore_ascii_case("true")
        {
            warn!(
                "NGAC_FAIL_OPEN=true — PEP 跳过 PDP 并注入全量列授权：本进程 NGAC 授权不生效（dev 语义）；生产环境 MUST NOT 启用"
            );
        }
        // tighten-pdp-decision-surface B：探针 pool 直查（与 self.pool 同源）
        let probe_pool = pool.clone();
        Self {
            pool: Some(pool),
            jwt_public_key,
            jwt_public_key_prev,
            jwks_client: if has_pdp_endpoint {
                Some(Arc::new(SsoJwksClient::new(sso_service_url.clone())))
            } else {
                None
            },

            version_probe: Arc::new(VersionProbe::with_pool(probe_pool)),
            column_cache: std::sync::Arc::new(super::cache::ColumnCache::with_defaults()),
            ngac_client: if has_pdp_endpoint {
                Some(HttpNgacClient::new(sso_service_url))
            } else {
                None
            },
            issuer: "http://localhost:9002".to_string(),
            audience: "http://localhost:9002".to_string(),
            public_noauth_paths: HashSet::new(),
            resource_registry: Some(Arc::new(platform_resource_registry())),
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
            resource_registry: Some(Arc::new(platform_resource_registry())),
        }
    }

    /// 无 DB 池、但注入 PDP 客户端的形态（测试 seam）——用于验证 PEP 判定路径
    /// （list/decide 的发起与合并语义）而不依赖真实 SSO 与数据库：会话吊销校验与
    /// 审计写仅在持有 pool 时执行（`pool: None` 时跳过）。
    pub fn new_without_pool_with_pdp(sso_service_url: impl Into<String>) -> Self {
        Self {
            pool: None,
            jwt_public_key: TEST_SSO_JWT_PUBLIC_KEY.to_vec(),
            jwt_public_key_prev: Vec::new(),
            jwks_client: None,
            version_probe: Arc::new(VersionProbe::new()),
            column_cache: std::sync::Arc::new(super::cache::ColumnCache::with_defaults()),
            ngac_client: Some(HttpNgacClient::new(sso_service_url.into())),
            issuer: "http://localhost:9002".to_string(),
            audience: "http://localhost:9002".to_string(),
            public_noauth_paths: HashSet::new(),
            resource_registry: Some(Arc::new(platform_resource_registry())),
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
        self.resource_registry = Some(Arc::new(registry));
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

/// PDP 检查请求/响应与 `Claims`/`PdpError` 经 re-export 保持原公开路径
/// （实现已迁 `pep::pdp_client` / `pep::jwt`；评审候选 5 内部 seam 分缝，2026-10-10）。
pub use super::jwt::Claims;
pub use super::pdp_client::{PdpCheckRequest, PdpCheckResponse, PdpError};

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
    resource_registry: Option<Arc<ngac_contract::ResourceRegistry>>,
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
            //
            // 集合 = **静态**（组合根一次求值）平台自有面 ∪ 各 ns 自持声明
            // （`Gateway/host/src/main.rs :: platform_public_noauth_paths()`）；无运行期可变面。
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
                    debug!("No-auth path access permitted: {}", req.path());
                    let response = svc.call(req).await?;
                    return Ok(response.map_into_left_body());
                }
                // 服务令牌命中 noauth 路径 → 继续下方 JWT 校验 + PDP 流程
            }

            // 令牌提取 + 来源判定：**委托 `common::csrf::resolve_credential`（唯一实现）**。
            // change implement-csrf-protection design §3：优先级 = scoped cookie（`x-auth-cookie-scope`
            // 前缀 cookie）→ Bearer → 环境 cookie（`access_token`）；本处曾内联 Bearer→cookie
            // 两分支，与 `common::middleware` / SSO `jwt::extract_token` 构成三份互相冲突的
            // 实现（spec-audit F1），已删除。来源必须与令牌一同取出——它是下方 CSRF 豁免的
            // 唯一判据（Bearer ⇒ 豁免；cookie ⇒ 校验），MUST NOT 另立「存在某头即豁免」启发式。
            let (token, credential_source) = match common::csrf::resolve_credential(req.request()) {
                Some(resolved) => resolved,
                None => {
                    // 错误响应语义与旧内联分支逐字保持一致（401 + 三分类 message）：
                    // 无 Authorization 头 → 缺凭证；头存在但非 UTF-8 → 头非法；
                    // 头存在但非 `Bearer ` 形态 → 格式非法。
                    let message = match req.headers().get("Authorization") {
                        None => "Missing authorization header",
                        Some(h) if h.to_str().is_err() => "Invalid authorization header",
                        Some(_) => "Invalid authorization header format",
                    };
                    warn!("{}", message);
                    return Ok(req.into_response(
                        HttpResponse::Unauthorized()
                            .json(serde_json::json!({ "error": message }))
                            .map_into_right_body(),
                    ));
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

            // standalone token_version 校验（fix-sso-noauth-removal）：
            // standalone logout bump 版本后，已签发 token 即时失效（无 30min TTL 残留）。
            if let Some(tv) = claims.tv {
                let tv_ok = match &pool {
                    Some(p) => match claims.sub.parse::<i64>() {
                        Ok(uid) => {
                            match sqlx::query_scalar::<_, i64>(
                                "SELECT token_version FROM isahl_auth.standalone_users WHERE id = $1",
                            )
                            .bind(uid)
                            .fetch_optional(p)
                            .await
                            {
                                Ok(Some(current)) => current == tv,
                                Ok(None) | Err(_) => false,
                            }
                        }
                        Err(_) => false,
                    },
                    None => true, // 无 DB 池（测试）跳过
                };
                if !tv_ok {
                    warn!("Access denied: standalone token revoked (version mismatch)");
                    return Ok(req.into_response(
                        HttpResponse::Unauthorized()
                            .json(serde_json::json!({
                                "error": "Token revoked"
                            }))
                            .map_into_right_body(),
                    ));
                }
            }

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
                if let Some(db_pool) = &pool {
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

            // ── CSRF 门（change implement-csrf-protection design §4；Gateway PEP 插入点）──
            // 次序铁律：校验 MUST 在**鉴权成功之后**求值——至此令牌提取/解码、
            // token_version、会话吊销、服务客户端有效性四类 401 判定已全部通过
            // ⇒ 失效会话的写请求仍为 401，MUST NOT 上移（上移会把 401 变成 403，
            // 破坏前端 `api-auth-401` 的刷新/登出语义）。
            // 守卫（开关 ∧ 非安全方法 ∧ 非豁免路径）与判据（Bearer 豁免 / cookie
            // 常量时间 double-submit 比较）都在单源实现内部；失败体 = 平铺
            // `{code,message}`（`ErrorResponse`，MUST NOT 用旧式嵌套体）。
            if let Some(csrf_response) = common::csrf::csrf_gate(req.request(), credential_source) {
                warn!(
                    "CSRF rejected: user={} method={} path={} source={:?}",
                    user_id_str,
                    req.method(),
                    req.path(),
                    credential_source
                );
                return Ok(req.into_response(csrf_response.map_into_right_body()));
            }

            // 审计主体标识（fix-ngac-audit-subject-identity）：username 优先，
            // 回落 user:{id}；服务令牌以 client 标识为主体。
            let audit_subject = resolve_audit_subject(
                &claims.username,
                user_id_i64,
                is_service_token,
                &user_id_str,
            );
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
                debug!(
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
                        // 派生单源 = common::openapi_scope::service_entity_from_path（文档
                        // 对账测试同源消费）；其余路径回退 map_resource 首段。
                        common::openapi_scope::service_entity_from_path(req.path()).or_else(|| {
                            map_resource(req.path())
                                .split(':')
                                .next()
                                .map(|s| s.to_string())
                        })
                    });
                if let Some(rt) = resolved_for_scope {
                    // scope 文法/覆盖判定单源：common::openapi_scope（签发面校验、
                    // 文档 conformance 测试同源；等价旧内联 s==required||*:{rt}||*）。
                    let required = common::openapi_scope::required_scope(action, &rt);
                    let has_scope = common::openapi_scope::covers_any(&claims.scope, &required);
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

                // 产品投影判定（change `wire-openapi-product-projection`）：**scope 之后**追加，
                // 收窄方向单调——投影集非空时投影外业务端点 403 NOT_IN_PRODUCT_SET；
                // 投影集为空/含 `*` ⇒ 跳过（无产品数据的 ns 行为不变）。
                // 判定单源 = `openapi::projection`（文档面同源消费）；DB 缺失/查询失败 ⇒ 全量（不阻断）。
                if let Some(db) = pool.as_ref() {
                    let projection = crate::openapi::projection::global_cache().get(db).await;
                    if !projection.allows(req.path()) {
                        // `req` 在 `into_response` 中被移动 ⇒ 先取出展示用值
                        let rejected_path = req.path().to_string();
                        let status_line = projection.status_line();
                        warn!(
                            "Product projection: path not exposed by any product: user={} path={} ({})",
                            user_id_str, rejected_path, status_line
                        );
                        return Ok(req.into_response(
                            HttpResponse::Forbidden()
                                .json(serde_json::json!({
                                    "error": "NOT_IN_PRODUCT_SET",
                                    "reason": format!(
                                        "Path '{}' is not exposed by any OpenAPI product ({})",
                                        rejected_path, status_line
                                    )
                                }))
                                .map_into_right_body(),
                        ));
                    }
                }
            }

            debug!(
                "PEP checking access for user={} resource={} action={}",
                user_id_str, resource, action
            );

            // NGAC_FAIL_OPEN 判定提前到 PDP 判定之前：fail-open 下列表端点同样
            // 跳过 PDP（与 decide 路径一致，消除 standalone/fail-open 下列表恒 403），
            // 按 §2.7 降级为不注入 visible_ids（None → crud 无 RLS 约束）。
            let fail_open = std::env::var("NGAC_FAIL_OPEN")
                .unwrap_or_default()
                .eq_ignore_ascii_case("true");
            // 版本探针（remove-ngac-pep-decision-cache）：决策不缓存——权限立即
            // 生效为硬性要求，每次请求现查 PDP；探针仅服务列级缓存（ColumnCache，
            // association 派生、版本化 ≤2s）失效。探针失败 → 本请求列授权绕过
            // 缓存直查（不服务陈旧条目）。无 PDP 客户端（standalone/FAIL_OPEN/e2e）不探针。
            let column_cache_bypass = match &ngac_client {
                Some(_client) if !fail_open => matches!(
                    version_probe.ensure_fresh(&column_cache).await,
                    ProbeOutcome::Unavailable
                ),
                _ => false,
            };

            // 开发环境：NGAC_FAIL_OPEN=true 时完全跳过 PDP 调用，消除 SSO 依赖
            if fail_open {
                debug!(
                    "NGAC_FAIL_OPEN: skipping PDP for user={} resource={}",
                    user_id_str, resource
                );
                // 无 PDP 形态：注入显式全量列授权（与 standalone/e2e 交付语义一致），
                // 使「header 缺失」恒为异常信号（crud 对缺失 fail-closed）
                req.headers_mut().insert(
                    actix_web::http::header::HeaderName::from_static("x-authorized-columns"),
                    actix_web::http::header::HeaderValue::from_static("*"),
                );
                req.extensions_mut().insert(request_context);
                let response =
                    AuditScope::actor_scope(audit_subject.clone(), svc.call(req)).await?;
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

            // 行级可见性（仅 collection read 需要）与资源级决策是两次**输入互相独立**的
            // PDP 调用 → 并发发起（合并语义与错误优先级与串行版逐字一致，仅省一次往返等待）。
            let list_req = resolved.as_ref().and_then(|r| {
                (r.resource_id == 0 && action == "read").then(|| ngac_contract::PdpListRequest {
                    user_id: user_id_i64,
                    resource_type: r.type_name.clone(),
                    action: action.to_string(),
                })
            });
            let decide_fut = call_pdp_remote(ngac_client, &token, user_id_i64, &resource, action);
            let (list_outcome, decide_outcome) = match &list_req {
                Some(list_req) => {
                    let (list_res, decide_res) =
                        tokio::join!(ngac_client.list(list_req, &token), decide_fut);
                    (Some(list_res), decide_res)
                }
                None => (None, decide_fut.await),
            };

            // visible_ids=None = admin 全量（NGAC_SPEC §6.2）：不注入 header /
            // 不设置 RLS——crud 侧缺失 = 无约束（全表）。
            // Some([]) = 显式空授权 → 注入 `none`（恒假谓词零行）。
            // Some(ids) = 行级过滤集。
            let visible_ids = match list_outcome {
                None => None,
                Some(Ok(list_resp)) if list_resp.permitted => list_resp.visible_ids,
                Some(_) => {
                    // PDP list 调用失败或 permitted=false → fail-close：
                    // 缺失 header 在 crud 侧 = 无约束（全表），不得静默放行
                    warn!(
                        "NGAC list check failed/denied for user={} type={} action={}",
                        user_id_str,
                        resolved
                            .as_ref()
                            .map(|r| r.type_name.as_str())
                            .unwrap_or_default(),
                        action
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
            };

            // 义务随判决下发（wire-obligations-into-pep-audit）：审计行落
            // obligations_triggered——与 SSO 决策审计同源对称（§2.6）。
            let decide_outcome = decide_outcome.map(|resp| (resp.permitted, resp.obligations));
            match decide_outcome {
                Ok((true, obligations)) => {
                    if let (Some(visible), Some(resolved)) = (visible_ids, resolved.as_ref()) {
                        request_context
                            .set_visible_resource_ids(resolved.type_name.clone(), visible.clone());
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
                            actix_web::http::header::HeaderName::from_static("x-visible-ids"),
                            actix_web::http::header::HeaderValue::from_str(&header_val)
                                .unwrap_or_else(|_| {
                                    actix_web::http::header::HeaderValue::from_static("none")
                                }),
                        );
                    }
                    // Insert context AFTER visible_ids enrichment so downstream sees RLS
                    req.extensions_mut().insert(request_context);

                    debug!("Access permitted for user={}", user_id_str);
                    if let Some(db_pool) = &pool {
                        spawn_audit_event(
                            db_pool.clone(),
                            user_id_i64,
                            AuditSpawn {
                                subject: &audit_subject,
                                email: &user_email,
                                resource: &resource,
                                action,
                                decision: AuditDecision::Permit,
                                obligations,
                            },
                        );
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
                    // 血缘审计（data_change_logs/audit_outbox）操作者标识来源：
                    // 作用域内全部 enqueue 自动带上（SECURITY_SPEC §10.1 口径）
                    let response =
                        AuditScope::actor_scope(audit_subject.clone(), svc.call(req)).await?;
                    Ok(response.map_into_left_body())
                }
                Ok((false, obligations)) => {
                    warn!("Access denied for user={}", user_id_str);
                    if let Some(db_pool) = &pool {
                        spawn_audit_event(
                            db_pool.clone(),
                            user_id_i64,
                            AuditSpawn {
                                subject: &audit_subject,
                                email: &user_email,
                                resource: &resource,
                                action,
                                decision: AuditDecision::Deny,
                                obligations,
                            },
                        );
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
                Err(e) => {
                    error!("PDP check failed: {}", e);
                    // fail_open 已在上方短路；此处为 fail-close（生产语义）
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
    ///
    /// 参数（8）：缓存句柄、PDP 客户端、主体、资源类型、令牌类型、令牌、请求、绕缓存位——
    /// 均为互不重叠的独立输入（无自然聚合体），按 documented exemption 处置。
    #[allow(clippy::too_many_arguments)]
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
}
