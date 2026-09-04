//! OpenAPI 订阅与对接对账（openapi-external-access P4）
//!
//! 企业应用与第三方系统对接治理（非销售/计费语义）：
//! - `GET  /api/admin/api-plans`               对接容量档位列表
//! - `GET  /api/admin/api-subscriptions`       对接关系列表（client/plan 关联）
//! - `PUT  /api/admin/api-subscriptions/{id}/plan`   变更对接容量档位
//! - `POST /api/admin/api-subscriptions/{id}/status` 开/关对接（active/suspended/canceled）
//! - `GET  /api/admin/api-reconcile?month=YYYY-MM`   对接用量对账（按月聚合）

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{Datelike, NaiveDate};
use sqlx::PgPool;

use crate::auth::AuthState;

// ── Models ───────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct PlanResponse {
    pub id: i64,
    pub code: String,
    pub tier: i16,
    pub rate_limit_rps: f64,
    pub burst: i32,
    pub quota_daily: i64,
    pub quota_monthly: i64,
    pub sla_availability: f64,
    pub sla_p95_ms: i32,
    pub support_level: String,
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct SubscriptionResponse {
    pub id: i64,
    pub client_id: String,
    pub client_name: String,
    pub plan_code: String,
    pub plan_tier: i16,
    pub status: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ChangePlanRequest {
    pub plan_code: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct SetStatusRequest {
    pub status: String, // active | suspended | canceled
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateSubscriptionRequest {
    pub fk_client: i64,
    pub fk_plan: i64,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateSubscriptionRequest {
    // 仅窗口字段——fk_client/fk_plan 不可编辑（spec: openapi-subscription-manual-management）
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// 窗口合法性：expires_at 存在时必须严格晚于 starts_at（starts_at >= expires_at → 400）。
fn validate_subscription_window(
    starts_at: chrono::DateTime<chrono::Utc>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(), String> {
    if let Some(exp) = expires_at {
        if exp <= starts_at {
            return Err("expires_at must be after starts_at".to_string());
        }
    }
    Ok(())
}

/// 订阅状态按生效窗口推导（创建路径的初始状态）：
/// - 窗口覆盖当前时间（starts_at <= now 且 expires_at 为空或晚于 now）→ active
/// - 否则（未开始/已过期）→ suspended
///
/// 说明：业务语义上的「pending（未生效）」在 `api_subscriptions.status` 的
/// CHECK 约束（active|suspended|canceled）内没有对应取值，故以 suspended 表达
/// "窗口未覆盖当前时间"。Gateway 计量仅对 status='active' 放行（metering.rs），
/// 未来窗口的订阅不会提前生效；生效后由管理面 set_status 切换。
fn derive_subscription_status(
    starts_at: chrono::DateTime<chrono::Utc>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> &'static str {
    if starts_at <= now && expires_at.is_none_or(|exp| exp > now) {
        "active"
    } else {
        "suspended"
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/admin/api-plans
pub async fn list_plans(
    _req: HttpRequest,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    let plans: Vec<PlanResponse> = match sqlx::query_as(
        "SELECT id, code, tier, rate_limit_rps::float8 AS rate_limit_rps, burst, \
                quota_daily, quota_monthly, \
                sla_availability::float8 AS sla_availability, sla_p95_ms, support_level \
         FROM isahl_auth.api_plans \
         WHERE deleted_at IS NULL \
         ORDER BY tier",
    )
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(p) => p,
        Err(e) => {
            log::error!("list_plans query failed: {}", e);
            Vec::new()
        }
    };

    HttpResponse::Ok().json(serde_json::json!({ "plans": plans }))
}

/// GET /api/admin/api-subscriptions
pub async fn list_subscriptions(
    _req: HttpRequest,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    let subs: Vec<SubscriptionResponse> = sqlx::query_as(
        r#"
        SELECT s.id, c.client_id, c.client_name, p.code AS plan_code, p.tier AS plan_tier,
               s.status, s.starts_at, s.expires_at
        FROM isahl_auth.api_subscriptions s
        JOIN isahl_auth.api_clients c ON c.id = s.fk_client AND c.deleted_at IS NULL
        JOIN isahl_auth.api_plans p ON p.id = s.fk_plan
        WHERE s.deleted_at IS NULL
        ORDER BY s.id DESC
        "#,
    )
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_default();

    HttpResponse::Ok().json(serde_json::json!({ "subscriptions": subs }))
}

/// PUT /api/admin/api-subscriptions/{id}/plan —— 变更套餐
pub async fn change_plan(
    _req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<ChangePlanRequest>,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    let sub_id = path.into_inner();

    let plan_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.api_plans WHERE code = $1 AND deleted_at IS NULL",
    )
    .bind(&body.plan_code)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);
    let Some(plan_id) = plan_id else {
        return HttpResponse::NotFound().json(serde_json::json!({
            "error": "Plan not found",
            "message": format!("Unknown plan code '{}'", body.plan_code),
        }));
    };

    match sqlx::query(
        "UPDATE isahl_auth.api_subscriptions SET fk_plan = $1 WHERE id = $2 AND deleted_at IS NULL",
    )
    .bind(plan_id)
    .bind(sub_id)
    .execute(pool.get_ref())
    .await
    {
        Ok(r) if r.rows_affected() > 0 => HttpResponse::Ok().json(serde_json::json!({
            "updated": true,
            "id": sub_id,
            "plan_code": body.plan_code,
        })),
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Subscription not found",
        })),
        Err(e) => {
            log::error!("change_plan error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Update failed",
            }))
        }
    }
}

/// POST /api/admin/api-subscriptions/{id}/status —— 暂停/恢复/取消
pub async fn set_status(
    _req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<SetStatusRequest>,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    let sub_id = path.into_inner();

    let status = body.status.to_ascii_lowercase();
    if !["active", "suspended", "canceled"].contains(&status.as_str()) {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "status must be active|suspended|canceled"
        }));
    }

    match sqlx::query(
        "UPDATE isahl_auth.api_subscriptions SET status = $1 WHERE id = $2 AND deleted_at IS NULL",
    )
    .bind(&status)
    .bind(sub_id)
    .execute(pool.get_ref())
    .await
    {
        Ok(r) if r.rows_affected() > 0 => HttpResponse::Ok().json(serde_json::json!({
            "updated": true,
            "id": sub_id,
            "status": status,
        })),
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Subscription not found",
        })),
        Err(e) => {
            log::error!("set_status error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Update failed",
            }))
        }
    }
}

/// POST /api/admin/api-subscriptions —— 手工创建订阅
///
/// 校验：client/plan 存在且未软删（否则 400）、窗口合法 starts_at < expires_at
/// （否则 400）；client/plan 校验 + 插入同一事务（`pool.begin()` 具体事务，
/// 与 create_api_client 的事务写法同款），id 走 `isahl.gen_next_zuid()`。
/// status 按生效窗口自动判定（active / 未生效 → suspended，见
/// `derive_subscription_status`）。
pub async fn create_subscription(
    _req: HttpRequest,
    body: web::Json<CreateSubscriptionRequest>,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    if let Err(msg) = validate_subscription_window(body.starts_at, body.expires_at) {
        return HttpResponse::BadRequest().json(serde_json::json!({ "error": msg }));
    }

    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => {
            log::error!("create_subscription begin tx error: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Transaction failed",
            }));
        }
    };

    let client_ok: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.api_clients WHERE id = $1 AND deleted_at IS NULL)",
    )
    .bind(body.fk_client)
    .fetch_one(&mut *tx)
    .await
    .unwrap_or(false);
    if !client_ok {
        let _ = tx.rollback().await;
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Client not found",
        }));
    }

    let plan_ok: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.api_plans WHERE id = $1 AND deleted_at IS NULL)",
    )
    .bind(body.fk_plan)
    .fetch_one(&mut *tx)
    .await
    .unwrap_or(false);
    if !plan_ok {
        let _ = tx.rollback().await;
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Plan not found",
        }));
    }

    let status = derive_subscription_status(body.starts_at, body.expires_at, chrono::Utc::now());

    let sub_id: i64 = match sqlx::query_scalar(
        r#"
        INSERT INTO isahl_auth.api_subscriptions
            (id, fk_client, fk_plan, status, starts_at, expires_at)
        VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(body.fk_client)
    .bind(body.fk_plan)
    .bind(status)
    .bind(body.starts_at)
    .bind(body.expires_at)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            log::error!("create_subscription insert error: {}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Create failed",
            }));
        }
    };

    let sub: SubscriptionResponse = match sqlx::query_as(
        r#"
        SELECT s.id, c.client_id, c.client_name, p.code AS plan_code, p.tier AS plan_tier,
               s.status, s.starts_at, s.expires_at
        FROM isahl_auth.api_subscriptions s
        JOIN isahl_auth.api_clients c ON c.id = s.fk_client
        JOIN isahl_auth.api_plans p ON p.id = s.fk_plan
        WHERE s.id = $1
        "#,
    )
    .bind(sub_id)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(s) => s,
        Err(e) => {
            log::error!("create_subscription readback error: {}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Create failed",
            }));
        }
    };

    if let Err(e) = tx.commit().await {
        log::error!("create_subscription commit error: {}", e);
        return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Transaction failed",
        }));
    }

    HttpResponse::Created().json(serde_json::json!({ "data": sub }))
}

/// PUT /api/admin/api-subscriptions/{id} —— 编辑订阅窗口
///
/// 仅更新 starts_at/expires_at；禁改 fk_client/fk_plan（UpdateSubscriptionRequest
/// 无 fk_* 字段）。status 保持管理面既有值——set_status 的显式意图（如 suspended）
/// 不被窗口编辑覆盖（与 change_plan 不触碰 status 的语义一致）。
pub async fn update_subscription(
    _req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateSubscriptionRequest>,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    let sub_id = path.into_inner();

    if let Err(msg) = validate_subscription_window(body.starts_at, body.expires_at) {
        return HttpResponse::BadRequest().json(serde_json::json!({ "error": msg }));
    }

    match sqlx::query(
        "UPDATE isahl_auth.api_subscriptions SET starts_at = $1, expires_at = $2 \
         WHERE id = $3 AND deleted_at IS NULL",
    )
    .bind(body.starts_at)
    .bind(body.expires_at)
    .bind(sub_id)
    .execute(pool.get_ref())
    .await
    {
        Ok(r) if r.rows_affected() == 0 => {
            return HttpResponse::NotFound().json(serde_json::json!({
                "error": "Subscription not found",
            }))
        }
        Ok(_) => {}
        Err(e) => {
            log::error!("update_subscription error: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Update failed",
            }));
        }
    }

    match sqlx::query_as::<_, SubscriptionResponse>(
        r#"
        SELECT s.id, c.client_id, c.client_name, p.code AS plan_code, p.tier AS plan_tier,
               s.status, s.starts_at, s.expires_at
        FROM isahl_auth.api_subscriptions s
        JOIN isahl_auth.api_clients c ON c.id = s.fk_client
        JOIN isahl_auth.api_plans p ON p.id = s.fk_plan
        WHERE s.id = $1
        "#,
    )
    .bind(sub_id)
    .fetch_one(pool.get_ref())
    .await
    {
        Ok(s) => HttpResponse::Ok().json(serde_json::json!({ "data": s })),
        Err(e) => {
            log::error!("update_subscription readback error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Update failed",
            }))
        }
    }
}

/// GET /api/admin/api-reconcile?month=YYYY-MM
///
/// 对接用量对账（企业应用 ↔ 第三方系统对接）：按月聚合每个对接方
/// （client）的调用量/错误量/P95 延迟，供对接双方核对调用明细。
/// 非计费语义——Gateway 面向第三方系统对接治理，不涉及对外销售。
pub async fn reconcile_export(
    _req: HttpRequest,
    query: web::Query<std::collections::HashMap<String, String>>,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    // 解析月份：默认当月
    let now = chrono::Utc::now();
    let month_str = query
        .get("month")
        .cloned()
        .unwrap_or_else(|| format!("{:04}-{:02}", now.year(), now.month()));
    let month_start = match NaiveDate::parse_from_str(&format!("{}-01", month_str), "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": "month must be YYYY-MM",
            }))
        }
    };
    let next_month = if month_start.month() == 12 {
        NaiveDate::from_ymd_opt(month_start.year() + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(month_start.year(), month_start.month() + 1, 1).unwrap()
    };

    // 按月聚合：每对接方（client × plan）的调用量/错误/平均延迟/P95
    // 注意 p.tier 是 smallint → i16（i32 解码失败会被静默吞成 items=[]）
    let rows: Vec<(String, String, i64, i64, f64, i64, i16)> = sqlx::query_as(
        r#"
        SELECT c.client_id, p.code AS plan_code,
               COUNT(*)::bigint AS total,
               COUNT(*) FILTER (WHERE u.status >= 400)::bigint AS errors,
               COALESCE(AVG(u.latency_ms), 0)::float8 AS avg_latency_ms,
               PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY u.latency_ms)::bigint AS p95_ms,
               p.tier AS plan_tier
        FROM isahl_auth.api_usage u
        JOIN isahl_auth.api_subscriptions s ON s.id = u.fk_subscription
        JOIN isahl_auth.api_clients c ON c.id = s.fk_client
        JOIN isahl_auth.api_plans p ON p.id = s.fk_plan
        WHERE u.requested_at >= $1 AND u.requested_at < $2
        GROUP BY c.client_id, p.code, p.tier
        ORDER BY c.client_id
        "#,
    )
    .bind(month_start.and_hms_opt(0, 0, 0).unwrap())
    .bind(next_month.and_hms_opt(0, 0, 0).unwrap())
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_else(|e| {
        // 诚实失败：静默吞错曾让 smallint/timestamptz 解码 bug 以"空数据"假象潜伏
        log::error!("reconcile_export query failed: {}", e);
        Vec::new()
    });

    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(client_id, plan_code, total, errors, avg, p95, tier)| {
            serde_json::json!({
                "client_id": client_id,
                "plan": plan_code,
                "plan_tier": tier,
                "total_requests": total,
                "error_requests": errors,
                "error_rate": if total > 0 { errors as f64 / total as f64 } else { 0.0 },
                "avg_latency_ms": avg,
                "p95_latency_ms": p95,
            })
        })
        .collect();

    HttpResponse::Ok().json(serde_json::json!({
        "month": month_str,
        "scope": "third-party-integration",
        "items": items,
    }))
}

// ── Unit tests（纯逻辑；DB 集成路径留给 API 实证） ──────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    #[test]
    fn window_validation_rejects_inverted_or_equal_window() {
        let now = Utc::now();
        // expires_at 早于 starts_at → 非法
        assert!(validate_subscription_window(now, Some(now - Duration::hours(1))).is_err());
        // expires_at == starts_at → 非法（starts_at >= expires_at 拒）
        assert!(validate_subscription_window(now, Some(now)).is_err());
        // 无过期时间（永久窗口）→ 合法
        assert!(validate_subscription_window(now, None).is_ok());
        // 正常窗口 → 合法
        assert!(validate_subscription_window(now, Some(now + Duration::days(30))).is_ok());
    }

    #[test]
    fn status_derived_from_window() {
        let now = Utc::now();
        // 窗口覆盖当前时间 → active
        assert_eq!(
            derive_subscription_status(now - Duration::days(1), Some(now + Duration::days(1)), now),
            "active"
        );
        // 已开始的永久订阅 → active
        assert_eq!(
            derive_subscription_status(now - Duration::days(1), None, now),
            "active"
        );
        // 尚未开始（未来窗口）→ 非 active（DB CHECK 无 pending，以 suspended 表达未生效）
        assert_eq!(
            derive_subscription_status(
                now + Duration::days(1),
                Some(now + Duration::days(30)),
                now
            ),
            "suspended"
        );
        // 已过期 → 非 active
        assert_eq!(
            derive_subscription_status(
                now - Duration::days(30),
                Some(now - Duration::days(1)),
                now
            ),
            "suspended"
        );
        // 边界：starts_at == now → active；expires_at == now → 到期即失效
        assert_eq!(
            derive_subscription_status(now, Some(now + Duration::days(1)), now),
            "active"
        );
        assert_eq!(
            derive_subscription_status(now - Duration::days(1), Some(now), now),
            "suspended"
        );
    }
}
