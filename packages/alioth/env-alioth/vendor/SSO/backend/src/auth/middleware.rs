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
use crate::auth::extract_user_id;
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
/// 上限与窗口通过环境变量 `RATE_LIMIT_MAX` / `RATE_LIMIT_WINDOW_SEC` /
/// `RATE_LIMIT_REGISTER_MAX` 调整，默认 10 请求 / 60 秒（外部注册 3 请求/窗口）。
/// 从环境变量读取限流上限（默认 10 请求/窗口）。
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

/// DB-backed 限流判定：原子递增当前窗口计数。
/// `Ok(true)` 放行；`Ok(false)` 超限（调用方 429）；`Err` DB 故障（调用方 fail-open）。
async fn db_rate_limit_check(
    pool: &PgPool,
    scope: &str,
    ip: &str,
    max: u32,
    window_secs: u64,
) -> Result<bool, sqlx::Error> {
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
    Ok((count as u32) <= max)
}

/// 是否为需要限流的认证端点路径。
fn is_auth_throttled_path(path: &str) -> bool {
    path.starts_with("/auth/") || path.starts_with("/api/auth/") || path.starts_with("/oidc/")
}

/// 外部主体注册通道（add-dual-register-channels）：独立更严限流档。
fn is_external_register_path(path: &str) -> bool {
    path == "/auth/register/external" || path == "/api/auth/register/external"
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

                // 认证端点速率限制（SECURITY_SPEC §4）：DB-backed 固定窗口（G3），
                // 跨实例一致。防护分级：外部注册通道独立更严档。
                if is_auth_throttled_path(&path) {
                    let remote_ip = req
                        .connection_info()
                        .realip_remote_addr()
                        .map(|s| s.to_string());
                    if let Some(ip) = remote_ip {
                        let (scope, max) = if is_external_register_path(&path) {
                            ("register", register_rate_limit_max())
                        } else {
                            ("auth", rate_limit_max())
                        };
                        let window_secs = rate_limit_window_secs();
                        let allowed = match req.app_data::<web::Data<PgPool>>() {
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
                                    true
                                })
                            }
                            None => {
                                log::error!("SSO middleware: PgPool missing for rate limiting");
                                true
                            }
                        };
                        if !allowed {
                            log::warn!(
                                "SSO middleware: rate limit exceeded for {} on {}",
                                ip,
                                path
                            );
                            return Ok(req
                                .into_response(
                                    HttpResponse::TooManyRequests()
                                        .insert_header((header::RETRY_AFTER, "60"))
                                        .json(ErrorResponse::new(
                                            "RATE_LIMITED",
                                            "Too many authentication attempts, please retry after 60 seconds",
                                        )),
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
                    let res = service.call(req).await?;
                    return Ok(res.map_into_left_body());
                }
            };

            // 解析 AuthState 与 user_id（JWT，由外层 RequireAuth 已校验）。
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
            let user_id = match extract_user_id(req.request(), &auth_state) {
                Ok(id) => id,
                Err(resp) => {
                    return Ok(req.into_response(resp).map_into_right_body());
                }
            };

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

/// 将 SSO 管理面请求映射为 NGAC `(object_type, resource_id, action)`。
///
/// - `/api/admin/api-{clients,plans,subscriptions,reconcile}` → `(openapi_admin, 0, action)`
///   （OpenAPI 管理面独立 OA——refactor-openapi-admin-ngac-pdp，迁移 029）
/// - 其余 `/api/admin/<entity>(/<id>)?` → `(sso_admin, 0, action)`（管理面资源，不区分实体）
/// - `/api/audit/*` → `(sso_audit, 0, action)`（审计面资源）
///
/// 资源标识遵循 NGAC_SPEC §2.1 `{type}:{id}`；OA/关联由迁移 019（sso_admin/
/// sso_audit）与 029（openapi_admin）seed。
/// action：GET→read / POST→create / PUT|PATCH→update / DELETE→delete（其余→access）。
fn map_pep_resource(path: &str, method: &str) -> Option<(String, i64, String)> {
    // OpenAPI 管理面四族端点独立 OA（其余 admin 端点保持 sso_admin 不动）
    let (object_type, rest) = match path.strip_prefix("/api/admin/") {
        Some(r)
            if r.starts_with("api-clients")
                || r.starts_with("api-plans")
                || r.starts_with("api-subscriptions")
                || r.starts_with("api-reconcile") =>
        {
            ("openapi_admin".to_string(), r)
        }
        Some(r) => ("sso_admin".to_string(), r),
        None => {
            let r = path.strip_prefix("/api/audit/")?;
            ("sso_audit".to_string(), r)
        }
    };
    if rest.is_empty() {
        return None;
    }
    let action = match method.to_uppercase().as_str() {
        "GET" => "read",
        "POST" => "create",
        "PUT" | "PATCH" => "update",
        "DELETE" => "delete",
        _ => "access",
    };
    Some((object_type, 0, action.to_string()))
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
        let window_secs = 1u64;
        for _ in 0..max {
            assert!(
                db_rate_limit_check(&pool, &scope, &ip, max, window_secs)
                    .await
                    .expect("check"),
                "first {} requests should be allowed",
                max
            );
        }
        assert!(
            !db_rate_limit_check(&pool, &scope, &ip, max, window_secs)
                .await
                .expect("check"),
            "request above max should be blocked"
        );
        // 不同 key（ip）不受影响
        assert!(
            db_rate_limit_check(&pool, &scope, "10.0.0.254", max, window_secs)
                .await
                .expect("check"),
            "different ip should be allowed"
        );

        // 窗口翻转（1s 窗口）→ 恢复放行
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(
            db_rate_limit_check(&pool, &scope, &ip, max, window_secs)
                .await
                .expect("check"),
            "new window should reset count"
        );

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
    }
}
