//! OpenAPI 计量与配额（openapi-external-access P2）
//!
//! `ApiUsageMiddleware`：
//! 1. 识别服务令牌（JWT `svc_user_id` > 0，PEP 已验签）
//! 2. 解析订阅（api_clients → api_subscriptions → plan 配额，60s 缓存）
//! 3. 配额检查：日/月窗口 `api_usage` COUNT（quota_daily/quota_monthly > 0 时），
//!    超限 → 429 + Retry-After
//! 4. 响应后异步批量写入 `api_usage`（channel + worker，不阻塞请求路径）
//!
//! 自然人令牌（svc_user_id=0）不计量——OpenAPI 计费面仅服务调用方。

use actix_web::{
    body::EitherBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpResponse,
};
use base64::Engine;
use futures::future::LocalBoxFuture;
use serde_json::json;
use sqlx::PgPool;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// 订阅解析缓存 TTL（60s —— 订阅变更低频，容忍 1 分钟延迟）
const SUBSCRIPTION_CACHE_TTL: Duration = Duration::from_secs(60);
/// 计量写入队列容量（请求侧不阻塞：满则丢弃计数，仅记日志）
const METERING_QUEUE_CAPACITY: usize = 4096;
/// 配额检查缓存 TTL（5s —— 窗口计数低频变化，容忍短延迟）
const QUOTA_CHECK_CACHE_TTL: Duration = Duration::from_secs(5);

/// 订阅信息（client → subscription → plan 配额）
#[derive(Clone, Debug)]
pub struct SubscriptionInfo {
    pub subscription_id: i64,
    pub plan_code: String,
    pub rate_limit_rps: f64,
    pub burst: i32,
    pub quota_daily: i64,
    pub quota_monthly: i64,
    pub tier: i16,
}

/// 订阅解析缓存条目
struct SubscriptionCacheEntry {
    info: SubscriptionInfo,
    cached_at: Instant,
}

/// 配额检查缓存条目
struct QuotaCacheEntry {
    passed: bool,
    cached_at: Instant,
}

/// 计量写入队列（批量 INSERT）
struct MeteringQueue {
    tx: std::sync::mpsc::SyncSender<UsageRecord>,
}

/// 单条计量记录
#[derive(Clone, Debug)]
struct UsageRecord {
    subscription_id: i64,
    route: String,
    method: String,
    status: u16,
    latency_ms: i64,
}

/// API Usage 计量中间件。
///
/// 挂载在 PEP 内层（/api scope），仅处理经 PEP 认证的请求。
pub struct ApiUsageMiddleware {
    pool: PgPool,
    /// client_id → SubscriptionInfo 缓存
    subscriptions: Arc<RwLock<HashMap<String, SubscriptionCacheEntry>>>,
    /// (client_id, window) → 配额检查结果缓存
    quota_cache: Arc<RwLock<HashMap<(String, String), QuotaCacheEntry>>>,
    /// client_id → 令牌桶（per-plan 限流：容量/补率来自订阅 plan）
    rate_buckets: Arc<RwLock<HashMap<String, RateBucket>>>,
    /// 计量写入队列
    queue: Arc<MeteringQueue>,
}

/// 令牌桶（per-client，容量/补率来自 plan）
#[derive(Clone)]
struct RateBucket {
    tokens: f64,
    capacity: f64,
    refill_rate: f64,
    last_refill: Instant,
}

impl RateBucket {
    fn new(capacity: f64, refill_rate: f64) -> Self {
        Self {
            tokens: capacity,
            capacity,
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    fn try_consume(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

impl ApiUsageMiddleware {
    pub fn new(pool: PgPool) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel(METERING_QUEUE_CAPACITY);
        let queue = Arc::new(MeteringQueue { tx });
        // 计量 worker：批量写入，失败仅记日志（计量不影响主请求路径）
        spawn_metering_worker(pool.clone(), rx);
        Self {
            pool,
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            quota_cache: Arc::new(RwLock::new(HashMap::new())),
            rate_buckets: Arc::new(RwLock::new(HashMap::new())),
            queue,
        }
    }

    /// per-plan 限流检查（Gap B 修复）：按 client 令牌桶，容量/补率来自订阅 plan。
    /// 返回剩余令牌数（供 X-RateLimit-Remaining 头）；超限返回 None。
    fn check_rate_limit(&self, client_id: &str, info: &SubscriptionInfo) -> Option<u64> {
        let mut buckets = self.rate_buckets.write().unwrap();
        let bucket = buckets.entry(client_id.to_string()).or_insert_with(|| {
            RateBucket::new(info.burst.max(1) as f64, info.rate_limit_rps.max(0.1))
        });
        if bucket.try_consume() {
            Some(bucket.tokens.floor() as u64)
        } else {
            None
        }
    }

    /// 解析订阅信息（缓存 60s）。
    async fn resolve_subscription(&self, client_id: &str) -> Option<SubscriptionInfo> {
        // 缓存命中
        {
            let cache = self.subscriptions.read().unwrap();
            if let Some(entry) = cache.get(client_id) {
                if entry.cached_at.elapsed() < SUBSCRIPTION_CACHE_TTL {
                    return Some(entry.info.clone());
                }
            }
        }
        // 缓存未命中 → 查库
        let row: Option<(i64, String, String, i32, i64, i64, i16)> = sqlx::query_as(
            r#"
            SELECT s.id, p.code, p.rate_limit_rps::text, p.burst, p.quota_daily, p.quota_monthly, p.tier
            FROM isahl_auth.api_clients c
            JOIN isahl_auth.api_subscriptions s
              ON s.fk_client = c.id AND s.deleted_at IS NULL
              AND s.status = 'active'
              AND (s.expires_at IS NULL OR s.expires_at > NOW())
            JOIN isahl_auth.api_plans p ON p.id = s.fk_plan AND p.deleted_at IS NULL
            WHERE c.client_id = $1 AND c.deleted_at IS NULL
            ORDER BY s.id DESC
            LIMIT 1
            "#,
        )
        .bind(client_id)
        .fetch_optional(&self.pool)
        .await
        .unwrap_or(None);

        let (
            subscription_id,
            plan_code,
            rate_limit_rps_str,
            burst,
            quota_daily,
            quota_monthly,
            tier,
        ) = match row {
            Some(r) => r,
            None => return None,
        };
        let rate_limit_rps = rate_limit_rps_str.parse::<f64>().unwrap_or(1.0);
        let info = SubscriptionInfo {
            subscription_id,
            plan_code,
            rate_limit_rps,
            burst,
            quota_daily,
            quota_monthly,
            tier,
        };
        let mut cache = self.subscriptions.write().unwrap();
        cache.insert(
            client_id.to_string(),
            SubscriptionCacheEntry {
                info: info.clone(),
                cached_at: Instant::now(),
            },
        );
        Some(info)
    }

    /// 配额检查（日/月窗口 COUNT，5s 缓存）。
    async fn check_quota(&self, client_id: &str, info: &SubscriptionInfo) -> bool {
        if info.quota_daily <= 0 && info.quota_monthly <= 0 {
            return true; // 不限配额
        }
        let day_key = format!("{}:d", client_id);
        let month_key = format!("{}:m", client_id);

        // 缓存命中（日）
        {
            let cache = self.quota_cache.read().unwrap();
            if let Some(e) = cache.get(&(day_key.clone(), "d".into())) {
                if e.cached_at.elapsed() < QUOTA_CHECK_CACHE_TTL && !e.passed {
                    return false;
                }
            }
            if let Some(e) = cache.get(&(month_key.clone(), "m".into())) {
                if e.cached_at.elapsed() < QUOTA_CHECK_CACHE_TTL && !e.passed {
                    return false;
                }
            }
        }

        let mut ok = true;
        if info.quota_daily > 0 {
            let daily: i64 = sqlx::query_scalar(
                r#"SELECT COUNT(*) FROM isahl_auth.api_usage
                   WHERE fk_subscription = $1 AND requested_at >= date_trunc('day', NOW())"#,
            )
            .bind(info.subscription_id)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
            if daily >= info.quota_daily {
                ok = false;
            }
        }
        if ok && info.quota_monthly > 0 {
            let monthly: i64 = sqlx::query_scalar(
                r#"SELECT COUNT(*) FROM isahl_auth.api_usage
                   WHERE fk_subscription = $1 AND requested_at >= date_trunc('month', NOW())"#,
            )
            .bind(info.subscription_id)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
            if monthly >= info.quota_monthly {
                ok = false;
            }
        }

        let mut cache = self.quota_cache.write().unwrap();
        cache.insert(
            (day_key, "d".into()),
            QuotaCacheEntry {
                passed: ok,
                cached_at: Instant::now(),
            },
        );
        cache.insert(
            (month_key, "m".into()),
            QuotaCacheEntry {
                passed: ok,
                cached_at: Instant::now(),
            },
        );
        ok
    }

    /// 从请求解析服务令牌 client_id（不验签 —— PEP 已验）。
    /// pub(crate)：openapi::idempotency 复用同一 JWT claims 解析（第三方写请求幂等面，
    /// 对齐「自然人不计量」边界）。
    pub(crate) fn service_client_id(req: &ServiceRequest) -> Option<String> {
        let auth = req
            .headers()
            .get(actix_web::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let token = auth.strip_prefix("Bearer ")?;
        let payload = token.split('.').nth(1)?;
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload.as_bytes())
            .ok()?;
        let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
        // SSO 签发 svc_user_id 为字符串（zuid 惯例，对齐 PEP Claims 的
        // common::serde_zuid 反序列化）；纯 as_i64 会把全部服务令牌误判为自然人，
        // 计量/幂等静默透传（change: add-wz-yy-fssc-callback-proxy 实测发现）。
        let svc = claims
            .get("svc_user_id")?
            .as_i64()
            .or_else(|| claims.get("svc_user_id")?.as_str()?.parse::<i64>().ok())?;
        if svc <= 0 {
            return None; // 自然人令牌不计量
        }
        claims.get("sub")?.as_str().map(|s| {
            // sub 格式为 `client:<client_id>`；剥离前缀得到 api_clients.client_id
            s.strip_prefix("client:")
                .map(|c| c.to_string())
                .unwrap_or_else(|| s.to_string())
        })
    }

    /// 入队计量记录（满则丢弃，不阻塞请求）。
    fn enqueue(&self, record: UsageRecord) {
        if let Err(e) = self.queue.tx.try_send(record) {
            log::warn!("api_usage queue full, dropping metering record: {}", e);
        }
    }
}

impl Clone for ApiUsageMiddleware {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            subscriptions: self.subscriptions.clone(),
            quota_cache: self.quota_cache.clone(),
            rate_buckets: self.rate_buckets.clone(),
            queue: self.queue.clone(),
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for ApiUsageMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = ApiUsageMiddlewareService<S>;
    type Future = std::future::Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        std::future::ready(Ok(ApiUsageMiddlewareService {
            service: Rc::new(service),
            inner: self.clone(),
        }))
    }
}

pub struct ApiUsageMiddlewareService<S> {
    service: Rc<S>,
    inner: ApiUsageMiddleware,
}

impl<S, B> Service<ServiceRequest> for ApiUsageMiddlewareService<S>
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
        let svc = self.service.clone();
        let inner = self.inner.clone();
        Box::pin(async move {
            let start = Instant::now();
            let path = req.path().to_string();
            let method = req.method().as_str().to_string();

            // 服务令牌识别（不验签；PEP 已验）
            let client_id = ApiUsageMiddleware::service_client_id(&req);

            // 订阅状态强制（Gap A 修复，fail-closed）：
            // 服务令牌必须命中有效订阅（status='active' 且未过期），
            // 否则拒绝访问——订阅暂停/过期/不存在时 API 不可用。
            // 解析失败（DB 错误）同样拒绝（安全优先，宁拒勿放）。
            let subscription = if let Some(cid) = &client_id {
                match inner.resolve_subscription(cid).await {
                    Some(info) => Some(info),
                    None => {
                        return Ok(req
                            .into_response(HttpResponse::Unauthorized().json(json!({
                                "error": "SUBSCRIPTION_INACTIVE",
                                "message": "No active subscription for this client",
                            })))
                            .map_into_right_body());
                    }
                }
            } else {
                None
            };

            // 配额检查 + per-plan 限流（仅服务令牌；超限 → 429 不进入 handler）
            if let Some(info) = &subscription {
                if let Some(cid) = &client_id {
                    // per-plan 限流（Gap B）：容量/补率来自订阅 plan
                    let remaining = inner.check_rate_limit(cid, info);
                    if remaining.is_none() {
                        let retry_after =
                            (info.burst.max(1) as f64 / info.rate_limit_rps.max(0.1)).ceil() as u64;
                        return Ok(req
                            .into_response(
                                HttpResponse::TooManyRequests()
                                    .insert_header(("Retry-After", retry_after.to_string()))
                                    .insert_header(("X-RateLimit-Limit", info.burst.to_string()))
                                    .insert_header(("X-RateLimit-Remaining", "0"))
                                    .insert_header((
                                        "X-RateLimit-Reset",
                                        (chrono::Utc::now().timestamp() as u64 + retry_after)
                                            .to_string(),
                                    ))
                                    .json(json!({
                                        "error": "RATE_LIMITED",
                                        "message": "Per-client rate limit exceeded",
                                    })),
                            )
                            .map_into_right_body());
                    }
                    // 配额检查（Gap A 已保证订阅有效）
                    if !inner.check_quota(cid, info).await {
                        return Ok(req
                            .into_response(
                                HttpResponse::TooManyRequests()
                                    .insert_header(("Retry-After", "3600"))
                                    .json(json!({
                                        "error": "QUOTA_EXCEEDED",
                                        "message": "Daily or monthly quota exceeded",
                                    })),
                            )
                            .map_into_right_body());
                    }
                }
            }

            let res = svc.call(req).await;
            let status = res.as_ref().map(|r| r.status().as_u16()).unwrap_or(500);
            let latency_ms = start.elapsed().as_millis() as i64;

            // 计量写入（仅服务令牌，且已确认有订阅）
            if let (Some(_cid), Some(info)) = (&client_id, &subscription) {
                inner.enqueue(UsageRecord {
                    subscription_id: info.subscription_id,
                    route: path,
                    method,
                    status,
                    latency_ms,
                });
            }

            res.map(|r| r.map_into_left_body())
        })
    }
}

/// 计量 worker：批量消费队列，批量 INSERT + 周期归档（Gap G）。
fn spawn_metering_worker(pool: PgPool, rx: std::sync::mpsc::Receiver<UsageRecord>) {
    std::thread::spawn(move || {
        // 归档计时：每 6 小时删 90 天前 api_usage（幂等）
        let mut last_archive = Instant::now();
        const ARCHIVE_INTERVAL: Duration = Duration::from_secs(6 * 3600);
        const RETENTION_DAYS: i64 = 90;

        loop {
            // 收集一批（最多 200 条或 100ms 超时）
            let mut batch: Vec<UsageRecord> = Vec::new();
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(r) => batch.push(r),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
            while batch.len() < 200 {
                match rx.try_recv() {
                    Ok(r) => batch.push(r),
                    Err(_) => break,
                }
            }

            // 周期归档（即使无新计量也执行；清理 90 天前流水）
            if last_archive.elapsed() >= ARCHIVE_INTERVAL {
                last_archive = Instant::now();
                let pool_clone = pool.clone();
                std::thread::spawn(move || {
                    let rt = match tokio::runtime::Runtime::new() {
                        Ok(r) => r,
                        Err(e) => {
                            log::error!("api_usage archive runtime error: {}", e);
                            return;
                        }
                    };
                    rt.block_on(async move {
                        match sqlx::query(
                            "DELETE FROM isahl_auth.api_usage \
                             WHERE requested_at < NOW() - ($1 * INTERVAL '1 day')",
                        )
                        .bind(RETENTION_DAYS)
                        .execute(&pool_clone)
                        .await
                        {
                            Ok(r) => log::info!(
                                "api_usage retention: purged {} rows older than {} days",
                                r.rows_affected(),
                                RETENTION_DAYS
                            ),
                            Err(e) => log::error!("api_usage retention purge error: {}", e),
                        }
                    });
                });
            }

            if batch.is_empty() {
                continue;
            }

            // 批量 INSERT（独立 tokio runtime，不依赖请求线程）
            let pool_clone = pool.clone();
            std::thread::spawn(move || {
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(r) => r,
                    Err(e) => {
                        log::error!("api_usage worker runtime error: {}", e);
                        return;
                    }
                };
                rt.block_on(async move {
                    let mut tx = match pool_clone.begin().await {
                        Ok(t) => t,
                        Err(e) => {
                            log::error!("api_usage tx begin error: {}", e);
                            return;
                        }
                    };
                    for r in &batch {
                        let res = sqlx::query(
                            r#"INSERT INTO isahl_auth.api_usage
                               (fk_subscription, route, method, status, latency_ms, requested_at)
                               VALUES ($1, $2, $3, $4, $5, NOW())"#,
                        )
                        .bind(r.subscription_id)
                        .bind(&r.route)
                        .bind(&r.method)
                        .bind(r.status as i16)
                        .bind(r.latency_ms)
                        .execute(&mut *tx)
                        .await;
                        if let Err(e) = res {
                            log::error!("api_usage insert error: {}", e);
                        }
                    }
                    if let Err(e) = tx.commit().await {
                        log::error!("api_usage tx commit error: {}", e);
                    }
                });
            });
        }
    });
}
