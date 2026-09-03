//! Alioth 中间件基础设施
//!
//! 提供跨应用共享的中间件组件，消除 Meta/Gateway/SSO 中的重复模式。

use std::sync::Arc;

// ── 公开路由匹配器 ──────────────────────────────────────────────────────────

/// 可配置的公开路由匹配器
///
/// 替代中间件中硬编码的公开路由列表，支持前缀匹配和精确匹配。
///
/// # 示例
///
/// ```rust,ignore
/// let matcher = PublicRouteMatcher::new()
///     .prefix("/api/meta/auth/")
///     .prefix("/health")
///     .exact("/api/meta/mise/config")
///     .predicate(|path, method| path.starts_with("/api/meta/mise/") && method == "GET");
///
/// assert!(matcher.is_public("/api/meta/auth/login", "POST"));
/// assert!(!matcher.is_public("/api/meta/users", "GET"));
/// ```
#[derive(Clone, Default)]
pub struct PublicRouteMatcher {
    prefixes: Arc<Vec<String>>,
    exact: Arc<Vec<String>>,
}

impl PublicRouteMatcher {
    /// 创建空匹配器（默认所有路由都不公开）
    pub fn new() -> Self {
        Self::default()
    }

    /// 添加前缀匹配规则
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        Arc::make_mut(&mut self.prefixes).push(prefix.into());
        self
    }

    /// 添加精确匹配规则
    pub fn exact(mut self, path: impl Into<String>) -> Self {
        Arc::make_mut(&mut self.exact).push(path.into());
        self
    }

    /// 从多个前缀批量创建
    pub fn from_prefixes(prefixes: &[&str]) -> Self {
        Self {
            prefixes: Arc::new(prefixes.iter().map(|s| s.to_string()).collect()),
            exact: Arc::new(Vec::new()),
        }
    }

    /// 判断给定路径和方法是否为公开路由
    pub fn is_public(&self, path: &str, _method: &str) -> bool {
        // 精确匹配
        if self.exact.iter().any(|e| e == path) {
            return true;
        }
        // 前缀匹配
        if self.prefixes.iter().any(|p| path.starts_with(p)) {
            return true;
        }
        false
    }
}

// ── 认证上下文 trait ────────────────────────────────────────────────────────

/// 标准化认证上下文
///
/// 由认证中间件设置，供 handler 和下游中间件读取。
/// 实现此 trait 的类型应通过 actix-web 的 `req.extensions().insert(ctx)` 注入。
pub trait AuthContext: Clone + Send + Sync + 'static {
    /// 用户主键（ZUID）
    fn user_id(&self) -> i64;
    /// 用户邮箱
    fn email(&self) -> Option<String>;
    /// 用户名
    fn username(&self) -> Option<String>;
    /// 是否超级管理员
    fn is_superuser(&self) -> bool;
}

// ── NGAC PEP JWT 中间件 ─────────────────────────────────────────────────────

/// NGAC PEP Middleware — JWT authentication only.
///
/// Authorization is enforced centrally by Gateway NgacEnforcer.
/// This middleware is used when the module runs standalone during development.
use actix_web::{
    body::EitherBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage, HttpResponse,
};
use base64::Engine;
use futures::future::LocalBoxFuture;
use serde::{Deserialize, Serialize};
use std::future::{ready, Ready};
use std::rc::Rc;

use crate::context::RequestContext;

/// NGAC PEP Middleware
///
/// Performs ES256 JWT verification (EC P-256) and inserts RequestContext into
/// extensions. Permission checks are handled by Gateway in production.
///
/// 与 SSO 签发端对齐的 JWT 契约：算法 ES256、SSO 私钥签发 / 本中间件持公钥验证。
/// 禁止 HS256 `JWT_SECRET`（已废弃，见 ENVIRONMENT_SPEC §6.1）。
#[derive(Clone)]
pub struct NgacPepMiddleware {
    /// SSO ES256 验证公钥（PEM）
    public_key_pem: String,
}

impl NgacPepMiddleware {
    /// Create middleware with an explicit ES256 public key (PEM)
    pub fn new(public_key_pem: impl Into<String>) -> Self {
        Self {
            public_key_pem: public_key_pem.into(),
        }
    }

    /// Create middleware from environment variables
    ///
    /// 优先读 `SSO_JWT_PUBLIC_KEY`（非空且非 `enc:` 密文），
    /// 否则读 `SSO_JWT_PUBLIC_KEY_PATH` 指向的 PEM 文件。
    /// 两者皆无 → panic（对齐 SECURITY_SPEC §6：禁止硬编码默认密钥）。
    pub fn from_env() -> Self {
        let pem = load_sso_public_key_pem();
        // 启动期 fail-fast：公钥必须是合法 EC P-256 PEM
        jsonwebtoken::DecodingKey::from_ec_pem(pem.as_bytes())
            .unwrap_or_else(|e| panic!("SSO_JWT_PUBLIC_KEY is not a valid EC P-256 PEM: {}", e));
        Self::new(pem)
    }
}

/// 加载 SSO ES256 验证公钥（PEM）
///
/// 优先 `SSO_JWT_PUBLIC_KEY`（非空且非 `enc:` 密文），否则读取
/// `SSO_JWT_PUBLIC_KEY_PATH` 指向的文件。两者皆无时 panic——
/// 不再回退到硬编码默认密钥（SECURITY_SPEC §6）。
fn load_sso_public_key_pem() -> String {
    if let Ok(key) = std::env::var("SSO_JWT_PUBLIC_KEY") {
        if !key.is_empty() && !key.starts_with("enc:") {
            return key;
        }
    }
    if let Ok(path) = std::env::var("SSO_JWT_PUBLIC_KEY_PATH") {
        if !path.is_empty() {
            return std::fs::read_to_string(&path)
                .unwrap_or_else(|e| {
                    panic!("Failed to read SSO_JWT_PUBLIC_KEY_PATH {}: {}", path, e)
                })
                .trim()
                .to_string();
        }
    }
    panic!(
        "SSO_JWT_PUBLIC_KEY / SSO_JWT_PUBLIC_KEY_PATH 未配置：\
         NgacPepMiddleware 需要 ES256 验证公钥（SECURITY_SPEC §6 禁止硬编码默认密钥）"
    );
}

impl<S, B> Transform<S, ServiceRequest> for NgacPepMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = NgacPepMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(NgacPepMiddlewareService {
            service: Rc::new(service),
            public_key_pem: self.public_key_pem.clone(),
        }))
    }
}

pub struct NgacPepMiddlewareService<S> {
    service: Rc<S>,
    public_key_pem: String,
}

impl<S, B> Service<ServiceRequest> for NgacPepMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let public_key_pem = self.public_key_pem.clone();

        Box::pin(async move {
            let path = req.path();

            // Allow health checks without auth
            if path == "/health" || path.starts_with("/health/") {
                let res = service.call(req).await?;
                return Ok(res.map_into_left_body());
            }

            // Extract JWT token
            let token = match extract_token(req.request()) {
                Some(t) => t,
                None => {
                    let response = HttpResponse::Unauthorized()
                        .json(error_body("MISSING_AUTH", "Authentication required"));
                    return Ok(req.into_response(response).map_into_right_body());
                }
            };

            // Verify token (ES256, EC P-256 public key)
            let claims = match verify_token(&token, &public_key_pem) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("JWT verification failed: {}", e);
                    let response = HttpResponse::Unauthorized().json(error_body(
                        "INVALID_TOKEN",
                        &format!("Invalid or expired token: {}", e),
                    ));
                    return Ok(req.into_response(response).map_into_right_body());
                }
            };

            // Build request context
            let user_id = claims.sub.parse().unwrap_or(0);
            let context =
                RequestContext::with_username(user_id, claims.username.clone(), claims.username);

            req.extensions_mut().insert(context);
            req.extensions_mut().insert(user_id);
            let res = service.call(req).await?;
            Ok(res.map_into_left_body())
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    username: String,
    #[serde(default)]
    #[serde(with = "crate::serde_zuid")]
    exp: i64,
}

fn verify_token(token: &str, public_key_pem: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
    // 仅接受 ES256（EC P-256）签名，对齐 SSO 签发端
    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = true;
    decode::<Claims>(
        token,
        &DecodingKey::from_ec_pem(public_key_pem.as_bytes())?,
        &validation,
    )
    .map(|data| data.claims)
}

fn extract_token(req: &actix_web::HttpRequest) -> Option<String> {
    if let Some(cookie) = req.cookie("access_token") {
        let value = cookie.value().trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    req.headers()
        .get(actix_web::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|auth| auth.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
}

fn error_body(code: &str, message: &str) -> serde_json::Value {
    serde_json::json!({
        "error": code,
        "message": message
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{http, test, web, App, HttpResponse};

    async fn echo_handler(req: actix_web::HttpRequest) -> HttpResponse {
        let ctx = RequestContext::from_request(&req);
        HttpResponse::Ok().json(serde_json::json!({"ok": true, "user": ctx.map(|c| c.user_id)}))
    }

    // 测试用 EC P-256 密钥对（与 auth.rs 测试同源，ES256）
    const TEST_EC_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg2wKAEH0lCQSd/7Ro
sPTNdBk/FA+0v4ySiQgKfEvyXC+hRANCAAQa4oJDdj0j4r9uhXyXkEM74YhrfymG
kLbde5YJ9O/mbHMcihareS5r7WuUT39QG078mQFzg2z0ELuBivpRAmCc
-----END PRIVATE KEY-----"#;

    const TEST_EC_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEGuKCQ3Y9I+K/boV8l5BDO+GIa38p
hpC23XuWCfTv5mxzHIoWq3kua+1rlE9/UBtO/JkBc4Ns9BC7gYr6UQJgnA==
-----END PUBLIC KEY-----"#;

    fn test_public_key() -> &'static str {
        TEST_EC_PUBLIC_KEY
    }

    fn make_token(claims: &Claims, private_key_pem: &str) -> String {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        // SSO 签发端按 RFC 7519 NumericDate 以 JSON 数字签发 exp（Claims 的
        // serde_zuid 字符串编码仅用于 ZUID 字段；exp 数字编码是验证端硬性要求）
        let payload = serde_json::json!({
            "sub": claims.sub,
            "username": claims.username,
            "exp": claims.exp,
        });
        encode(
            &Header::new(Algorithm::ES256),
            &payload,
            &EncodingKey::from_ec_pem(private_key_pem.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    #[actix_rt::test]
    async fn test_public_path_allowed() {
        let app = test::init_service(
            App::new()
                .wrap(NgacPepMiddleware::new(test_public_key()))
                .route("/health", web::get().to(echo_handler)),
        )
        .await;
        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
    }

    #[actix_rt::test]
    async fn test_missing_token_rejected() {
        let app = test::init_service(
            App::new()
                .wrap(NgacPepMiddleware::new(test_public_key()))
                .route("/products", web::post().to(echo_handler)),
        )
        .await;
        let req = test::TestRequest::post().uri("/products").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
    }

    #[actix_rt::test]
    async fn test_valid_token_allowed() {
        let app = test::init_service(
            App::new()
                .wrap(NgacPepMiddleware::new(test_public_key()))
                .route("/products", web::post().to(echo_handler)),
        )
        .await;
        let token = make_token(
            &Claims {
                sub: "42".to_string(),
                username: "user".to_string(),
                exp: i64::MAX,
            },
            TEST_EC_PRIVATE_KEY,
        );
        let req = test::TestRequest::post()
            .uri("/products")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
    }

    #[actix_rt::test]
    async fn test_cookie_token_extracted() {
        let app = test::init_service(
            App::new()
                .wrap(NgacPepMiddleware::new(test_public_key()))
                .route("/products", web::post().to(echo_handler)),
        )
        .await;
        let token = make_token(
            &Claims {
                sub: "99".to_string(),
                username: "cookie_user".to_string(),
                exp: i64::MAX,
            },
            TEST_EC_PRIVATE_KEY,
        );
        let req = test::TestRequest::post()
            .uri("/products")
            .cookie(actix_web::cookie::Cookie::new("access_token", token))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
    }

    #[actix_rt::test]
    async fn per_user_key_extracts_sub_claim() {
        let token = make_token(
            &Claims {
                sub: "42".to_string(),
                username: "user42".to_string(),
                exp: i64::MAX,
            },
            TEST_EC_PRIVATE_KEY,
        );
        let req = test::TestRequest::default()
            .insert_header(("Authorization", format!("Bearer {token}")))
            .to_srv_request();
        let mw = RateLimitMiddleware::per_user_any(&["/api/files"], 20.0, 20.0 / 60.0);
        assert_eq!((mw.key_extractor)(&req), "user:42");
    }

    #[actix_rt::test]
    async fn per_user_key_falls_back_to_ip_without_token() {
        let req = test::TestRequest::default().to_srv_request();
        let mw = RateLimitMiddleware::per_user_any(&["/api/files"], 20.0, 20.0 / 60.0);
        let ip = req
            .connection_info()
            .realip_remote_addr()
            .unwrap_or("unknown")
            .to_string();
        assert_eq!((mw.key_extractor)(&req), ip);
    }

    #[actix_rt::test]
    async fn per_user_key_falls_back_to_ip_on_garbage_token() {
        let req = test::TestRequest::default()
            .insert_header(("Authorization", "Bearer not-a-jwt"))
            .to_srv_request();
        let mw = RateLimitMiddleware::per_user_any(&["/api/files"], 20.0, 20.0 / 60.0);
        let ip = req
            .connection_info()
            .realip_remote_addr()
            .unwrap_or("unknown")
            .to_string();
        assert_eq!((mw.key_extractor)(&req), ip);
    }
}

// ── Rate Limiting Middleware ────────────────────────────────────────────────

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug)]
pub struct TokenBucket {
    tokens: f64,
    capacity: f64,
    refill_rate: f64,
    last_refill: Instant,
}

impl TokenBucket {
    pub fn new(capacity: f64, refill_rate: f64) -> Self {
        Self {
            tokens: capacity,
            capacity,
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    pub fn try_consume(&mut self, n: f64) -> bool {
        self.refill();
        if self.tokens >= n {
            self.tokens -= n;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = now;
    }
}

#[derive(Debug, Clone)]
pub struct RateLimiter {
    buckets: Arc<Mutex<HashMap<String, TokenBucket>>>,
    capacity: f64,
    refill_rate: f64,
}

impl RateLimiter {
    pub fn new(capacity: f64, refill_rate: f64) -> Self {
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            capacity,
            refill_rate,
        }
    }

    pub fn try_consume(&self, key: &str, cost: f64) -> bool {
        let mut buckets = self.buckets.lock().unwrap();
        let bucket = buckets
            .entry(key.to_string())
            .or_insert_with(|| TokenBucket::new(self.capacity, self.refill_rate));
        bucket.try_consume(cost)
    }

    /// 桶容量（用于 X-RateLimit-Limit 头）。
    pub fn capacity(&self) -> f64 {
        self.capacity
    }

    /// 补率（用于估算 Retry-After 窗口）。
    pub fn refill_rate(&self) -> f64 {
        self.refill_rate
    }
}

#[derive(Clone)]
pub struct RateLimitMiddleware {
    limiter: RateLimiter,
    key_extractor: fn(&ServiceRequest) -> String,
    cost: f64,
    path_prefixes: Vec<String>,
}

impl RateLimitMiddleware {
    pub fn per_ip(path_prefix: &str, capacity: f64, refill_rate: f64) -> Self {
        Self {
            limiter: RateLimiter::new(capacity, refill_rate),
            key_extractor: |req: &ServiceRequest| {
                req.connection_info()
                    .realip_remote_addr()
                    .unwrap_or("unknown")
                    .to_string()
            },
            cost: 1.0,
            path_prefixes: vec![path_prefix.to_string()],
        }
    }

    pub fn per_ip_any(path_prefixes: &[&str], capacity: f64, refill_rate: f64) -> Self {
        Self {
            limiter: RateLimiter::new(capacity, refill_rate),
            key_extractor: |req: &ServiceRequest| {
                req.connection_info()
                    .realip_remote_addr()
                    .unwrap_or("unknown")
                    .to_string()
            },
            cost: 1.0,
            path_prefixes: path_prefixes.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// per-client 限流（openapi-external-access）：以服务令牌的 `svc_user_id`
    /// 为限流 key（不同 client 互不影响）；无令牌/自然人回退 per-IP。
    ///
    /// 与 per_ip 不同，key 从 Authorization Bearer JWT 的 `svc_user_id` claim
    /// 提取（不验签——限流 key 无安全语义，PEP 后续验签兜底；仅用于隔离）。
    pub fn per_client(path_prefixes: &[&str], capacity: f64, refill_rate: f64) -> Self {
        Self {
            limiter: RateLimiter::new(capacity, refill_rate),
            key_extractor: |req: &ServiceRequest| {
                let ip = req
                    .connection_info()
                    .realip_remote_addr()
                    .unwrap_or("unknown")
                    .to_string();
                let auth = req
                    .headers()
                    .get(actix_web::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                let Some(token) = auth.strip_prefix("Bearer ") else {
                    return ip;
                };
                match jwt_claim(token, "svc_user_id").and_then(|v| v.as_i64()) {
                    Some(uid) if uid > 0 => format!("client:{}", uid),
                    _ => ip,
                }
            },
            cost: 1.0,
            path_prefixes: path_prefixes.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// per-user 限流（file-manager 上传防滥用，SECURITY_SPEC §4：20 req/min/user）：
    /// 以 JWT `sub` claim（自然用户 id，String）为限流 key；无令牌/解析失败回退 per-IP。
    /// 与 per_client 同模式：不验签解码（限流 key 无安全语义，PEP 后续验签兜底）。
    pub fn per_user_any(path_prefixes: &[&str], capacity: f64, refill_rate: f64) -> Self {
        Self {
            limiter: RateLimiter::new(capacity, refill_rate),
            key_extractor: |req: &ServiceRequest| {
                let ip = req
                    .connection_info()
                    .realip_remote_addr()
                    .unwrap_or("unknown")
                    .to_string();
                let auth = req
                    .headers()
                    .get(actix_web::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                let Some(token) = auth.strip_prefix("Bearer ") else {
                    return ip;
                };
                match jwt_claim(token, "sub").and_then(|v| v.as_str().map(String::from)) {
                    Some(sub) if !sub.is_empty() => format!("user:{sub}"),
                    _ => ip,
                }
            },
            cost: 1.0,
            path_prefixes: path_prefixes.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// 不验签解码 JWT payload 指定 claim（限流 key 提取用；签名校验由 PEP 承担）。
fn jwt_claim(token: &str, claim: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1).unwrap_or("");
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.as_bytes())
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(b.as_slice()).ok())
        .and_then(|v| v.get(claim).cloned())
}

impl<S, B> Transform<S, ServiceRequest> for RateLimitMiddleware
where
    S: actix_web::dev::Service<
            ServiceRequest,
            Response = ServiceResponse<B>,
            Error = actix_web::Error,
        > + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = actix_web::Error;
    type InitError = ();
    type Transform = RateLimitMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RateLimitMiddlewareService {
            service: Rc::new(service),
            limiter: self.limiter.clone(),
            key_extractor: self.key_extractor,
            cost: self.cost,
            path_prefixes: self.path_prefixes.clone(),
        }))
    }
}

pub struct RateLimitMiddlewareService<S> {
    service: Rc<S>,
    limiter: RateLimiter,
    key_extractor: fn(&ServiceRequest) -> String,
    cost: f64,
    path_prefixes: Vec<String>,
}

impl<S, B> actix_web::dev::Service<ServiceRequest> for RateLimitMiddlewareService<S>
where
    S: actix_web::dev::Service<
            ServiceRequest,
            Response = ServiceResponse<B>,
            Error = actix_web::Error,
        > + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = actix_web::Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let limiter = self.limiter.clone();
        let key_extractor = self.key_extractor;
        let cost = self.cost;
        let path_prefixes = self.path_prefixes.clone();

        Box::pin(async move {
            let path = req.path();
            if path_prefixes.iter().any(|p| path.starts_with(p)) {
                let key = (key_extractor)(&req);
                if !limiter.try_consume(&key, cost) {
                    // 标准限流响应头（openapi-external-access）：429 + Retry-After
                    // + X-RateLimit-Limit/Remaining/Reset。Retry-After 以桶重置
                    // 时间（capacity / refill_rate 秒）为估算窗口。
                    // refill_rate=0（一次性桶/测试无补充配置）无确定重置窗口 →
                    // 报 0；避免 capacity/0=inf 经 as u64 饱和为 u64::MAX 后
                    // 与 timestamp 相加溢出 panic（fix-gateway-baseline-tests 1.1）。
                    let retry_after = if limiter.refill_rate() > 0.0 {
                        (limiter.capacity() / limiter.refill_rate()).ceil() as u64
                    } else {
                        0
                    };
                    return Ok(req
                        .into_response(
                            HttpResponse::TooManyRequests()
                                .insert_header(("Retry-After", retry_after.to_string()))
                                .insert_header((
                                    "X-RateLimit-Limit",
                                    limiter.capacity().to_string(),
                                ))
                                .insert_header(("X-RateLimit-Remaining", "0"))
                                .insert_header((
                                    "X-RateLimit-Reset",
                                    (chrono::Utc::now().timestamp().max(0) as u64)
                                        .saturating_add(retry_after)
                                        .to_string(),
                                ))
                                .json(serde_json::json!({
                                    "error": "RATE_LIMITED",
                                    "message": "Too many requests, please try again later"
                                })),
                        )
                        .map_into_right_body());
                }
            }
            let res = service.call(req).await?;
            Ok(res.map_into_left_body())
        })
    }
}
