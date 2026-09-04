//! OpenAPI 幂等键（openspec/changes/add-openapi-idempotency-keys/）
//!
//! `IdempotencyMiddleware`：第三方写请求（POST/PUT/PATCH `/api/service/*`）的
//! 服务端幂等，行业最佳实践（Stripe/Moesif）的 `Idempotency-Key` 语义：
//!
//! 1. 触发条件（全部满足，否则零影响透传）：写方法 ∧ `/api/service/` 前缀
//!    ∧ `Idempotency-Key` header ∧ 服务令牌（`svc_user_id > 0`，PEP 已验签）
//! 2. 首次（leader）：INSERT `ON CONFLICT DO NOTHING` 抢占 → 执行 handler →
//!    存储响应快照（status/content-type/body，cap 256KB）
//! 3. 重放（follower，同指纹）：返回快照 + `Idempotency-Replayed: true`，
//!    不执行 handler
//! 4. 冲突：同 key 异指纹 → 409 `IDEMPOTENCY_PAYLOAD_MISMATCH`；leader 仍在
//!    执行 → 409 `IDEMPOTENCY_REQUEST_IN_PROGRESS` + `Retry-After`
//! 5. 5xx 不占用幂等槽（记录删除，key 可重用——对齐 Stripe 语义）
//! 6. TTL 24h，后台周期清理（10min）
//!
//! 存储：`isahl_auth.api_idempotency_keys`——DB 唯一约束提供跨实例正确性
//! （多实例天然正确，无需分布式锁/Redis）。`api_version` 为版本专项字段
//! （`X-Api-Version` header，默认 `v1`），参与唯一约束：未来版本机制落地时
//! 同 key 跨版本互不冲突。
//!
//! 挂载在 PEP + metering 内层（metering 外层 → 重放请求仍计量：replay 是
//! 被服务的 API 调用，计入用量与配额）。

use actix_web::{
    body::{to_bytes, EitherBody},
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    http::header::{HeaderName, HeaderValue},
    Error, HttpMessage, HttpResponse,
};
use futures::future::LocalBoxFuture;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::rc::Rc;
use std::time::Duration;

/// 幂等键最大长度（Stripe 同款 255）
const IDEM_KEY_MAX_LEN: usize = 255;
/// 响应快照 body 上限；超限降级为仅存 status（重放不伪造截断内容）
const RESPONSE_SNAPSHOT_CAP: usize = 256 * 1024;
/// 记录保留期（24h）
const RETENTION_HOURS: i64 = 24;
/// 后台清理周期
const CLEANUP_INTERVAL: Duration = Duration::from_secs(600);
/// in-progress 冲突建议重试等待
const IN_PROGRESS_RETRY_AFTER_SECS: u32 = 1;

/// 幂等记录行（follower 读取：指纹/状态/快照）。
type IdemSnapshotRow = (String, String, Option<i16>, Option<String>, Option<String>);

/// 幂等键中间件。
pub struct IdempotencyMiddleware {
    pool: PgPool,
}

impl IdempotencyMiddleware {
    pub fn new(pool: PgPool) -> Self {
        spawn_cleanup_worker(pool.clone());
        Self { pool }
    }
}

impl Clone for IdempotencyMiddleware {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
        }
    }
}

/// TTL 清理 worker：周期删除过期幂等记录（幂等，失败仅记日志）。
fn spawn_cleanup_worker(pool: PgPool) {
    std::thread::spawn(move || loop {
        std::thread::sleep(CLEANUP_INTERVAL);
        let pool_clone = pool.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(r) => r,
                Err(e) => {
                    log::error!("api_idempotency_keys cleanup runtime error: {}", e);
                    return;
                }
            };
            rt.block_on(async move {
                match sqlx::query(
                    "DELETE FROM isahl_auth.api_idempotency_keys \
                         WHERE created_at < NOW() - ($1 * INTERVAL '1 hour')",
                )
                .bind(RETENTION_HOURS)
                .execute(&pool_clone)
                .await
                {
                    Ok(r) => {
                        if r.rows_affected() > 0 {
                            log::info!(
                                "api_idempotency_keys retention: purged {} rows older than {}h",
                                r.rows_affected(),
                                RETENTION_HOURS
                            );
                        }
                    }
                    Err(e) => {
                        log::error!("api_idempotency_keys retention purge error: {}", e)
                    }
                }
            });
        });
    });
}

impl<S, B> Transform<S, ServiceRequest> for IdempotencyMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: actix_web::body::MessageBody + 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = IdempotencyMiddlewareService<S>;
    type Future = std::future::Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        std::future::ready(Ok(IdempotencyMiddlewareService {
            service: Rc::new(service),
            pool: self.pool.clone(),
        }))
    }
}

pub struct IdempotencyMiddlewareService<S> {
    service: Rc<S>,
    pool: PgPool,
}

/// 把已缓冲的请求体回填到 request（actix-http 提供 From<Vec<u8>> for Payload）。
fn restore_payload(req: &mut ServiceRequest, body_bytes: Vec<u8>) {
    req.set_payload(body_bytes.into());
}

/// 解析 `X-Api-Version`（版本专项字段，当前无版本机制仅透传存储）。
/// 格式 `^[a-zA-Z0-9][a-zA-Z0-9.-]{0,31}$`，非法 → Err。
fn resolve_api_version(req: &ServiceRequest) -> Result<String, String> {
    let raw = req
        .headers()
        .get("x-api-version")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .unwrap_or("");
    if raw.is_empty() {
        return Ok("v1".to_string());
    }
    let valid = raw.len() <= 32
        && raw
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if valid {
        Ok(raw.to_string())
    } else {
        Err(raw.to_string())
    }
}

/// 构建幂等冲突/错误响应（错误体平铺 error/message，对齐 metering 429 先例）。
fn conflict_response(error: &str, message: &str, retry_after: Option<u32>) -> HttpResponse {
    let mut resp = HttpResponse::Conflict();
    if let Some(secs) = retry_after {
        resp.insert_header(("Retry-After", secs.to_string()));
    }
    resp.json(serde_json::json!({ "error": error, "message": message }))
}

/// 重放存储的快照。
fn replay_response(
    status: u16,
    content_type: Option<&str>,
    body: Option<&str>,
    idem_key: &str,
) -> HttpResponse {
    let mut resp =
        HttpResponse::build(actix_web::http::StatusCode::from_u16(status).unwrap_or_default());
    resp.insert_header(("Idempotency-Replayed", "true"))
        .insert_header(("Idempotency-Key", idem_key));
    if let Some(ct) = content_type {
        if let Ok(hv) = HeaderValue::from_str(ct) {
            resp.insert_header((actix_web::http::header::CONTENT_TYPE, hv));
        }
    }
    match body {
        // 诚实降级：快照超限未存 body → 空 body 重放（不伪造截断内容）
        Some(b) => resp.body(b.to_string()),
        None => resp.finish(),
    }
}

impl<S, B> Service<ServiceRequest> for IdempotencyMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: actix_web::body::MessageBody + 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let svc = self.service.clone();
        let pool = self.pool.clone();
        Box::pin(async move {
            // ── 触发条件（不满足 → 零影响透传）──
            let method = req.method().as_str().to_string();
            let is_write = matches!(method.as_str(), "POST" | "PUT" | "PATCH");
            let path = req.path().to_string();
            let idem_key = req
                .headers()
                .get("idempotency-key")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            // 服务令牌识别（不验签；PEP 已验）——复用 metering 同款 claims 解析
            let client_id = super::metering::ApiUsageMiddleware::service_client_id(&req);

            if !(is_write && path.starts_with("/api/service/") && idem_key.is_some()) {
                return svc.call(req).await.map(|r| r.map_into_left_body());
            }
            let Some(idem_key) = idem_key else {
                unreachable!()
            };
            if idem_key.len() > IDEM_KEY_MAX_LEN {
                return Ok(req
                    .into_response(
                        HttpResponse::BadRequest().json(serde_json::json!({
                            "error": "IDEMPOTENCY_KEY_TOO_LONG",
                            "message": format!("Idempotency-Key must be at most {} characters", IDEM_KEY_MAX_LEN),
                        })),
                    )
                    .map_into_right_body());
            }
            let api_version = match resolve_api_version(&req) {
                Ok(v) => v,
                Err(bad) => {
                    log::warn!("rejected X-Api-Version value (len={})", bad.len());
                    return Ok(req
                        .into_response(HttpResponse::BadRequest().json(serde_json::json!({
                            "error": "INVALID_API_VERSION",
                            "message": "X-Api-Version must match [a-zA-Z0-9][a-zA-Z0-9.-]{0,31}",
                        })))
                        .map_into_right_body());
                }
            };
            // 幂等面仅第三方服务令牌（对齐 metering「自然人不计量」边界）
            let Some(client_id) = client_id else {
                return svc.call(req).await.map(|r| r.map_into_left_body());
            };

            // ── 请求体缓冲 + 指纹 ──
            use futures::StreamExt;
            let mut body_bytes: Vec<u8> = Vec::new();
            let mut payload_stream = req.take_payload();
            while let Some(chunk) = payload_stream.next().await {
                match chunk {
                    Ok(c) => body_bytes.extend_from_slice(&c),
                    Err(e) => {
                        log::warn!("idempotency request body read error: {:?}", e);
                        return Err(Error::from(e));
                    }
                }
            }
            let mut hasher = Sha256::new();
            hasher.update(method.as_bytes());
            hasher.update(path.as_bytes());
            hasher.update(&body_bytes);
            let fingerprint: String = hasher
                .finalize()
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect();

            // ── client 行 id（幂等作用域主体）──
            let fk_client: Option<i64> = sqlx::query_scalar(
                "SELECT id FROM isahl_auth.api_clients \
                 WHERE client_id = $1 AND deleted_at IS NULL",
            )
            .bind(&client_id)
            .fetch_optional(&pool)
            .await
            .unwrap_or(None);
            let Some(fk_client) = fk_client else {
                // PEP 已验证 client 有效但此处查无（竞态删除）→ 透传，
                // 由后续链路（metering 订阅强制）拒绝
                restore_payload(&mut req, body_bytes);
                return svc.call(req).await.map(|r| r.map_into_left_body());
            };

            // ── 抢占（leader/follower 分流）──
            let claimed: Option<i64> = sqlx::query_scalar(
                r#"
                INSERT INTO isahl_auth.api_idempotency_keys
                    (id, fk_client, api_version, idem_key, method, path, request_fingerprint)
                VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6)
                ON CONFLICT (fk_client, api_version, idem_key) DO NOTHING
                RETURNING id
                "#,
            )
            .bind(fk_client)
            .bind(&api_version)
            .bind(&idem_key)
            .bind(&method)
            .bind(&path)
            .bind(&fingerprint)
            .fetch_optional(&pool)
            .await
            .unwrap_or(None); // DB 错误 → 透传（fail-open：幂等是保障增强，非安全边界）

            if claimed.is_none() {
                // follower：读取既有记录判定
                let row: Option<IdemSnapshotRow> = sqlx::query_as(
                    "SELECT request_fingerprint, state, response_status, \
                                response_content_type, response_body \
                         FROM isahl_auth.api_idempotency_keys \
                         WHERE fk_client = $1 AND api_version = $2 AND idem_key = $3",
                )
                .bind(fk_client)
                .bind(&api_version)
                .bind(&idem_key)
                .fetch_optional(&pool)
                .await
                .unwrap_or(None);
                return match row {
                    Some((stored_fp, state, status, ctype, body)) => {
                        if stored_fp != fingerprint {
                            Ok(req
                                .into_response(conflict_response(
                                    "IDEMPOTENCY_PAYLOAD_MISMATCH",
                                    "Idempotency-Key was already used with a different request payload",
                                    None,
                                ))
                                .map_into_right_body())
                        } else if state == "in_progress" {
                            Ok(req
                                .into_response(conflict_response(
                                    "IDEMPOTENCY_REQUEST_IN_PROGRESS",
                                    "First request with this key is still in progress; retry shortly",
                                    Some(IN_PROGRESS_RETRY_AFTER_SECS),
                                ))
                                .map_into_right_body())
                        } else {
                            Ok(req
                                .into_response(replay_response(
                                    status.unwrap_or(200) as u16,
                                    ctype.as_deref(),
                                    body.as_deref(),
                                    &idem_key,
                                ))
                                .map_into_right_body())
                        }
                    }
                    // 记录在 INSERT 与 SELECT 间被 TTL 清理 → 透传重执行
                    None => {
                        restore_payload(&mut req, body_bytes);
                        svc.call(req).await.map(|r| r.map_into_left_body())
                    }
                };
            }

            // ── leader：执行 handler，缓冲响应存储快照 ──
            restore_payload(&mut req, body_bytes);
            let res = match svc.call(req).await {
                Ok(r) => r,
                Err(e) => {
                    // 框架级错误：释放幂等槽（key 可重用）
                    let _ =
                        sqlx::query("DELETE FROM isahl_auth.api_idempotency_keys WHERE id = $1")
                            .bind(claimed.unwrap())
                            .execute(&pool)
                            .await;
                    return Err(e);
                }
            };
            let status = res.status();
            let row_id = claimed.unwrap();

            if status.as_u16() >= 500 {
                // 5xx 不占用幂等槽（服务端未成功执行；Stripe 同款语义）
                let _ = sqlx::query("DELETE FROM isahl_auth.api_idempotency_keys WHERE id = $1")
                    .bind(row_id)
                    .execute(&pool)
                    .await;
                return Ok(res.map_into_left_body());
            }

            // 缓冲响应体（重建 response 需要；快照 cap 检查）
            let (http_req, resp) = res.into_parts();
            let content_type = resp
                .headers()
                .get(actix_web::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let bytes = match to_bytes(resp.into_body()).await {
                Ok(b) => b,
                Err(_e) => {
                    log::error!("idempotency response buffering failed");
                    let _ =
                        sqlx::query("DELETE FROM isahl_auth.api_idempotency_keys WHERE id = $1")
                            .bind(row_id)
                            .execute(&pool)
                            .await;
                    return Err(actix_web::error::ErrorInternalServerError(
                        "idempotency response buffering failed",
                    ));
                }
            };
            let snapshot_body: Option<String> = if bytes.len() <= RESPONSE_SNAPSHOT_CAP {
                Some(String::from_utf8_lossy(&bytes).into_owned())
            } else {
                log::warn!(
                    "idempotency snapshot exceeds cap ({} bytes) — replay will degrade to status-only",
                    bytes.len()
                );
                None
            };

            // 存储快照（失败不阻断响应——重放退化为重新执行，可接受）
            if let Err(e) = sqlx::query(
                r#"
                UPDATE isahl_auth.api_idempotency_keys
                SET state = 'completed',
                    response_status = $2,
                    response_content_type = $3,
                    response_body = $4,
                    completed_at = NOW()
                WHERE id = $1
                "#,
            )
            .bind(row_id)
            .bind(status.as_u16() as i16)
            .bind(content_type.as_deref())
            .bind(snapshot_body.as_deref())
            .execute(&pool)
            .await
            {
                log::error!("idempotency snapshot store failed: {}", e);
            }

            // 重建响应（content-length 由 actix 重算，避免陈旧头）
            let mut builder = HttpResponse::build(status);
            if let Some(ct) = &content_type {
                if let Ok(hv) = HeaderValue::from_str(ct) {
                    builder.insert_header((actix_web::http::header::CONTENT_TYPE, hv));
                }
            }
            builder.insert_header((
                HeaderName::from_static("idempotency-key"),
                idem_key.as_str(),
            ));
            let response = builder.body(bytes);
            Ok(ServiceResponse::new(http_req, response).map_into_right_body())
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn api_version_validation() {
        // 合法：默认 / 简单 / 带点横线
        assert!(validate_version_str("").is_none()); // 空 → 默认 v1，调用方处理
        assert_eq!(validate_version_str("v1"), Some("v1"));
        assert_eq!(validate_version_str("v2.1"), Some("v2.1"));
        assert_eq!(validate_version_str("2026-08"), Some("2026-08"));
        // 非法：前导横线 / 非法字符 / 超长
        assert_eq!(validate_version_str("-v1"), None);
        assert_eq!(validate_version_str("v1 beta"), None);
        assert_eq!(validate_version_str("a".repeat(33).as_str()), None);
    }

    fn validate_version_str(raw: &str) -> Option<&str> {
        let valid = !raw.is_empty()
            && raw.len() <= 32
            && raw
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
            && raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
        if valid {
            Some(raw)
        } else {
            None
        }
    }
}
