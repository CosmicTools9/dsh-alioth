//! SSO Authentication Middleware
//!
//! Protects sensitive routes (audit, NGAC, WebSocket) by validating JWT tokens.

use actix_web::{
    body::EitherBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    http::header,
    web, Error, HttpMessage, HttpResponse,
};
use chrono::Utc;
use futures::future::LocalBoxFuture;
use std::collections::HashMap;
use std::future::{ready, Ready};
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use common::{ErrorResponse, PublicRouteMatcher};
use sqlx::PgPool;

use super::jwt::validate_access_token;
use super::session::is_session_active;
use super::AuthState;
use crate::auth::extract_user_id_with_source;
use crate::ngac::pdp::{audit_decision, decide_access, Decision};
use serde_json::json;

/// 认证端点速率限制器（SECURITY_SPEC §4）：每 IP N 请求/窗口 令牌桶（内存近似）。
///
/// 认证端点速率限制（SECURITY_SPEC §4）：DB-backed 固定窗口（fix-sso-residual-gaps G3）。
///
/// 计数存 `isahl_auth.auth_rate_limits`（scope+ip+窗口起点联合主键，UPSERT 原子递增）——
/// 独立进程与 Gateway 内嵌共享同一 DB，限流跨实例一致（取代旧进程内
/// `Mutex<HashMap>` 近似实现，SECURITY_SPEC §11.1 内存态清单已同步移除）。
/// 窗口固定对齐 epoch（60s 默认），窗口边界最多放行 ≤2× 上限（固定窗口语义，接受）。
/// DB 故障 fail-open 放行并记日志（限流为防护面非授权面）；窗口行由 housekeeping 清理。
///
/// 上限与窗口通过环境变量调整（分档语义见 [`throttle_tier`]）：
/// - `RATE_LIMIT_MAX`：`credential` 档（凭证提交/交换面，防爆破目标），默认 10 请求/窗口；
/// - `RATE_LIMIT_SESSION_MAX`：`session` 档（会话与权限**只读**面），默认 120 请求/窗口；
/// - `RATE_LIMIT_REGISTER_MAX`：`register` 档（外部注册通道），默认 3 请求/窗口；
/// - `RATE_LIMIT_WINDOW_SEC`：三档共用窗口长度，默认 60 秒。
///
/// 从环境变量读取 `credential` 档上限（默认 10 请求/窗口）。
fn rate_limit_max() -> u32 {
    std::env::var("RATE_LIMIT_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10)
}

/// 从环境变量读取限流窗口秒数（默认 60 秒）。
fn rate_limit_window_secs() -> u64 {
    std::env::var("RATE_LIMIT_WINDOW_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60)
}

/// `session` 档上限（默认 120 请求/窗口）。
///
/// 口径依据（fix-auth-ratelimit-bucket-split）：会话/权限只读端点（`/api/auth/me`、
/// `/api/auth/portal-context`、`/api/auth/check-access`、`/api/auth/refresh`、
/// `/api/auth/mode`、`/api/auth/oauth/providers`、`/api/auth/sessions*`）
/// 在单文档一次「未登录 → 登录 → 首页 + N 次导航」中天然产生 6–10 次请求；与凭证面
/// （`credential`，10/60s 防爆破）同桶时**合法用户单独即可打满整桶**——2026-09-22 实测事故：
/// `silentRefresh` 固定 5s 冷却 ⇒ 12 次/分刷新重试打满配额，连启动探测与人工登录都被 429；
/// 共享出口 IP（NAT / 首跳未设转发头）时一人触顶即全体 429。故只读面独立成档并给足
/// 导航语义额度，凭证面口径保持不变。
fn rate_limit_session_max() -> u32 {
    std::env::var("RATE_LIMIT_SESSION_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120)
}

/// 外部注册限流上限（默认 3 请求/窗口；防护分级——公开注册面严于通用认证端点）。
fn register_rate_limit_max() -> u32 {
    std::env::var("RATE_LIMIT_REGISTER_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3)
}

/// 幂等建表（进程内一次；多进程并发 IF NOT EXISTS 安全）。
static ENSURE_TABLE: OnceLock<()> = OnceLock::new();

async fn ensure_rate_limit_table(pool: &PgPool) {
    if ENSURE_TABLE.get().is_some() {
        return;
    }
    let _ = sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.auth_rate_limits (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL,
            scope TEXT NOT NULL,
            ip TEXT NOT NULL,
            window_start TIMESTAMPTZ NOT NULL,
            count INTEGER NOT NULL DEFAULT 1,
            created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
            CONSTRAINT auth_rate_limits_pkey PRIMARY KEY (scope, ip, window_start)
        )",
    )
    .execute(pool)
    .await;
    let _ = ENSURE_TABLE.set(());
}

/// 限流判定结果：本次请求计入后的窗口内计数 + 是否放行。
///
/// 计数随判定一并返回，供 429 响应 `details` 与 warn 日志输出（现场可定位"谁打满的哪个桶"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RateLimitDecision {
    /// 窗口内累计计数（含本次请求）
    count: u32,
    /// 是否放行（`count <= max`）
    allowed: bool,
}

/// DB-backed 限流判定：原子递增当前窗口计数。
/// `Ok(decision)`：`decision.allowed == false` 即超限（调用方 429）；`Err` DB 故障（调用方 fail-open）。
async fn db_rate_limit_check(
    pool: &PgPool,
    scope: &str,
    ip: &str,
    max: u32,
    window_secs: u64,
) -> Result<RateLimitDecision, sqlx::Error> {
    ensure_rate_limit_table(pool).await;
    let now_epoch = Utc::now().timestamp();
    let window_epoch = now_epoch - now_epoch.rem_euclid(window_secs as i64);
    let window_start = chrono::DateTime::from_timestamp(window_epoch, 0).unwrap_or_else(Utc::now);
    let count: i32 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.auth_rate_limits (scope, ip, window_start, count) \
         VALUES ($1, $2, $3, 1) \
         ON CONFLICT (scope, ip, window_start) \
         DO UPDATE SET count = isahl_auth.auth_rate_limits.count + 1 \
         RETURNING count",
    )
    .bind(scope)
    .bind(ip)
    .bind(window_start)
    .fetch_one(pool)
    .await?;
    let count = count as u32;
    Ok(RateLimitDecision {
        count,
        allowed: count <= max,
    })
}

/// 是否为需要限流的认证端点路径。
fn is_auth_throttled_path(path: &str) -> bool {
    path.starts_with("/auth/") || path.starts_with("/api/auth/") || path.starts_with("/oidc/")
}

/// 外部主体注册通道（add-dual-register-channels）：独立更严限流档。
fn is_external_register_path(path: &str) -> bool {
    path == "/auth/register/external" || path == "/api/auth/register/external"
}

/// `session` 档（会话与权限**只读**面）白名单——**auth 前缀之后**的子路径。
///
/// 两种挂载形态共用同一份判定：SSO 独立部署（`Release/SSO`）挂 `/auth/...`，
/// Gateway 反代/别名形态挂 `/api/auth/...`（`sso_remote_proxy` 保留原路径转发）。
/// 故白名单只登记前缀之后的子路径，避免出现"只放宽了反代形态、独立形态仍按 10/60s"的裂口。
///
/// 放宽面必须**显式登记**：未列入本表的一切 `/auth/*`、`/api/auth/*`、`/oidc/*` 路径
/// 一律落 `credential` 默认档（更严）——新增端点漏登记只会更严，不会更松（fail-safe）。
/// 与 `openspec/specs/sso-auth-rate-limiting` 的分档表逐行对应。
const SESSION_READ_SUBPATHS: &[&str] = &[
    "/me",
    "/portal-context",
    "/check-access",
    "/refresh",
    "/mode",
    "/oauth/providers",
    "/sessions",
];

/// 段边界前缀匹配：`prefix` 命中自身或其子路径，MUST NOT 命中同前缀异段路径
/// （`/me` 命中 `/me` 与 `/me/profile`，不命中 `/messaging`）。
///
/// 不复用 `common::PublicRouteMatcher`：其 `is_public` 为裸 `starts_with`
/// （`Framework/backend/common/src/middleware.rs`），用作放宽档白名单会把
/// `/api/auth/messaging` 误判进 `session` 档 ⇒ 违反「默认档最严」。
fn matches_segment_prefix(path: &str, prefix: &str) -> bool {
    match path.strip_prefix(prefix) {
        Some(rest) => rest.is_empty() || rest.starts_with('/'),
        None => false,
    }
}

/// 剥掉认证挂载前缀（`/api/auth` 优先，其次 `/auth`）→ 返回前缀之后的子路径。
fn auth_subpath(path: &str) -> Option<&str> {
    path.strip_prefix("/api/auth")
        .or_else(|| path.strip_prefix("/auth"))
}

/// `session` 档路径判定（段边界白名单，见 [`SESSION_READ_SUBPATHS`]）。
fn is_session_read_path(path: &str) -> bool {
    match auth_subpath(path) {
        Some(subpath) => SESSION_READ_SUBPATHS
            .iter()
            .any(|prefix| matches_segment_prefix(subpath, prefix)),
        None => false,
    }
}

/// 限流档判定 → `(scope, 上限)`。
///
/// 顺序固定：`register`（精确匹配）→ `session`（段边界白名单）→ `credential`（默认档）。
fn throttle_tier(path: &str) -> (&'static str, u32) {
    if is_external_register_path(path) {
        ("register", register_rate_limit_max())
    } else if is_session_read_path(path) {
        ("session", rate_limit_session_max())
    } else {
        ("credential", rate_limit_max())
    }
}

/// Require valid JWT for all non-public routes
pub struct RequireAuth {
    matcher: PublicRouteMatcher,
}

impl RequireAuth {
    /// 使用默认公开路由列表创建中间件
    pub fn new() -> Self {
        Self {
            matcher: PublicRouteMatcher::new()
                .prefix("/auth/")
                .prefix("/api/auth/")
                .prefix("/oauth/")
                .prefix("/slo/")
                .prefix("/oidc/")
                .exact("/health")
                .exact("/.well-known/jwks.json")
                .exact("/.well-known/openid-configuration"),
        }
    }

    /// 使用自定义公开路由匹配器创建中间件
    pub fn with_matcher(matcher: PublicRouteMatcher) -> Self {
        Self { matcher }
    }
}

impl Default for RequireAuth {
    fn default() -> Self {
        Self::new()
    }
}

impl<S, B> Transform<S, ServiceRequest> for RequireAuth
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = RequireAuthService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RequireAuthService {
            service: Rc::new(service),
            matcher: self.matcher.clone(),
        }))
    }
}

pub struct RequireAuthService<S> {
    service: Rc<S>,
    matcher: PublicRouteMatcher,
}

impl<S, B> Service<ServiceRequest> for RequireAuthService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let matcher = self.matcher.clone();

        Box::pin(async move {
            let path = req.path().to_string();
            let method = req.method().to_string();
            log::debug!("SSO middleware: {} {}", method, path);

            if matcher.is_public(&path, req.method().as_str()) {
                log::debug!(
                    "SSO middleware: {} {} is public, bypassing auth",
                    method,
                    path
                );

                // 认证端点速率限制（SECURITY_SPEC §4）：DB-backed 固定窗口（G3），跨实例一致。
                // 分档（fix-auth-ratelimit-bucket-split）：`credential`（凭证提交/交换，防爆破）/
                // `session`（会话与权限只读）/ `register`（外部注册，更严）三档独立桶，
                // 档位判定见 `throttle_tier`。
                if is_auth_throttled_path(&path) {
                    // 分桶键 = 可信跳采信的客户端地址（2026-09-23 fix-auth-ratelimit-client-ip：
                    // 旧实现无条件读转发头 ⇒ 平台链路全部远程用户折进 127.0.0.1 同一桶，
                    // 且客户端可自带伪造头绕过限流；详见 `auth::session::client_ip` 文档）。
                    let remote_ip =
                        crate::auth::session::client_ip_from_conn_info(&req.connection_info());
                    if let Some(ip) = remote_ip {
                        // 分档（fix-auth-ratelimit-bucket-split）：凭证面 / 会话只读面 /
                        // 外部注册面各自独立桶，互不串扰；未登记路径落 credential 默认档。
                        let (scope, max) = throttle_tier(&path);
                        let window_secs = rate_limit_window_secs();
                        let decision = match req.app_data::<web::Data<PgPool>>() {
                            Some(pool) => {
                                db_rate_limit_check(
                                    pool.get_ref(),
                                    scope,
                                    &ip,
                                    max,
                                    window_secs,
                                )
                                .await
                                .unwrap_or_else(|e| {
                                    // DB 故障 fail-open（限流为防护面非授权面）
                                    log::warn!(
                                        "SSO middleware: rate limit DB check failed (fail-open): {}",
                                        e
                                    );
                                    RateLimitDecision {
                                        count: 0,
                                        allowed: true,
                                    }
                                })
                            }
                            None => {
                                log::error!("SSO middleware: PgPool missing for rate limiting");
                                RateLimitDecision {
                                    count: 0,
                                    allowed: true,
                                }
                            }
                        };
                        if !decision.allowed {
                            log::warn!(
                                "SSO middleware: rate limit exceeded ip={} path={} scope={} count={} limit={} window_seconds={}",
                                ip,
                                path,
                                scope,
                                decision.count,
                                max,
                                window_secs
                            );
                            return Ok(req
                                .into_response(
                                    HttpResponse::TooManyRequests()
                                        .insert_header((
                                            header::RETRY_AFTER,
                                            window_secs.to_string(),
                                        ))
                                        .json(
                                            ErrorResponse::new(
                                                "RATE_LIMITED",
                                                format!(
                                                    "Too many authentication requests, please retry after {} seconds",
                                                    window_secs
                                                ),
                                            )
                                            .with_details(json!({
                                                "scope": scope,
                                                "count": decision.count,
                                                "limit": max,
                                                "window_seconds": window_secs,
                                                "retry_after_seconds": window_secs,
                                            })),
                                        ),
                                )
                                .map_into_right_body());
                        }
                    }
                }

                let res = service.call(req).await?;
                log::debug!(
                    "SSO middleware: {} {} public response status {}",
                    method,
                    path,
                    res.status()
                );
                return Ok(res.map_into_left_body());
            }

            // Extract AuthState from app_data
            let auth_state = req.app_data::<actix_web::web::Data<AuthState>>().cloned();

            let auth_state = match auth_state {
                Some(state) => state,
                None => {
                    log::error!("SSO middleware: AuthState missing for {} {}", method, path);
                    return Ok(req
                        .into_response(
                            HttpResponse::InternalServerError()
                                .json(ErrorResponse::internal("Auth state missing")),
                        )
                        .map_into_right_body());
                }
            };

            match validate_access_token(req.request(), &auth_state.verification_keys()).await {
                Ok(claims) => {
                    // 会话吊销校验：当 token 绑定 SSO 会话（sid 非空）时，
                    // 必须会话仍处 active 才放行。否则登出/吊销后 token 有效期内
                    // 仍可访问 SSO 受保护端点（audit/NGAC/WebSocket）。
                    if !claims.sid.is_empty() {
                        let active = match req.app_data::<web::Data<PgPool>>() {
                            Some(pool) => is_session_active(pool, &claims.sid).await,
                            None => {
                                log::error!(
                                    "SSO middleware: PgPool missing, cannot verify session revocation"
                                );
                                false
                            }
                        };
                        if !active {
                            log::warn!(
                                "SSO middleware: session {} revoked/expired for {} {}",
                                claims.sid,
                                method,
                                path
                            );
                            return Ok(req
                                .into_response(
                                    HttpResponse::Unauthorized()
                                        .json(ErrorResponse::unauthorized("Session revoked")),
                                )
                                .map_into_right_body());
                        }
                    }
                    log::debug!("SSO middleware: {} {} auth accepted", method, path);
                    // CSRF 校验（implement-csrf-protection design.md §4）：
                    // 求值 MUST 在鉴权成功之后（失效会话已在上方短路为 401，不改变
                    // 401/403 次序语义）；凭据来源 = 实际完成鉴权者（Bearer 豁免；
                    // cookie 来源须 double-submit 相等）。开关∧非安全方法∧非豁免路径
                    // 由 `csrf_gate` 内部判定（/auth/refresh、/auth/logout 为显式豁免面）。
                    if let Some((_, source)) = common::csrf::resolve_credential(req.request()) {
                        if let Some(resp) = common::csrf::csrf_gate(req.request(), source) {
                            log::warn!(
                                "SSO middleware: CSRF rejected {} {} (source {:?})",
                                method,
                                path,
                                source
                            );
                            return Ok(req.into_response(resp).map_into_right_body());
                        }
                    }
                    // fix（tighten-pdp-decision-surface A）：claims 注入 extensions，
                    // 服务决策 handler 做主体一致性校验免二次验签
                    req.extensions_mut().insert(claims.clone());
                    let res = service.call(req).await?;
                    log::debug!(
                        "SSO middleware: {} {} protected response status {}",
                        method,
                        path,
                        res.status()
                    );
                    Ok(res.map_into_left_body())
                }
                Err(e) => {
                    log::warn!(
                        "SSO auth middleware rejected request to {} {}: {}",
                        method,
                        path,
                        e
                    );
                    Ok(req
                        .into_response(
                            HttpResponse::Unauthorized()
                                .json(ErrorResponse::unauthorized("Unauthorized")),
                        )
                        .map_into_right_body())
                }
            }
        })
    }
}

/// NGAC 资源级 PEP 中间件（M1）。
///
/// 仅作用于 SSO 自有管理面路由（`/api/admin/*`）。对请求按 `(entity, http-method)` 映射为
/// NGAC `(object, operation)`，复用 `ngac::pdp::decide_access`（含 **bootstrap Permit 兜底**，
/// 不会在无策略时锁死 SSO 管理员）做资源级决策：
/// - `enforce` 模式（默认，fail-closed，符合 NGAC_SPEC §4.3）：仅 `Decision::Permit` 放行，其余 403。
/// - `audit` 模式：仅记录决策、不阻断（灰度上线用）。
///
/// 绝不记录原始 JWT/Token；审计字段仅限 user_id / resource / action / decision（复用 `audit_decision`）。
/// 日志统一使用 `log` crate。资源标识遵循 NGAC_SPEC §2.1 `{type}:{id}`。
///
/// 注：scim 路由使用静态 Bearer token（`SCIM_BEARER_TOKEN`），不解析为 NGAC user_id，
/// 用户属性模型不适用，故不套用本 PEP；scim 维持静态 token 保护（fail-closed）。
pub struct NgacPep {
    mode: PepMode,
    cache: Arc<Mutex<HashMap<CacheKey, (Decision, Instant)>>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PepMode {
    Enforce,
    Audit,
}

type CacheKey = (i64, String, String);

/// PEP 决策缓存 TTL：降低每请求 PDP 策略加载压力（NGAC_SPEC §8）。
const PEP_CACHE_TTL: Duration = Duration::from_secs(5);

impl NgacPep {
    pub fn new() -> Self {
        let mode = match std::env::var("NGAC_PEP_MODE").as_deref() {
            Ok("audit") => PepMode::Audit,
            _ => PepMode::Enforce,
        };
        NgacPep {
            mode,
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for NgacPep {
    fn default() -> Self {
        Self::new()
    }
}

impl<S, B> Transform<S, ServiceRequest> for NgacPep
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = NgacPepService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(NgacPepService {
            service: Rc::new(service),
            mode: self.mode,
            cache: self.cache.clone(),
        }))
    }
}

pub struct NgacPepService<S> {
    service: Rc<S>,
    mode: PepMode,
    cache: Arc<Mutex<HashMap<CacheKey, (Decision, Instant)>>>,
}

impl<S, B> Service<ServiceRequest> for NgacPepService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let mode = self.mode;
        let cache = self.cache.clone();

        Box::pin(async move {
            let path = req.path().to_string();
            let method = req.method().as_str().to_string();

            // 仅对 /api/admin/* 与 /api/audit/* 做资源级拦截；其余透传。
            let (object_type, resource_id, action) = match map_pep_resource(&path, &method) {
                Some(v) => v,
                None => {
                    // 透传分支同样 gate（否则 `/api/admin/*` 未映射路径漏检）：
                    // 仅在 CSRF 面确实需要校验时解析来源；无凭据/验签失败不在此下结论
                    // （透传与 401 语义交由下游，MUST NOT 变 403）。
                    if common::csrf::csrf_requires_check(req.request()) {
                        if let Some(state) = req.app_data::<web::Data<AuthState>>().cloned() {
                            if let Ok((_, source)) =
                                extract_user_id_with_source(req.request(), &state)
                            {
                                if let Some(resp) = common::csrf::csrf_gate(req.request(), source) {
                                    log::warn!(
                                        "ngac_pep: CSRF rejected {} {} (source {:?})",
                                        method,
                                        path,
                                        source
                                    );
                                    return Ok(req.into_response(resp).map_into_right_body());
                                }
                            }
                        }
                    }
                    let res = service.call(req).await?;
                    return Ok(res.map_into_left_body());
                }
            };

            // 解析 AuthState 与 `(user_id, 凭据来源)`（JWT，由外层 RequireAuth 已校验）。
            // 来源 MUST 与产出 token 的提取分支同源（`extract_user_id_with_source`，
            // design.md §3——CSRF 豁免/校验判据的唯一输入）。
            let auth_state = match req.app_data::<web::Data<AuthState>>().cloned() {
                Some(s) => s,
                None => {
                    log::error!("ngac_pep: AuthState missing for {} {}", method, path);
                    return Ok(req
                        .into_response(
                            HttpResponse::InternalServerError()
                                .json(json!({"error": "auth state missing"})),
                        )
                        .map_into_right_body());
                }
            };
            let (user_id, credential_source) =
                match extract_user_id_with_source(req.request(), &auth_state) {
                    Ok(v) => v,
                    Err(resp) => {
                        return Ok(req.into_response(resp).map_into_right_body());
                    }
                };

            // CSRF 校验（design.md §4）：鉴权成功之后、NGAC 决策/分发之前短路 403 平铺体。
            if let Some(resp) = common::csrf::csrf_gate(req.request(), credential_source) {
                log::warn!(
                    "ngac_pep: CSRF rejected {} {} (source {:?})",
                    method,
                    path,
                    credential_source
                );
                return Ok(req.into_response(resp).map_into_right_body());
            }

            // PgPool（决策与审计落库所需）。
            let pool_data = match req.app_data::<web::Data<PgPool>>() {
                Some(p) => p.clone(),
                None => {
                    log::error!("ngac_pep: PgPool missing for {} {}", method, path);
                    return Ok(req
                        .into_response(
                            HttpResponse::InternalServerError()
                                .json(json!({"error": "PgPool missing"})),
                        )
                        .map_into_right_body());
                }
            };
            let pool = pool_data.get_ref().clone();

            let resource = format!("{}:{}", object_type, resource_id);

            // 决策（含短 TTL 缓存，避免每请求加载策略图）。
            let cache_key = (user_id, resource.clone(), action.clone());
            let decision = {
                let cached = {
                    let guard = cache.lock().unwrap();
                    guard
                        .get(&cache_key)
                        .map(|(d, ts)| (d.clone(), ts.elapsed() < PEP_CACHE_TTL))
                };
                match cached {
                    Some((d, true)) => d,
                    _ => {
                        let d = decide_access(&pool, user_id, &resource, &action).await;
                        cache
                            .lock()
                            .unwrap()
                            .insert(cache_key, (d.clone(), Instant::now()));
                        d
                    }
                }
            };

            // 审计：复用既有 fire-and-forget 管道，仅记 user_id/resource/action/decision，不记 Token。
            audit_decision(
                pool_data.clone(),
                user_id,
                &resource,
                &action,
                decision.clone(),
                req.request(),
            );

            match mode {
                PepMode::Audit => {
                    log::info!(
                        "ngac_pep[audit]: user={} {} {} -> {:?}",
                        user_id,
                        method,
                        path,
                        decision
                    );
                    let res = service.call(req).await?;
                    Ok(res.map_into_left_body())
                }
                PepMode::Enforce => {
                    if decision == Decision::Permit {
                        let res = service.call(req).await?;
                        Ok(res.map_into_left_body())
                    } else {
                        log::warn!(
                            "ngac_pep[enforce]: DENIED user={} {} {} -> {:?}",
                            user_id,
                            method,
                            path,
                            decision
                        );
                        Ok(req
                            .into_response(HttpResponse::Forbidden().json(json!({
                                "error": "resource access denied by NGAC",
                                "resource": resource,
                                "action": action
                            })))
                            .map_into_right_body())
                    }
                }
            }
        })
    }
}

/// SSO 管理/审计面 PEP 映射（**消费判定链 PEP 段唯一实现** `ResourceRegistry::resolve_path`）。
///
/// 映射声明（唯一实现）：`ResourceRegistry::with_sso_face_defaults` ——
/// `/api/admin/api-{clients,plans,subscriptions,reconcile}` → `openapi_admin:0`、
/// 其余 `/api/admin/…` → `sso_admin:0`、`/api/audit/…` → `sso_audit:0`。
/// 动作映射同样单源（`ngac_contract::action_for_method`：未知方法 ⇒ `access`）。
fn map_pep_resource(path: &str, method: &str) -> Option<(String, i64, String)> {
    static REGISTRY: std::sync::OnceLock<ngac_contract::ResourceRegistry> =
        std::sync::OnceLock::new();
    // 空子路径（`/api/admin/`）在旧实现中不拦截——保持该边界语义。
    if path.ends_with('/') {
        return None;
    }
    let registry =
        REGISTRY.get_or_init(|| ngac_contract::ResourceRegistry::new().with_sso_face_defaults());
    let resolved = registry.resolve_path(path)?;
    Some((
        resolved.type_name,
        resolved.resource_id,
        ngac_contract::action_for_method(method).to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> PgPool {
        let url = std::env::var("DATABASE_URL")
            .or_else(|_| std::env::var("SSO_TEST_DATABASE_URL"))
            .unwrap_or_else(|_| {
                let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_string());
                format!("postgres://{}@localhost:5432/aliothstudio_test", user)
            });
        PgPool::connect(&url)
            .await
            .expect("无法连接测试库，请先运行 `bash scripts/db/reset-db.sh --test`")
    }

    /// 固定窗口内前 max 次放行，超限拒绝；窗口翻转后恢复（G3 DB-backed 语义）。
    #[tokio::test]
    async fn test_db_rate_limit_allows_up_to_max_then_blocks() {
        let pool = test_pool().await;
        let scope = format!("test-{}", uuid::Uuid::new_v4().simple());
        let ip = format!("10.0.0.{}", rand::random::<u8>());

        // 清理历史（防残留污染断言）
        let _ = sqlx::query("DELETE FROM isahl_auth.auth_rate_limits WHERE scope = $1")
            .bind(&scope)
            .execute(&pool)
            .await;

        let max = 3u32;
        // 窗口 1s 在高负载机（多次认证轮实测）会跨窗翻转致假放行——放宽到 30s，
        // 测试总耗 << 窗长即确定
        let window_secs = 30u64;
        for i in 1..=max {
            let d = db_rate_limit_check(&pool, &scope, &ip, max, window_secs)
                .await
                .expect("check");
            assert!(d.allowed, "first {} requests should be allowed", max);
            assert_eq!(d.count, i, "计数随判定原子返回");
        }
        let blocked = db_rate_limit_check(&pool, &scope, &ip, max, window_secs)
            .await
            .expect("check");
        assert!(!blocked.allowed, "request above max should be blocked");
        assert_eq!(
            blocked.count,
            max + 1,
            "超限请求同样计入窗口计数（响应 details 用它定位）"
        );
        // 不同 key（ip）不受影响
        assert!(
            db_rate_limit_check(&pool, &scope, "10.0.0.254", max, window_secs)
                .await
                .expect("check")
                .allowed,
            "different ip should be allowed"
        );

        // 不同档（scope）互不串扰：同 IP 在另一档仍从 1 起（fix-auth-ratelimit-bucket-split）
        let scope_other = format!("test-other-{}", uuid::Uuid::new_v4().simple());
        let other = db_rate_limit_check(&pool, &scope_other, &ip, max, window_secs)
            .await
            .expect("check");
        assert!(other.allowed && other.count == 1, "档位之间互不串扰");
        let _ = sqlx::query("DELETE FROM isahl_auth.auth_rate_limits WHERE scope = $1")
            .bind(&scope_other)
            .execute(&pool)
            .await;

        // 窗口翻转语义（独立 scope 探针，免 sleep——高负载机跨窗翻转会假放行/假阻塞）：
        // 新 scope 预置「两个窗口前已打满」的历史行；当前窗口计数独立 ⇒ 应放行
        let scope_flip = format!("test-flip-{}", uuid::Uuid::new_v4().simple());
        sqlx::query(
            "INSERT INTO isahl_auth.auth_rate_limits (scope, ip, window_start, count) \
             VALUES ($1, $2, to_timestamp(floor(extract(epoch from now()) / $3)::bigint * $3 - $3 * 2), $4)",
        )
        .bind(&scope_flip)
        .bind(&ip)
        .bind(window_secs as i64)
        .bind(max as i32)
        .execute(&pool)
        .await
        .expect("seed expired window");
        assert!(
            db_rate_limit_check(&pool, &scope_flip, &ip, max, window_secs)
                .await
                .expect("check")
                .allowed,
            "new window should reset count"
        );
        let _ = sqlx::query("DELETE FROM isahl_auth.auth_rate_limits WHERE scope = $1")
            .bind(&scope_flip)
            .execute(&pool)
            .await;

        // 清理测试数据
        let _ = sqlx::query("DELETE FROM isahl_auth.auth_rate_limits WHERE scope = $1")
            .bind(&scope)
            .execute(&pool)
            .await;
    }

    #[test]
    fn test_is_auth_throttled_path() {
        assert!(is_auth_throttled_path("/auth/login"));
        assert!(is_auth_throttled_path("/api/auth/refresh"));
        assert!(!is_auth_throttled_path("/health"));
        assert!(!is_auth_throttled_path("/.well-known/jwks.json"));
        assert!(!is_auth_throttled_path("/api/ngac/eval"));
        assert!(!is_auth_throttled_path("/oauth/callback"));
    }

    /// 限流分档判定（fix-auth-ratelimit-bucket-split）：
    /// 会话只读白名单按**段边界**命中；未登记路径落 `credential` 默认档（fail-safe）。
    #[test]
    fn test_throttle_tier_classification() {
        let scope_of = |p: &str| throttle_tier(p).0;

        // 会话只读面（含子路径）——两种挂载前缀必须同档
        for p in [
            "/api/auth/me",
            "/api/auth/me/profile",
            "/api/auth/me/mfa/status",
            "/api/auth/portal-context",
            "/api/auth/check-access",
            "/api/auth/refresh",
            "/api/auth/mode",
            "/api/auth/oauth/providers",
            "/api/auth/sessions",
            "/api/auth/sessions/abc",
            "/auth/me",
            "/auth/me/profile",
            "/auth/portal-context",
            "/auth/check-access",
            "/auth/refresh",
            "/auth/mode",
            "/auth/oauth/providers",
            "/auth/sessions",
        ] {
            assert_eq!(scope_of(p), "session", "{} 应为 session 档", p);
        }

        // 段边界：同前缀异段 MUST NOT 被放宽；凭证面与其它认证路径落默认档
        for p in [
            "/api/auth/messaging",
            "/api/auth/men",
            "/api/auth/session-refresh-helper",
            "/auth/messaging",
            "/auth/men",
            "/auth/sessions-legacy",
            "/api/auth/login",
            "/api/auth/login/email-code",
            "/auth/login",
            "/api/auth/token",
            "/api/auth/introspect",
            "/api/auth/register",
            "/api/auth/reset-password/request",
            "/api/auth/webauthn/authenticate/start",
            // 外部主体签发面（move-apikey-issuance-to-external-portal D6）：凭据写面落更严默认档
            "/api/auth/external/api-clients",
            "/auth/external/api-clients",
            "/oidc/token",
            "/oidc/authorize",
        ] {
            assert_eq!(scope_of(p), "credential", "{} 应落 credential 默认档", p);
        }

        // 外部注册通道：精确匹配（子路径不享更严档，落默认档）
        assert_eq!(scope_of("/auth/register/external"), "register");
        assert_eq!(scope_of("/api/auth/register/external"), "register");
        assert_eq!(scope_of("/api/auth/register/external/x"), "credential");
    }

    /// 段边界前缀判定单测（`throttle_tier` 的判据基元）。
    #[test]
    fn test_matches_segment_prefix() {
        assert!(matches_segment_prefix("/me", "/me"));
        assert!(matches_segment_prefix("/me/profile", "/me"));
        assert!(!matches_segment_prefix("/messaging", "/me"));
        assert!(!matches_segment_prefix("/m", "/me"));
        assert!(!matches_segment_prefix("", "/me"));
    }

    /// 挂载前缀剥离：`/api/auth/*` 与 `/auth/*` 归一到同一子路径（独立部署形态同档）。
    #[test]
    fn test_auth_subpath_strips_both_mount_prefixes() {
        assert_eq!(auth_subpath("/api/auth/me"), Some("/me"));
        assert_eq!(auth_subpath("/auth/me"), Some("/me"));
        assert_eq!(auth_subpath("/api/auth"), Some(""));
        assert_eq!(auth_subpath("/auth"), Some(""));
        // 同前缀异段不剥成子路径（`/api/authx` 不得被当成 `/api/auth` + `x`）
        assert_eq!(auth_subpath("/api/authx"), Some("x"));
        assert_eq!(auth_subpath("/api/ngac/decide"), None);
        assert_eq!(auth_subpath("/health"), None);
    }

    #[test]
    fn test_map_pep_resource() {
        // admin 面 → sso_admin:0（不区分实体/资源 id）
        assert_eq!(
            map_pep_resource("/api/admin/users", "GET"),
            Some(("sso_admin".to_string(), 0, "read".to_string()))
        );
        assert_eq!(
            map_pep_resource("/api/admin/users/123", "DELETE"),
            Some(("sso_admin".to_string(), 0, "delete".to_string()))
        );
        assert_eq!(
            map_pep_resource("/api/admin/oidc/clients/5", "PUT"),
            Some(("sso_admin".to_string(), 0, "update".to_string()))
        );
        // audit 面 → sso_audit:0
        assert_eq!(
            map_pep_resource("/api/audit/events", "GET"),
            Some(("sso_audit".to_string(), 0, "read".to_string()))
        );
        assert_eq!(
            map_pep_resource("/api/audit/events/cleanup", "DELETE"),
            Some(("sso_audit".to_string(), 0, "delete".to_string()))
        );
        // 空子路径与 admin/audit 之外路径均透传（不拦截）。
        assert_eq!(map_pep_resource("/api/admin/", "GET"), None);
        assert_eq!(map_pep_resource("/health", "GET"), None);
        assert_eq!(map_pep_resource("/api/ngac/decide", "POST"), None);
        // OpenAPI 管理面四族独立 OA（长前缀优先于 /api/admin/）
        assert_eq!(
            map_pep_resource("/api/admin/api-clients", "GET"),
            Some(("openapi_admin".to_string(), 0, "read".to_string()))
        );
        assert_eq!(
            map_pep_resource("/api/admin/api-subscriptions/7", "POST"),
            Some(("openapi_admin".to_string(), 0, "create".to_string()))
        );
        // org-policy 管理面按族独立资源（add-org-policy-admin-resource-granularity）
        assert_eq!(
            map_pep_resource("/api/admin/ngac/org-policy/classes", "GET"),
            Some(("org_policy_admin".to_string(), 0, "read".to_string()))
        );
        assert_eq!(
            map_pep_resource("/api/admin/ngac/org-policy/rules/42", "PATCH"),
            Some(("org_policy_admin".to_string(), 0, "update".to_string()))
        );
        assert_eq!(
            map_pep_resource("/api/admin/ngac/org-policy/labels/L1", "DELETE"),
            Some(("org_policy_admin".to_string(), 0, "delete".to_string()))
        );
        // 未知方法 ⇒ access（单源动作映射；MUST NOT 退化为 read）
        assert_eq!(
            map_pep_resource("/api/admin/users", "TRACE"),
            Some(("sso_admin".to_string(), 0, "access".to_string()))
        );
    }
}
