//! Admin agent 管理面 CRUD（fix-chat-ai-feature-gaps D2.17，SSO-only）。
//!
//! 读写 isahl."zc_id_empl-agent" 配置行（trigger_crud 写入；settings JSONB 存
//! AgentConfig 可覆盖字段 {system_prompt, capabilities, available_tools, icon,
//! category, max_execution_steps, ...}；物理列 code/notice(=name)/t_color_）。
//! Registry TTL 60s 自动热更（AgentRouterAdapter.refresh_registry_if_needed）。
//! 守卫：复用 gateway_sso::admin::handlers::require_admin（admin_ngac_assist
//! 先例，NGAC admin 角色）。DELETE = 软删（deleted_at）。

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::collections::HashMap;

/// 校验 agent code 形状（小写字母/数字/下划线/连字符）
fn valid_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn bad_request(code: &str, message: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(serde_json::json!({
        "success": false,
        "error": { "code": code, "message": message }
    }))
}

fn internal_error(code: &str, message: &str) -> HttpResponse {
    HttpResponse::InternalServerError().json(serde_json::json!({
        "success": false,
        "error": { "code": code, "message": message }
    }))
}

/// R2（D2.5）内部错误泛化文案：DB/SQL 原文只进 telemetry，不透传响应。
const GENERIC_INTERNAL_MSG: &str = "操作失败，请稍后重试";

fn success<T: Serialize>(data: T) -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({ "success": true, "data": data }))
}

// ── 响应/请求类型 ─────────────────────────────────────────

#[derive(Serialize)]
pub struct AgentAdminResponse {
    pub code: String,
    pub name: String,
    pub color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public: Option<bool>,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub settings: Value,
    pub deleted: bool,
}

#[derive(Deserialize)]
pub struct CreateAgentRequest {
    pub code: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub public: Option<bool>,
    #[serde(default)]
    pub settings: Value,
}

#[derive(Deserialize)]
pub struct PatchAgentRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub public: Option<bool>,
    /// AgentConfig 可覆盖字段（浅合并进既有 settings；null 键删除）
    #[serde(default)]
    pub settings: Value,
}

/// 从 DB 行组装响应（settings.public 与顶层 public 一致化）
fn row_to_response(
    code: String,
    notice: Option<String>,
    color: Option<String>,
    settings: Option<Value>,
    deleted: bool,
) -> AgentAdminResponse {
    let mut settings = settings.unwrap_or_else(|| json!({}));
    let public = if let Value::Object(map) = &mut settings {
        map.get("public").and_then(|p| p.as_bool())
    } else {
        None
    };
    AgentAdminResponse {
        code,
        name: notice.unwrap_or_default(),
        color: color.unwrap_or_default(),
        public,
        settings,
        deleted,
    }
}

// ── 守卫 ─────────────────────────────────────────────────

async fn require_admin_id(
    req: &HttpRequest,
    pool: &PgPool,
    state: &gateway_sso::AuthState,
) -> Result<i64, HttpResponse> {
    gateway_sso::admin::handlers::require_admin(req, pool, state).await
}

// ── Handlers ─────────────────────────────────────────────

/// GET /api/chat-sessions/admin/agents — 全部配置行（含软删标记）
pub async fn list_agents(
    pool: web::Data<PgPool>,
    state: web::Data<gateway_sso::AuthState>,
    req: HttpRequest,
) -> HttpResponse {
    if let Err(resp) = require_admin_id(&req, pool.get_ref(), state.get_ref()).await {
        return resp;
    }
    let rows = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            Option<String>,
            Option<Value>,
            Option<chrono::DateTime<chrono::Utc>>,
        ),
    >(
        r#"SELECT code, notice, t_color_, settings, deleted_at
           FROM isahl."zc_id_empl-agent"
           ORDER BY id ASC"#,
    )
    .fetch_all(pool.get_ref())
    .await;
    match rows {
        Ok(rows) => {
            let items: Vec<AgentAdminResponse> = rows
                .into_iter()
                .map(|(code, notice, color, settings, deleted_at)| {
                    row_to_response(code, notice, color, settings, deleted_at.is_some())
                })
                .collect();
            success(items)
        }
        Err(e) => {
            common::telemetry::error!("admin agents: list failed: {}", e);
            internal_error("DB_ERROR", GENERIC_INTERNAL_MSG)
        }
    }
}

/// POST /api/chat-sessions/admin/agents — 新建配置行
pub async fn create_agent(
    pool: web::Data<PgPool>,
    state: web::Data<gateway_sso::AuthState>,
    req: HttpRequest,
    body: web::Json<CreateAgentRequest>,
) -> HttpResponse {
    if let Err(resp) = require_admin_id(&req, pool.get_ref(), state.get_ref()).await {
        return resp;
    }
    let code = body.code.trim().to_string();
    if !valid_code(&code) {
        return bad_request(
            "INVALID_CODE",
            "code 必须为 1-64 位小写字母/数字/下划线/连字符",
        );
    }
    if let Value::Object(map) = &body.settings {
        if map.contains_key("code") {
            return bad_request("INVALID_SETTINGS", "code 属于物理列，不能写入 settings");
        }
    }

    let mut record = HashMap::new();
    record.insert("code".to_string(), Value::String(code.clone()));
    record.insert(
        "notice".to_string(),
        Value::String(body.name.clone().unwrap_or_else(|| code.clone())),
    );
    record.insert(
        "t_color_".to_string(),
        Value::String(body.color.clone().unwrap_or_else(|| "#6366f1".to_string())),
    );
    let mut settings = body.settings.clone();
    if let Value::Object(map) = &mut settings {
        match body.public {
            Some(p) => {
                map.insert("public".to_string(), Value::Bool(p));
            }
            None => {
                map.entry("public".to_string())
                    .or_insert(Value::Bool(false));
            }
        }
    }
    record.insert("settings".to_string(), settings);

    match crate::trigger_crud::insert_with_triggers(
        pool.get_ref(),
        "zc_id_empl-agent",
        record,
        None,
    )
    .await
    {
        Ok(result_map) => {
            let code = result_map
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or(&code)
                .to_string();
            let notice = result_map
                .get("notice")
                .and_then(|v| v.as_str())
                .map(String::from);
            let color = result_map
                .get("t_color_")
                .and_then(|v| v.as_str())
                .map(String::from);
            let settings = result_map.get("settings").cloned();
            success(row_to_response(code, notice, color, settings, false))
        }
        Err(e) => {
            let msg = format!("{}", e);
            if msg.contains("duplicate key") {
                bad_request("CODE_EXISTS", &format!("agent code '{}' 已存在", code))
            } else {
                common::telemetry::error!("admin agents: create '{}' failed: {}", code, e);
                internal_error("DB_ERROR", GENERIC_INTERNAL_MSG)
            }
        }
    }
}

/// PATCH /api/chat-sessions/admin/agents/{code} — 更新（物理列 + settings 浅合并）
pub async fn patch_agent(
    pool: web::Data<PgPool>,
    state: web::Data<gateway_sso::AuthState>,
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<PatchAgentRequest>,
) -> HttpResponse {
    if let Err(resp) = require_admin_id(&req, pool.get_ref(), state.get_ref()).await {
        return resp;
    }
    let code = path.into_inner();
    if !valid_code(&code) {
        return bad_request("INVALID_CODE", "非法 code");
    }

    // 旧行（update_with_triggers 前置输入 + code 定位）
    let old: Option<(Value,)> = sqlx::query_as(
        r#"SELECT to_jsonb(e) AS record FROM isahl."zc_id_empl-agent" AS e
           WHERE e.code = $1 AND e.deleted_at IS NULL"#,
    )
    .bind(&code)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(|e| common::telemetry::error!("admin agents: load '{}' failed: {}", code, e))
    .ok()
    .flatten();
    let Some((old_json,)) = old else {
        return bad_request("AGENT_NOT_FOUND", &format!("agent '{}' 不存在", code));
    };
    let old_map: HashMap<String, Value> = match serde_json::from_value(old_json) {
        Ok(m) => m,
        Err(e) => {
            common::telemetry::error!("admin agents: row parse failed for '{}': {}", code, e);
            return internal_error("DB_ERROR", GENERIC_INTERNAL_MSG);
        }
    };

    let mut record = HashMap::new();
    if let Some(name) = &body.name {
        record.insert("notice".to_string(), Value::String(name.trim().to_string()));
    }
    if let Some(color) = &body.color {
        record.insert("t_color_".to_string(), Value::String(color.clone()));
    }
    // settings 浅合并（settings 键可含 public；null 值删除键）
    let mut settings = old_map
        .get("settings")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if let Value::Object(existing) = &mut settings {
        if let Value::Object(patch) = &body.settings {
            for (k, v) in patch {
                if v.is_null() {
                    existing.remove(k);
                } else {
                    existing.insert(k.clone(), v.clone());
                }
            }
        }
        if let Some(p) = body.public {
            existing.insert("public".to_string(), Value::Bool(p));
        }
    }
    record.insert("settings".to_string(), settings);

    match crate::trigger_crud::update_with_triggers(
        pool.get_ref(),
        "zc_id_empl-agent",
        old_map
            .get("id")
            .and_then(|v| v.as_i64())
            .unwrap_or_default(),
        record,
        &old_map,
        None,
    )
    .await
    {
        Ok(result_map) => {
            let code = result_map
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or(&code)
                .to_string();
            let notice = result_map
                .get("notice")
                .and_then(|v| v.as_str())
                .map(String::from);
            let color = result_map
                .get("t_color_")
                .and_then(|v| v.as_str())
                .map(String::from);
            let settings = result_map.get("settings").cloned();
            success(row_to_response(code, notice, color, settings, false))
        }
        Err(e) => {
            common::telemetry::error!("admin agents: patch '{}' failed: {}", code, e);
            internal_error("DB_ERROR", GENERIC_INTERNAL_MSG)
        }
    }
}

/// DELETE /api/chat-sessions/admin/agents/{code} — 软删（deleted_at）
pub async fn delete_agent(
    pool: web::Data<PgPool>,
    state: web::Data<gateway_sso::AuthState>,
    req: HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    if let Err(resp) = require_admin_id(&req, pool.get_ref(), state.get_ref()).await {
        return resp;
    }
    let code = path.into_inner();
    let rows_affected = sqlx::query(
        r#"UPDATE isahl."zc_id_empl-agent"
           SET deleted_at = NOW()
           WHERE code = $1 AND deleted_at IS NULL"#,
    )
    .bind(&code)
    .execute(pool.get_ref())
    .await;
    match rows_affected {
        Ok(res) if res.rows_affected() > 0 => HttpResponse::NoContent().finish(),
        Ok(_) => bad_request("AGENT_NOT_FOUND", &format!("agent '{}' 不存在", code)),
        Err(e) => {
            common::telemetry::error!("admin agents: delete '{}' failed: {}", code, e);
            internal_error("DB_ERROR", GENERIC_INTERNAL_MSG)
        }
    }
}
