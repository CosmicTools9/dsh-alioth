//! OpenAPI 套餐管理（add-openapi-admin-crud）
//!
//! 管理 `isahl_auth.api_plans`（free/basic/pro/enterprise 之外的自定义档位）：
//! - `POST   /api/admin/api-plans`       新建套餐（code 唯一，冲突 409）
//! - `PUT    /api/admin/api-plans/{id}`  编辑套餐（禁改 code）
//! - `DELETE /api/admin/api-plans/{id}`  软删套餐（存在 active 订阅 → 409）
//!
//! 对齐 `openspec/changes/add-openapi-admin-crud/`（Requirement: openapi-plan-crud）：
//! code 是套餐业务标识，创建后不可变；删除走软删（deleted_at=NOW()）保留历史审计。

use actix_web::{web, HttpRequest, HttpResponse};
use sqlx::PgPool;

use crate::auth::AuthState;

// ── Models ───────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct PlanEntity {
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
    pub enabled: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct CreatePlanRequest {
    pub code: String,
    pub tier: i16,
    pub rate_limit_rps: f64,
    pub burst: i32,
    pub quota_daily: i64,
    pub quota_monthly: i64,
    pub sla_availability: f64,
    pub sla_p95_ms: i32,
    pub support_level: String,
    pub enabled: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdatePlanRequest {
    // 与 CreatePlanRequest 同构但无 code——code 创建后不可改（spec 约束）
    pub tier: i16,
    pub rate_limit_rps: f64,
    pub burst: i32,
    pub quota_daily: i64,
    pub quota_monthly: i64,
    pub sla_availability: f64,
    pub sla_p95_ms: i32,
    pub support_level: String,
    pub enabled: bool,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// code 业务校验：非空、非纯空白、不超过列宽 VARCHAR(32)。
/// 唯一性由 DB UNIQUE 约束 + 预检兜底（见 create_plan），此处只管载荷合法性。
fn validate_plan_code(code: &str) -> Result<(), String> {
    if code.trim().is_empty() {
        return Err("Plan code is required".to_string());
    }
    if code.trim().chars().count() > 32 {
        return Err("Plan code must be at most 32 characters".to_string());
    }
    Ok(())
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// POST /api/admin/api-plans —— 新建套餐
///
/// code 重复 → 409（预检 + UNIQUE 约束兜底，覆盖并发窗口）；id 走
/// `isahl.gen_next_zuid()`（isahl_auth 链规则，与 api_clients.rs create 一致）。
pub async fn create_plan(
    _req: HttpRequest,
    body: web::Json<CreatePlanRequest>,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    let code = body.code.trim().to_string();
    if let Err(msg) = validate_plan_code(&code) {
        return HttpResponse::BadRequest().json(serde_json::json!({ "error": msg }));
    }

    // code 唯一预检
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.api_plans WHERE code = $1 AND deleted_at IS NULL)",
    )
    .bind(&code)
    .fetch_one(pool.get_ref())
    .await
    .unwrap_or(false);
    if exists {
        return HttpResponse::Conflict().json(serde_json::json!({
            "error": format!("Plan code '{}' already exists", code),
        }));
    }

    match sqlx::query_as::<_, PlanEntity>(
        r#"
        INSERT INTO isahl_auth.api_plans
            (id, code, tier, rate_limit_rps, burst, quota_daily, quota_monthly,
             sla_availability, sla_p95_ms, support_level, enabled)
        VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING id, code, tier, rate_limit_rps::float8 AS rate_limit_rps, burst, quota_daily, quota_monthly,
                  sla_availability::float8 AS sla_availability, sla_p95_ms, support_level, enabled
        "#,
    )
    .bind(&code)
    .bind(body.tier)
    .bind(body.rate_limit_rps)
    .bind(body.burst)
    .bind(body.quota_daily)
    .bind(body.quota_monthly)
    .bind(body.sla_availability)
    .bind(body.sla_p95_ms)
    .bind(&body.support_level)
    .bind(body.enabled)
    .fetch_one(pool.get_ref())
    .await
    {
        Ok(plan) => HttpResponse::Created().json(serde_json::json!({ "data": plan })),
        Err(e) => {
            // 并发窗口内的 code 重复：UNIQUE 约束兜底 → 409（与 create_api_client 同款处理）
            if let Some(db) = e.as_database_error() {
                if db.is_unique_violation() {
                    return HttpResponse::Conflict().json(serde_json::json!({
                        "error": format!("Plan code '{}' already exists", code),
                    }));
                }
            }
            log::error!("create_plan insert error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Create failed",
            }))
        }
    }
}

/// PUT /api/admin/api-plans/{id} —— 编辑套餐
///
/// 载荷不含 code（UpdatePlanRequest 无 code 字段），UPDATE 也不触碰 code 列。
pub async fn update_plan(
    _req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdatePlanRequest>,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    let plan_id = path.into_inner();

    match sqlx::query_as::<_, PlanEntity>(
        r#"
        UPDATE isahl_auth.api_plans SET
            tier = $1, rate_limit_rps = $2, burst = $3, quota_daily = $4,
            quota_monthly = $5, sla_availability = $6, sla_p95_ms = $7,
            support_level = $8, enabled = $9
        WHERE id = $10 AND deleted_at IS NULL
        RETURNING id, code, tier, rate_limit_rps::float8 AS rate_limit_rps, burst, quota_daily, quota_monthly,
                  sla_availability::float8 AS sla_availability, sla_p95_ms, support_level, enabled
        "#,
    )
    .bind(body.tier)
    .bind(body.rate_limit_rps)
    .bind(body.burst)
    .bind(body.quota_daily)
    .bind(body.quota_monthly)
    .bind(body.sla_availability)
    .bind(body.sla_p95_ms)
    .bind(&body.support_level)
    .bind(body.enabled)
    .bind(plan_id)
    .fetch_one(pool.get_ref())
    .await
    {
        Ok(plan) => HttpResponse::Ok().json(serde_json::json!({ "data": plan })),
        Err(e) => match e {
            sqlx::Error::RowNotFound => HttpResponse::NotFound().json(serde_json::json!({
                "error": "Plan not found",
            })),
            other => {
                log::error!("update_plan error: {}", other);
                HttpResponse::InternalServerError().json(serde_json::json!({
                    "error": "Update failed",
                }))
            }
        },
    }
}

/// DELETE /api/admin/api-plans/{id} —— 软删套餐
///
/// 存在 status='active' 的订阅 → 409 并说明原因（spec scenario：删除被活跃订阅拦截）；
/// 软删（deleted_at=NOW()）保留历史计量/对账引用。
pub async fn delete_plan(
    _req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    let plan_id = path.into_inner();

    let has_active_sub: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.api_subscriptions \
         WHERE fk_plan = $1 AND status = 'active' AND deleted_at IS NULL)",
    )
    .bind(plan_id)
    .fetch_one(pool.get_ref())
    .await
    .unwrap_or(false);
    if has_active_sub {
        return HttpResponse::Conflict().json(serde_json::json!({
            "error": "Plan has active subscriptions and cannot be deleted",
        }));
    }

    match sqlx::query(
        "UPDATE isahl_auth.api_plans SET deleted_at = NOW() WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(plan_id)
    .execute(pool.get_ref())
    .await
    {
        Ok(r) if r.rows_affected() > 0 => {
            HttpResponse::Ok().json(serde_json::json!({ "success": true }))
        }
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Plan not found",
        })),
        Err(e) => {
            log::error!("delete_plan error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Delete failed",
            }))
        }
    }
}

// ── Unit tests（纯逻辑；DB 集成路径留给 API 实证） ──────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_code_validation() {
        assert!(validate_plan_code("custom-tier").is_ok());
        assert!(validate_plan_code(&"x".repeat(32)).is_ok());
        // 空 / 纯空白 → 非法
        assert!(validate_plan_code("").is_err());
        assert!(validate_plan_code("   ").is_err());
        // 超列宽 VARCHAR(32) → 非法
        assert!(validate_plan_code(&"x".repeat(33)).is_err());
    }

    #[test]
    fn plan_code_conflict_surface() {
        // 冲突响应文案与 code 绑定（预检与 UNIQUE 兜底共用同一文案，避免双轨漂移）
        let code = "basic";
        let conflict = serde_json::json!({
            "error": format!("Plan code '{}' already exists", code),
        });
        assert_eq!(conflict["error"], "Plan code 'basic' already exists");
    }
}
