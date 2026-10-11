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
    http::header::{HeaderName, HeaderValue},
    Error, HttpResponse,
};
use base64::Engine;
use futures::future::LocalBoxFuture;
use serde_json::json;
use sqlx::PgPool;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// 订阅解析缓存 TTL（60s —— 订阅变更低频，容忍 1 分钟延迟）
const SUBSCRIPTION_CACHE_TTL: Duration = Duration::from_secs(60);
/// 计量写入队列容量（请求侧不阻塞：满则丢弃**并记账**——MUST NOT 静默丢弃）
const METERING_QUEUE_CAPACITY: usize = 4096;
/// 配额检查缓存 TTL（5s —— 窗口计数低频变化，容忍短延迟）
const QUOTA_CHECK_CACHE_TTL: Duration = Duration::from_secs(5);
/// 空闲令牌桶回收阈值（超过该时长未使用的桶在下一次清扫时回收——桶表 MUST 有界）
const BUCKET_IDLE_TTL: Duration = Duration::from_secs(30 * 60);
/// 桶清扫触发步长（每 N 次限流检查触发一次空闲淘汰；避免每请求全表扫描）
const BUCKET_SWEEP_EVERY: u64 = 256;
/// 计量丢弃告警步长（首次 + 每累计 N 条输出一次 warn）
const METERING_DROP_WARN_EVERY: u64 = 100;

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

/// 计量写入队列（批量 INSERT）；`dropped` = 排队满导致的累计丢弃数（MUST NOT 静默）
struct MeteringQueue {
    tx: std::sync::mpsc::SyncSender<UsageRecord>,
    dropped: Arc<AtomicU64>,
}

/// 单条计量记录
#[derive(Clone, Debug)]
struct UsageRecord {
    subscription_id: i64,
    route: String,
    method: String,
    status: u16,
    latency_ms: i64,
    /// 来源 IP（无法确定/非法 ⇒ `None` → NULL；MUST NOT 伪造）
    client_ip: Option<String>,
}

/// API Usage 计量中间件。
///
/// 装配单源 = `openapi::governance`（执行序 Metrics → PEP → **metering** → idempotency；
/// 按 actix「后注册者先执行」以逆序登记）——仅处理经 PEP 认证的请求；历史倒置缺陷
/// 与修复见 change `fix-openapi-review-findings` §1。
pub struct ApiUsageMiddleware {
    pool: PgPool,
    /// client_id → SubscriptionInfo 缓存
    subscriptions: Arc<RwLock<HashMap<String, SubscriptionCacheEntry>>>,
    /// (client_id, window) → 配额检查结果缓存
    quota_cache: Arc<RwLock<HashMap<(String, String), QuotaCacheEntry>>>,
    /// client_id → 令牌桶（per-plan 限流：容量/补率来自订阅 plan）
    rate_buckets: Arc<RwLock<HashMap<String, RateBucket>>>,
    /// 限流检查计数（驱动空闲桶清扫节奏）
    bucket_ops: Arc<AtomicU64>,
    /// 计量写入队列
    queue: Arc<MeteringQueue>,
}

/// 令牌桶（per-client，容量/补率来自 plan）。
///
/// **plan 新鲜度**：桶记录建桶时的 plan 参数，检查时与当前订阅 plan 比对，不一致即**重建**
/// （套餐升降级无需进程重启即生效）。桶表由 `maybe_sweep_buckets` 空闲淘汰保持有界。
#[derive(Clone)]
struct RateBucket {
    tokens: f64,
    capacity: f64,
    refill_rate: f64,
    last_refill: Instant,
    /// 建桶时的 plan burst（与当前订阅 plan 比对用）
    plan_burst: i64,
    /// 建桶时的 plan RPS（毫秒粒度比较，避免 f64 直比）
    plan_rps_millis: i64,
}

/// 限流检查结果：**放行与 429 共用同一取值口径**（`X-RateLimit-*` 三头 MUST 同源）。
struct RateLimitOutcome {
    limit: i64,
    remaining: u64,
    /// 桶重置的 Unix 秒（429 ⇒ 距补出一个令牌的时刻；放行 ⇒ 当前时刻）
    reset_at: i64,
}

/// 写 `X-RateLimit-Limit/Remaining/Reset` 三头（放行与 429 共用；MUST NOT 各写一套公式）。
fn insert_rate_limit_headers(
    headers: &mut actix_web::http::header::HeaderMap,
    outcome: &RateLimitOutcome,
) {
    for (name, value) in [
        ("x-ratelimit-limit", outcome.limit.to_string()),
        ("x-ratelimit-remaining", outcome.remaining.to_string()),
        ("x-ratelimit-reset", outcome.reset_at.to_string()),
    ] {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_lowercase(name.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            headers.insert(name, value);
        }
    }
}

impl RateBucket {
    fn new(info: &SubscriptionInfo) -> Self {
        let capacity = info.burst.max(1) as f64;
        let refill_rate = info.rate_limit_rps.max(0.1);
        Self {
            tokens: capacity,
            capacity,
            refill_rate,
            last_refill: Instant::now(),
            plan_burst: info.burst.max(1) as i64,
            plan_rps_millis: (refill_rate * 1000.0).round() as i64,
        }
    }

    /// 与当前订阅 plan 参数一致？（不一致 ⇒ 调用方重建该桶）
    fn matches_plan(&self, info: &SubscriptionInfo) -> bool {
        self.plan_burst == info.burst.max(1) as i64
            && self.plan_rps_millis == (info.rate_limit_rps.max(0.1) * 1000.0).round() as i64
    }

    /// 消费一个令牌：返回消费后剩余令牌数（不足 ⇒ `None`，不消费）。
    fn try_consume(&mut self) -> Option<u64> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Some(self.tokens.floor() as u64)
        } else {
            None
        }
    }
}

/// 桶检查（**纯函数**：只依赖桶表 + 订阅信息 ⇒ 单测与生产同源）：
/// plan 参数与建桶时不一致即**重建**（套餐升降级即时生效——受订阅解析缓存 TTL 约束）。
fn rate_limit_check(
    buckets: &mut HashMap<String, RateBucket>,
    client_id: &str,
    info: &SubscriptionInfo,
    now_unix: i64,
) -> (RateLimitOutcome, bool) {
    let limit = info.burst.max(1) as i64;
    let retry_after = (limit as f64 / info.rate_limit_rps.max(0.1)).ceil() as i64;
    let bucket = buckets
        .entry(client_id.to_string())
        .and_modify(|b| {
            if !b.matches_plan(info) {
                // 套餐变更（升/降级）⇒ 以新 plan 参数重建
                *b = RateBucket::new(info);
            }
        })
        .or_insert_with(|| RateBucket::new(info));
    match bucket.try_consume() {
        Some(remaining) => (
            RateLimitOutcome {
                limit,
                remaining,
                reset_at: now_unix,
            },
            true,
        ),
        None => (
            RateLimitOutcome {
                limit,
                remaining: 0,
                reset_at: now_unix + retry_after,
            },
            false,
        ),
    }
}

/// 距下一个 UTC 日 0 点的秒数（≥1）。
fn seconds_until_next_utc_day() -> i64 {
    let now = chrono::Utc::now();
    let tomorrow = (now.date_naive() + chrono::Days::new(1))
        .and_hms_opt(0, 0, 0)
        .expect("次日 0 点恒可构造");
    let target = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(tomorrow, chrono::Utc);
    (target - now).num_seconds().max(1)
}

/// 距下一个月 1 日 0 点的秒数（≥1）。
fn seconds_until_next_utc_month() -> i64 {
    use chrono::Datelike;
    let now = chrono::Utc::now();
    let (year, month) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1)
        .expect("次月 1 日恒可构造")
        .and_hms_opt(0, 0, 0)
        .expect("0 点恒可构造");
    let target = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(first, chrono::Utc);
    (target - now).num_seconds().max(1)
}

/// 配额超限的 `Retry-After`（秒）= 距**判定窗口**重置：日 → 次日 0 点；月 → 次月 1 日 0 点；
/// 两者皆超取更晚者。MUST NOT 返回固定常量（月度配额下 1 小时是错误承诺）。
fn quota_retry_after(day_exceeded: bool, month_exceeded: bool) -> i64 {
    let day = if day_exceeded {
        seconds_until_next_utc_day()
    } else {
        0
    };
    let month = if month_exceeded {
        seconds_until_next_utc_month()
    } else {
        0
    };
    day.max(month).max(1)
}

impl ApiUsageMiddleware {
    pub fn new(pool: PgPool) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel(METERING_QUEUE_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let queue = Arc::new(MeteringQueue {
            tx,
            dropped: dropped.clone(),
        });
        // 计量 worker：批量写入，失败仅记日志（计量不影响主请求路径）
        spawn_metering_worker(pool.clone(), rx, dropped);
        Self {
            pool,
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            quota_cache: Arc::new(RwLock::new(HashMap::new())),
            rate_buckets: Arc::new(RwLock::new(HashMap::new())),
            bucket_ops: Arc::new(AtomicU64::new(0)),
            queue,
        }
    }

    /// per-plan 限流检查（Gap B 修复）：按 client 令牌桶，容量/补率来自**当前**订阅 plan
    /// （plan 变更即重建桶 ⇒ 升降级无需重启生效）。
    /// 返回 `(outcome, allowed)`：`outcome` 同时供放行响应头与 429 响应头（同源取值）。
    fn check_rate_limit(
        &self,
        client_id: &str,
        info: &SubscriptionInfo,
    ) -> (RateLimitOutcome, bool) {
        let now_unix = chrono::Utc::now().timestamp();
        let result = {
            let mut buckets = self.rate_buckets.write().unwrap();
            rate_limit_check(&mut buckets, client_id, info, now_unix)
        };
        self.maybe_sweep_buckets();
        result
    }

    /// 空闲桶淘汰（每 `BUCKET_SWEEP_EVERY` 次检查触发一次）——桶表 MUST 有界
    /// （client 删除/长期静默后其桶最迟 `BUCKET_IDLE_TTL` 被回收）。
    fn maybe_sweep_buckets(&self) {
        if !self
            .bucket_ops
            .fetch_add(1, Ordering::Relaxed)
            .is_multiple_of(BUCKET_SWEEP_EVERY)
        {
            return;
        }
        let mut buckets = self.rate_buckets.write().unwrap();
        let before = buckets.len();
        buckets.retain(|_, b| b.last_refill.elapsed() < BUCKET_IDLE_TTL);
        let removed = before - buckets.len();
        if removed > 0 {
            log::debug!("metering: evicted {} idle rate-limit bucket(s)", removed);
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

    /// 配额检查（日/月窗口 COUNT，5s 缓存）：`None` = 放行；`Some(retry_after)` = 超限，
    /// 值为**距判定窗口重置**的秒数（日 → 次日 0 点；月 → 次月 1 日 0 点；两者皆超取更晚者）。
    async fn check_quota(&self, client_id: &str, info: &SubscriptionInfo) -> Option<i64> {
        if info.quota_daily <= 0 && info.quota_monthly <= 0 {
            return None; // 不限配额
        }
        let day_key = format!("{}:d", client_id);
        let month_key = format!("{}:m", client_id);

        // 缓存命中：仅「超限」结论可复用（放行结论不缓存 ⇒ 窗口开闸不被延迟 5s）
        {
            let cache = self.quota_cache.read().unwrap();
            let day_exceeded = cache
                .get(&(day_key.clone(), "d".into()))
                .map(|e| e.cached_at.elapsed() < QUOTA_CHECK_CACHE_TTL && !e.passed)
                .unwrap_or(false);
            let month_exceeded = cache
                .get(&(month_key.clone(), "m".into()))
                .map(|e| e.cached_at.elapsed() < QUOTA_CHECK_CACHE_TTL && !e.passed)
                .unwrap_or(false);
            if day_exceeded || month_exceeded {
                return Some(quota_retry_after(day_exceeded, month_exceeded));
            }
        }

        let mut day_exceeded = false;
        let mut month_exceeded = false;
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
                day_exceeded = true;
            }
        }
        if info.quota_monthly > 0 {
            let monthly: i64 = sqlx::query_scalar(
                r#"SELECT COUNT(*) FROM isahl_auth.api_usage
                   WHERE fk_subscription = $1 AND requested_at >= date_trunc('month', NOW())"#,
            )
            .bind(info.subscription_id)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
            if monthly >= info.quota_monthly {
                month_exceeded = true;
            }
        }
        let exceeded = day_exceeded || month_exceeded;

        let mut cache = self.quota_cache.write().unwrap();
        cache.insert(
            (day_key, "d".into()),
            QuotaCacheEntry {
                passed: !day_exceeded,
                cached_at: Instant::now(),
            },
        );
        cache.insert(
            (month_key, "m".into()),
            QuotaCacheEntry {
                passed: !month_exceeded,
                cached_at: Instant::now(),
            },
        );
        if !exceeded {
            return None;
        }
        Some(quota_retry_after(day_exceeded, month_exceeded))
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
            // 丢弃 MUST NOT 静默：首次 + 每累计 N 条告警（累计数可供运维对账缺口）
            let dropped = self.queue.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if dropped == 1 || dropped.is_multiple_of(METERING_DROP_WARN_EVERY) {
                log::warn!(
                    "api_usage queue full: dropped {} metering record(s) so far ({})",
                    dropped,
                    e
                );
            }
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
            bucket_ops: self.bucket_ops.clone(),
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
            let mut rate_headers: Option<RateLimitOutcome> = None;
            if let Some(info) = &subscription {
                if let Some(cid) = &client_id {
                    // per-plan 限流：容量/补率来自**当前**订阅 plan（变更即重建桶）
                    let (outcome, allowed) = inner.check_rate_limit(cid, info);
                    if !allowed {
                        let retry_after =
                            (outcome.reset_at - chrono::Utc::now().timestamp()).max(0);
                        return Ok(req
                            .into_response(
                                HttpResponse::TooManyRequests()
                                    .insert_header(("Retry-After", retry_after.to_string()))
                                    .insert_header(("X-RateLimit-Limit", outcome.limit.to_string()))
                                    .insert_header(("X-RateLimit-Remaining", "0"))
                                    .insert_header((
                                        "X-RateLimit-Reset",
                                        outcome.reset_at.to_string(),
                                    ))
                                    .json(json!({
                                        "error": "RATE_LIMITED",
                                        "message": "Per-client rate limit exceeded",
                                    })),
                            )
                            .map_into_right_body());
                    }
                    // 配额检查（Gap A 已保证订阅有效）：`Retry-After` 按判定窗口重置时刻
                    if let Some(retry_after) = inner.check_quota(cid, info).await {
                        return Ok(req
                            .into_response(
                                HttpResponse::TooManyRequests()
                                    .insert_header(("Retry-After", retry_after.to_string()))
                                    .json(json!({
                                        "error": "QUOTA_EXCEEDED",
                                        "message": "Daily or monthly quota exceeded",
                                    })),
                            )
                            .map_into_right_body());
                    }
                    // 放行 ⇒ 成功响应同样携带限流头（第三方可自适应；与 429 同源取值）
                    rate_headers = Some(outcome);
                }
            }

            // 来源 IP：actix `ConnectionInfo::realip_remote_addr`（框架口径——存在
            // `X-Forwarded-For`/`Forwarded` 时取其首值，否则回退对端地址；信任链归部署面
            // 代理配置）。仅作**审计**列，MUST NOT 用作授权/限流判定输入。
            // 无法确定或非法 ⇒ NULL（MUST NOT 伪造）。
            let client_ip = req
                .connection_info()
                .realip_remote_addr()
                .and_then(|s| s.parse::<std::net::IpAddr>().ok())
                .map(|ip| ip.to_string());

            let mut res = svc.call(req).await;
            if let (Some(outcome), Ok(response)) = (rate_headers.as_ref(), res.as_mut()) {
                insert_rate_limit_headers(response.headers_mut(), outcome);
            }
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
                    client_ip,
                });
            }

            res.map(|r| r.map_into_left_body())
        })
    }
}

/// 计量 worker：批量消费队列，批量 INSERT + 周期归档（Gap G）。
fn spawn_metering_worker(
    pool: PgPool,
    rx: std::sync::mpsc::Receiver<UsageRecord>,
    dropped: Arc<AtomicU64>,
) {
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
                let dropped_clone = dropped.clone();
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
                                "api_usage retention: purged {} rows older than {} days \
                                 (metering records dropped so far: {})",
                                r.rows_affected(),
                                RETENTION_DAYS,
                                dropped_clone.load(Ordering::Relaxed)
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
                               (fk_subscription, route, method, status, latency_ms, client_ip, requested_at)
                               VALUES ($1, $2, $3, $4, $5, $6::inet, NOW())"#,
                        )
                        .bind(r.subscription_id)
                        .bind(&r.route)
                        .bind(&r.method)
                        .bind(r.status as i16)
                        .bind(r.latency_ms)
                        .bind(r.client_ip.as_deref())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn info(
        code: &str,
        burst: i32,
        rps: f64,
        quota_daily: i64,
        quota_monthly: i64,
    ) -> SubscriptionInfo {
        SubscriptionInfo {
            subscription_id: 1,
            plan_code: code.to_string(),
            rate_limit_rps: rps,
            burst,
            quota_daily,
            quota_monthly,
            tier: 0,
        }
    }

    /// 配额等待时间按窗口：仅日超限 ⇒ ≤ 24h；仅月超限 ⇒ ≥ 24h 且 ≤ 31 天。
    #[test]
    fn quota_retry_after_follows_window() {
        let day_only = quota_retry_after(true, false);
        assert!((1..=24 * 3600).contains(&day_only), "day: {day_only}");

        let month_only = quota_retry_after(false, true);
        assert!(
            (24 * 3600..=31 * 24 * 3600).contains(&month_only),
            "month: {month_only}"
        );

        // 两者皆超限 ⇒ 取更晚者
        assert_eq!(quota_retry_after(true, true), day_only.max(month_only));
    }

    /// UTC 窗口边界有界（≥1s、日 ≤24h、月 ≤31 天）。
    #[test]
    fn utc_window_boundaries_are_bounded() {
        assert!((1..=24 * 3600).contains(&seconds_until_next_utc_day()));
        assert!((1..=31 * 24 * 3600).contains(&seconds_until_next_utc_month()));
    }

    /// 桶随 plan 变更重建：同 client 按 burst=1 耗尽后，改判 burst=10 ⇒ MUST 立即放行
    /// （套餐升降级无需进程重启；订阅信息缓存 TTL 之外即时可见）。
    #[test]
    fn rate_limit_check_rebuilds_bucket_on_plan_change() {
        let mut buckets: HashMap<String, RateBucket> = HashMap::new();
        let free = info("free", 1, 0.0001, 0, 0);

        let (first, allowed) = rate_limit_check(&mut buckets, "c1", &free, 1_000);
        assert!(allowed && first.limit == 1 && first.remaining == 0);

        let (second, allowed) = rate_limit_check(&mut buckets, "c1", &free, 1_000);
        assert!(!allowed, "burst=1 已耗尽 ⇒ 拒绝");
        assert_eq!(second.remaining, 0);
        assert!(second.reset_at > 1_000, "429 的 reset 必须在未来");

        let pro = info("pro", 10, 5.0, 0, 0);
        let (third, allowed) = rate_limit_check(&mut buckets, "c1", &pro, 1_001);
        assert!(allowed, "套餐变更 MUST 即时生效（桶按新 plan 重建）");
        assert_eq!(third.limit, 10);
        assert_eq!(third.remaining, 9);
    }

    /// `matches_plan` 检测容量与补率两类变更。
    #[test]
    fn bucket_plan_freshness_detects_change() {
        let free = info("free", 5, 1.0, 0, 0);
        let bucket = RateBucket::new(&free);
        assert!(bucket.matches_plan(&free));
        assert!(!bucket.matches_plan(&info("pro", 100, 50.0, 0, 0)));
        assert!(!bucket.matches_plan(&info("free2", 5, 2.0, 0, 0)));
    }

    /// 桶消费语义：容量内返回剩余；耗尽返回 None（不消费）。
    #[test]
    fn bucket_consumes_until_exhausted() {
        let mut bucket = RateBucket::new(&info("tiny", 2, 0.0001, 0, 0));
        assert_eq!(bucket.try_consume(), Some(1));
        assert_eq!(bucket.try_consume(), Some(0));
        assert_eq!(bucket.try_consume(), None);
    }
}
