//! POST /api/system-config/llm/test — LLM 配置连通性测试端点
//!
//! 语义对齐 Meta `/api/meta/llm-config/verify`（`LlmService::verify()` 探针）：
//! - `{ id }`：已存配置（服务端解密 enc_fields.api_key；body 携带的覆盖字段优先）
//! - `{ config }`：表单草稿（provider/base_url/api_key/model/flash_model 等明文）
//!
//! api_key 仅经请求体瞬时传输：不落盘、不出现在响应/日志。

use crate::api::chat_sessions::adapters::db_llm_config::build_llm_service;
use actix_web::HttpMessage;
use actix_web::{web, HttpRequest, HttpResponse};
use common::context::RequestContext;
use llm::LlmService;
use sqlx::PgPool;
use std::time::Instant;

#[derive(serde::Deserialize)]
pub struct LlmTestRequest {
    /// 已存配置行 id；与 config 二选一（同时给时 config 字段覆盖 id 行字段）
    pub id: Option<i64>,
    /// 表单草稿覆盖字段（键同 settings：provider/base_url/api_key/model/flash_model/timeout…）
    pub config: Option<serde_json::Value>,
}

#[derive(serde::Serialize)]
pub struct LlmTestResponse {
    pub success: bool,
    pub latency_ms: u64,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn success_response(latency_ms: u64, model: String) -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": LlmTestResponse {
            success: true,
            latency_ms,
            model,
            error: None,
        }
    }))
}

fn failure_response(message: String) -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": LlmTestResponse {
            success: false,
            latency_ms: 0,
            model: String::new(),
            error: Some(message),
        }
    }))
}

/// 读取已存配置行（settings/enc_fields/明文覆盖）。
/// 返回 (provider_code, api_key, settings)。
async fn load_saved_config(
    pool: &PgPool,
    id: i64,
    overrides: Option<&serde_json::Value>,
) -> Result<(String, String, serde_json::Value), String> {
    let row = sqlx::query_as::<_, (Option<String>, Option<serde_json::Value>, serde_json::Value)>(
        r#"SELECT settings->>'provider' as provider_code, enc_fields, COALESCE(settings, '{}'::jsonb)
           FROM isahl."zc_id_prot-llm_config"
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    let (provider_code, enc_fields, mut settings) =
        row.ok_or_else(|| format!("LLM config id {} not found", id))?;
    let provider_code = provider_code.unwrap_or_default();

    // 解密 api_key（enc: 前缀 → AES-256-GCM）
    let api_key = enc_fields
        .as_ref()
        .and_then(|c| c.get("api_key").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .map(|s| {
            if let Some(payload) = s.strip_prefix("enc:") {
                system_config::crypto::decrypt(payload).unwrap_or_else(|_| s.to_string())
            } else {
                s.to_string()
            }
        })
        .unwrap_or_default();

    // 表单覆盖字段合并进 settings（不落库）
    if let Some(ov) = overrides {
        if let Some(src) = ov.as_object() {
            if let Some(dst) = settings.as_object_mut() {
                for (k, v) in src {
                    dst.insert(k.clone(), v.clone());
                }
            }
        }
    }

    Ok((provider_code, api_key, settings))
}

/// 从草稿 config 提取 provider/api_key/settings。
fn draft_config(config: &serde_json::Value) -> (String, String, serde_json::Value) {
    let provider = config
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("deepseek")
        .to_string();
    let api_key = config
        .get("api_key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let settings = config.clone();
    (provider, api_key, settings)
}

/// POST /api/system-config/llm/test
pub async fn llm_test(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<LlmTestRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    // 管理员边界：与 /api/system-config 一致（PEP/NGAC 在路由层已校验）
    let _ctx = req
        .extensions()
        .get::<RequestContext>()
        .ok_or_else(|| actix_web::error::ErrorUnauthorized("Authentication required"))?;

    let body = body.into_inner();

    // ── 解析输入：id 行（服务端解密）或草稿 ──
    let (provider_code, api_key, settings) = match (body.id, body.config.as_ref()) {
        (Some(id), overrides) => match load_saved_config(&pool, id, overrides).await {
            Ok(v) => v,
            Err(e) => return Ok(failure_response(format!("[配置读取] {}", e))),
        },
        (None, Some(config)) => draft_config(config),
        (None, None) => {
            return Ok(failure_response(
                "[请求参数] 缺少 id 或 config（新建配置时需填写表单或已存配置 id）".to_string(),
            ))
        }
    };

    // ── 构建服务（共享 build_llm_service；探测固定 30s 超时/0 重试）──
    let service = match build_llm_service_with_probe(&provider_code, &api_key, &settings) {
        Ok(s) => s,
        Err(e) => return Ok(failure_response(format!("[服务构建] {}", e))),
    };

    // ── 探针 ──
    let start = Instant::now();
    match service.verify().await {
        Ok(()) => {
            let latency = start.elapsed().as_millis() as u64;
            let model = settings
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(success_response(latency, model))
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            // 只回显错误文本；LlmError 不含 Authorization/API key
            common::telemetry::warn!("LLM connectivity test failed ({}ms): {}", latency, e);
            Ok(failure_response(format!("[连接验证] {}", e)))
        }
    }
}

/// 探测用服务构建：build_llm_service + 固定探测参数（30s 超时、0 重试）。
/// 复用共享构建函数；探测语义与 Meta verify 一致（timeout=30, retries=0）。
fn build_llm_service_with_probe(
    provider_code: &str,
    api_key: &str,
    settings: &serde_json::Value,
) -> Result<LlmService, String> {
    let mut settings = settings.clone();
    if let Some(obj) = settings.as_object_mut() {
        obj.insert("timeout".into(), serde_json::json!(30));
        obj.insert("max_retries".into(), serde_json::json!(0));
    }
    build_llm_service(provider_code, api_key, Some(&settings), None)
}
