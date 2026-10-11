//! 义务配置面（admin CRUD）——`isahl_auth.ngac_obligation` 的**唯一**管理写径。
//!
//! 规约：`openspec/specs/ngac-admin :: org-policy-declaration-field-wiring`（配置面接线）+
//! `ngac-obligation-evaluation`（求值语义不变；本模块只补配置通道）。
//! 语义边界：
//! - 表达式字段 `event_pattern` / `response_ops` 写前 MUST 过统一校验家族
//!   （`crate::ngac::org_policy::validate_org_policy_expression`；键集 `ORG_POLICY_EXPR_KEYS`
//!   与义务求值上下文变量**逐名一致**），fail-closed；`response_ops` 允许第二形态（静态算子字符串数组），
//!   逐元素过同一校验。
//! - 写径 MUST 同事务写策略审计（`ngac_policy_audit_log`，`entity_type = ngac_obligation`）；
//!   审计写失败 MUST 回滚业务变更（与既有 `policy-change-audit-wiring` 同口径）。
//! - 出参 64bit id MUST 经 `serde_zuid` 字符串化（`ID_JSON_PRECISION`）。
//! - 派生行（`o_name = org_policy:{class.code}:{rule.id}`，见 `common::ngac_policy`）与本端点
//!   同表：删除派生行同样走本端点（软删后重派生**不复活** admin 软删行——墓碑语义与派生器一致）。

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

use crate::admin::handlers::require_admin;
use crate::auth::AuthState;

/// admin 门控（与 `ngac::analysis` 同源实现：SSO `require_admin` 唯一语义实现）。
async fn guard_admin(req: &HttpRequest, pool: &PgPool) -> Result<i64, HttpResponse> {
    let state = req
        .app_data::<web::Data<AuthState>>()
        .cloned()
        .expect("AuthState in app data");
    require_admin(req, pool, &state).await
}

/// 义务行（出参；`id` 经 `serde_zuid` 字符串化）。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ObligationRow {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: Option<String>,
    pub event_pattern: Option<String>,
    pub response_ops: Option<Value>,
    pub is_active: bool,
    pub priority: i32,
    pub conditions: Option<Value>,
    pub phase: String,
}

/// 新建 / 局部更新入参（`None` ⇒ 保持现值，COALESCE 语义）。
#[derive(Debug, Deserialize)]
pub struct ObligationUpsertRequest {
    #[serde(default)]
    o_name: Option<String>,
    #[serde(default)]
    event_pattern: Option<String>,
    #[serde(default)]
    response_ops: Option<Value>,
    #[serde(default)]
    conditions: Option<Value>,
    #[serde(default)]
    phase: Option<String>,
    #[serde(default)]
    priority: Option<i32>,
    #[serde(default)]
    is_active: Option<bool>,
}

/// 义务表达式写前校验（唯一校验家族；键集与义务求值上下文一致）。
///
/// - `null` / 缺省 ⇒ 放行（未设置）；
/// - JSON 字符串 ⇒ **Rhai 源**（求值侧语义：字符串即表达式 ⇒ 必过
///   `validate_all`：语法 + 标识符 ⊆ `ORG_POLICY_EXPR_KEYS` + 函数白名单）；
/// - JSON 字符串数组 ⇒ **静态算子清单**（求值侧第二形态，元素是算子串而非 Rhai 代码 ⇒
///   只校验「非空字符串」；算子白名单归消费侧执行面 fail-visible，不在写径收窄）；
/// - 其他形态 ⇒ 拒绝（MUST NOT 以未校验的结构化占位承载表达式）。
fn validate_obligation_expr(field: &str, value: &Value) -> Result<(), String> {
    match value {
        Value::Null => Ok(()),
        Value::String(_) => crate::ngac::org_policy::validate_org_policy_expression(field, value)
            .map_err(|e| format!("{field}: {e}")),
        Value::Array(items) => {
            if items.is_empty() {
                return Err(format!("{field}: EMPTY_ARRAY"));
            }
            for (i, item) in items.iter().enumerate() {
                match item.as_str() {
                    Some(s) if !s.trim().is_empty() => {}
                    _ => return Err(format!("{field}[{i}]: MUST be non-empty string")),
                }
            }
            Ok(())
        }
        _ => Err(format!(
            "{field}: MUST be Rhai source string or string array"
        )),
    }
}

/// `phase` 校验（DDL CHECK 同语义；先于写库给出 400 而非 500）。
fn validate_phase(phase: &str) -> bool {
    phase == "pre" || phase == "post"
}

/// 入参校验聚合（新建与更新共用）。返回 `Err(reason)` ⇒ 400。
fn validate_upsert(b: &ObligationUpsertRequest) -> Result<(), String> {
    if let Some(p) = b.phase.as_deref() {
        if !validate_phase(p) {
            return Err("INVALID_PHASE".to_string());
        }
    }
    if let Some(p) = b.event_pattern.as_deref() {
        validate_obligation_expr("event_pattern", &Value::String(p.to_string()))?;
    }
    if let Some(ops) = b.response_ops.as_ref() {
        validate_obligation_expr("response_ops", ops)?;
    }
    if let Some(name) = b.o_name.as_deref() {
        if name.trim().is_empty() && b.o_name.is_some() {
            return Err("o_name MUST NOT be empty".to_string());
        }
    }
    // `conditions` 键集白名单（tasks 1.5）：核心 `evaluate_conditions` 对未知键 fail-closed
    // （义务永不触发）⇒ 写径 MUST 早拒（400）而非静默接受。键集真源 = `ngac_decision::CONDITION_KEYS`。
    if let Some(c) = b.conditions.as_ref() {
        ngac_host::validate_condition_keys(c)?;
    }
    Ok(())
}

/// GET /api/admin/ngac/obligations —— 义务清单（未删；按 priority/id 升序，与求值序一致）。
pub async fn list_obligations(req: HttpRequest, pool: web::Data<PgPool>) -> HttpResponse {
    if let Err(resp) = guard_admin(&req, pool.get_ref()).await {
        return resp;
    }
    // 编译期字面量 SQL（零运行期拼接——dynamic-table-name 门禁合规）。
    match sqlx::query_as::<_, ObligationRow>(
        "SELECT id, o_name, event_pattern, response_ops, is_active, priority, conditions, phase \
           FROM isahl_auth.ngac_obligation \
          WHERE deleted_at IS NULL ORDER BY priority ASC NULLS LAST, id ASC",
    )
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(rows) => HttpResponse::Ok().json(serde_json::json!({ "obligations": rows })),
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({ "error": format!("obligations: {e}") })),
    }
}

/// POST /api/admin/ngac/obligations —— 新建义务（校验 fail-closed + 同事务审计）。
pub async fn create_obligation(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<ObligationUpsertRequest>,
) -> HttpResponse {
    let admin_id = match guard_admin(&req, pool.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let b = body.into_inner();
    if let Err(reason) = validate_upsert(&b) {
        return HttpResponse::BadRequest().json(serde_json::json!({ "error": reason }));
    }
    let phase = b.phase.clone().unwrap_or_else(|| "post".to_string());

    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": format!("tx: {e}") }))
        }
    };
    let row: Option<ObligationRow> = match sqlx::query_as(
        "INSERT INTO isahl_auth.ngac_obligation \
             (o_name, event_pattern, response_ops, is_active, priority, conditions, phase, \
              created_by_id, updated_by_id, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8, NOW(), NOW()) \
         RETURNING id, o_name, event_pattern, response_ops, is_active, priority, conditions, phase",
    )
    .bind(&b.o_name)
    .bind(&b.event_pattern)
    .bind(
        b.response_ops
            .clone()
            .unwrap_or_else(|| Value::Array(vec![])),
    )
    .bind(b.is_active.unwrap_or(true))
    .bind(b.priority.unwrap_or(0))
    .bind(
        b.conditions
            .clone()
            .unwrap_or_else(|| serde_json::json!({})),
    )
    .bind(&phase)
    .bind(admin_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": format!("insert: {e}") }))
        }
    };
    let Some(row) = row else {
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({ "error": "insert returned no row" }));
    };
    let audit = crate::ngac::audit_writer::AuditRecord {
        action: "insert",
        entity_type: "ngac_obligation",
        entity_id: row.id,
        old_values: None,
        new_values: Some(serde_json::json!({
            "o_name": row.o_name, "event_pattern": row.event_pattern,
            "response_ops": row.response_ops, "phase": row.phase,
            "priority": row.priority, "is_active": row.is_active,
        })),
        actor: admin_id,
        session_id: None,
        ip_address: None,
    };
    if let Err(e) = crate::ngac::audit_writer::write_audit_tx(&mut tx, &audit).await {
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({ "error": format!("audit: {e}") }));
    }
    if let Err(e) = tx.commit().await {
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({ "error": format!("commit: {e}") }));
    }
    HttpResponse::Created().json(row)
}

/// PUT /api/admin/ngac/obligations/{id} —— 局部更新（COALESCE；校验 + 同事务审计）。
pub async fn update_obligation(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<ObligationUpsertRequest>,
) -> HttpResponse {
    let admin_id = match guard_admin(&req, pool.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let id: i64 = match req.match_info().get("id").and_then(|v| v.parse().ok()) {
        Some(v) => v,
        None => {
            return HttpResponse::BadRequest().json(serde_json::json!({ "error": "INVALID_ID" }))
        }
    };
    let b = body.into_inner();
    if let Err(reason) = validate_upsert(&b) {
        return HttpResponse::BadRequest().json(serde_json::json!({ "error": reason }));
    }

    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": format!("tx: {e}") }))
        }
    };
    let updated: Option<ObligationRow> = match sqlx::query_as(
        "UPDATE isahl_auth.ngac_obligation \
            SET o_name = COALESCE($2, o_name), \
                event_pattern = COALESCE($3, event_pattern), \
                response_ops = COALESCE($4, response_ops), \
                conditions = COALESCE($5, conditions), \
                phase = COALESCE($6, phase), \
                priority = COALESCE($7, priority), \
                is_active = COALESCE($8, is_active), \
                updated_at = NOW(), updated_by_id = $9 \
          WHERE id = $1 AND deleted_at IS NULL \
         RETURNING id, o_name, event_pattern, response_ops, is_active, priority, conditions, phase",
    )
    .bind(id)
    .bind(&b.o_name)
    .bind(&b.event_pattern)
    .bind(b.response_ops.clone())
    .bind(b.conditions.clone())
    .bind(&b.phase)
    .bind(b.priority)
    .bind(b.is_active)
    .bind(admin_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": format!("update: {e}") }))
        }
    };
    let Some(row) = updated else {
        return HttpResponse::NotFound().json(serde_json::json!({ "error": "NOT_FOUND" }));
    };
    let audit = crate::ngac::audit_writer::AuditRecord {
        action: "update",
        entity_type: "ngac_obligation",
        entity_id: row.id,
        old_values: None,
        new_values: Some(serde_json::json!({
            "o_name": row.o_name, "event_pattern": row.event_pattern,
            "response_ops": row.response_ops, "phase": row.phase,
            "priority": row.priority, "is_active": row.is_active,
        })),
        actor: admin_id,
        session_id: None,
        ip_address: None,
    };
    if let Err(e) = crate::ngac::audit_writer::write_audit_tx(&mut tx, &audit).await {
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({ "error": format!("audit: {e}") }));
    }
    if let Err(e) = tx.commit().await {
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({ "error": format!("commit: {e}") }));
    }
    HttpResponse::Ok().json(row)
}

/// DELETE /api/admin/ngac/obligations/{id} —— 软删（同事务审计；派生行同面）。
pub async fn delete_obligation(req: HttpRequest, pool: web::Data<PgPool>) -> HttpResponse {
    let admin_id = match guard_admin(&req, pool.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let id: i64 = match req.match_info().get("id").and_then(|v| v.parse().ok()) {
        Some(v) => v,
        None => {
            return HttpResponse::BadRequest().json(serde_json::json!({ "error": "INVALID_ID" }))
        }
    };
    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": format!("tx: {e}") }))
        }
    };
    let deleted: Option<(Option<String>,)> = match sqlx::query_as(
        "UPDATE isahl_auth.ngac_obligation \
            SET deleted_at = NOW(), is_active = FALSE, updated_at = NOW(), updated_by_id = $2 \
          WHERE id = $1 AND deleted_at IS NULL \
         RETURNING o_name",
    )
    .bind(id)
    .bind(admin_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": format!("delete: {e}") }))
        }
    };
    let Some((o_name,)) = deleted else {
        return HttpResponse::NotFound().json(serde_json::json!({ "error": "NOT_FOUND" }));
    };
    let audit = crate::ngac::audit_writer::AuditRecord {
        action: "delete",
        entity_type: "ngac_obligation",
        entity_id: id,
        old_values: None,
        // 审计行 MUST 自体可定位被删实体（软删后行仍在库，`o_name` 是唯一业务句柄）。
        new_values: Some(serde_json::json!({ "o_name": o_name, "soft_deleted": true })),
        actor: admin_id,
        session_id: None,
        ip_address: None,
    };
    if let Err(e) = crate::ngac::audit_writer::write_audit_tx(&mut tx, &audit).await {
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({ "error": format!("audit: {e}") }));
    }
    if let Err(e) = tx.commit().await {
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({ "error": format!("commit: {e}") }));
    }
    HttpResponse::Ok().json(serde_json::json!({ "deleted": 1 }))
}

/// 注册义务配置端点（挂既有 `/api/admin` scope；`require_admin` 门控）。
pub fn configure_admin_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/ngac/obligations", web::get().to(list_obligations))
        .route("/ngac/obligations", web::post().to(create_obligation))
        .route("/ngac/obligations/{id}", web::put().to(update_obligation))
        .route(
            "/ngac/obligations/{id}",
            web::delete().to(delete_obligation),
        );
}
