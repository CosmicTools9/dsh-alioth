use async_trait::async_trait;
use llm::{GenerationParams, LlmProvider, LlmService, LlmServiceConfig, ReasoningEffort};
use sqlx::PgPool;

use crate::api::chat_sessions::ports::LlmConfigPort;

/// 从 provider code + 已解密 api_key + settings 构建 `LlmService`。
/// `load_service`（DB 优先/env 兜底）与系统配置「测试连接」端点共用——
/// 测试端点从表单草稿或已存行（服务端解密）拿到同构输入后调用本函数，
/// 禁止复制配置解析逻辑（REUSE_FIRST）。
/// `env_fallback`：DB settings 缺失字段时的环境变量兜底（model/flash_model/base_url）。
pub(crate) fn build_llm_service(
    provider_code: &str,
    api_key: &str,
    settings: Option<&serde_json::Value>,
    env_fallback: Option<&std::collections::HashMap<String, String>>,
) -> Result<LlmService, String> {
    let provider = map_provider(provider_code);

    let env_get = |k: &str| env_fallback.and_then(|m| m.get(k)).map(|s| s.as_str());

    let model = settings
        .and_then(|s| s.get("model").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| env_get("model").map(String::from))
        .unwrap_or_else(|| provider.default_model().to_string());

    let base_url = settings
        .and_then(|s| s.get("base_url").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| env_get("base_url").map(String::from))
        .or_else(|| Some(provider.default_base_url().to_string()));

    let flash_model = settings
        .and_then(|s| s.get("flash_model").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| env_get("flash_model").map(String::from))
        .unwrap_or_else(|| provider.default_flash_model().to_string());

    let timeout_seconds = settings
        .and_then(|s| s.get("timeout").and_then(|v| v.as_i64()))
        .unwrap_or(120) as u64;

    let max_retries = settings
        .and_then(|s| s.get("max_retries").and_then(|v| v.as_i64()))
        .unwrap_or(2) as u32;

    let temperature = settings
        .and_then(|s| s.get("temperature").and_then(|v| v.as_f64()))
        .unwrap_or(1.0);
    let max_tokens = settings
        .and_then(|s| s.get("max_tokens").and_then(|v| v.as_i64()))
        .unwrap_or(4096) as u64;
    let top_p = settings
        .and_then(|s| s.get("top_p").and_then(|v| v.as_f64()))
        .unwrap_or(1.0);
    let frequency_penalty = settings
        .and_then(|s| s.get("frequency_penalty").and_then(|v| v.as_f64()))
        .unwrap_or(0.0);
    let presence_penalty = settings
        .and_then(|s| s.get("presence_penalty").and_then(|v| v.as_f64()))
        .unwrap_or(0.0);

    let generation_params = GenerationParams {
        temperature,
        max_tokens,
        top_p,
        frequency_penalty,
        presence_penalty,
        reasoning_effort: ReasoningEffort::Medium,
        response_format: None,
        thinking: None,
        service_tier: None,
        reasoning_split: None,
    };

    if api_key.is_empty() {
        return Err(
            "LLM_API_KEY not configured. Add an LLM provider in System Config > LLM or set the LLM_API_KEY environment variable."
                .to_string(),
        );
    }

    let config = LlmServiceConfig {
        provider,
        api_key: api_key.to_string(),
        model,
        base_url,
        flash_model,
        timeout_seconds,
        max_retries,
        generation_params,
        roles: Default::default(),
    };

    LlmService::new(config).map_err(|e| format!("Failed to init LLM: {}", e))
}

/// Map provider code (from `settings->>'provider'` of `zc_id_prot-llm_config`) to `LlmProvider`.
/// Providers: openai, anthropic, kimi, deepseek, glm, custom
fn map_provider(code: &str) -> LlmProvider {
    match code.to_lowercase().as_str() {
        "deepseek" => LlmProvider::DeepSeek,
        "kimi" | "moonshot" => LlmProvider::Kimi,
        "minimax" => LlmProvider::MiniMax,
        "glm" | "zhipu" => LlmProvider::Glm,
        // OpenAI-compatible: openai, custom, anthropic → use DeepSeek as generic provider
        other => {
            common::telemetry::warn!(
                "Unrecognized LLM provider '{}', falling back to DeepSeek (OpenAI-compatible)",
                other
            );
            LlmProvider::DeepSeek
        }
    }
}

pub struct DbLlmConfigAdapter {
    pool: PgPool,
}

impl DbLlmConfigAdapter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl LlmConfigPort for DbLlmConfigAdapter {
    async fn load_service(&self) -> Result<LlmService, String> {
        // ── Env var defaults (lowest priority) ──
        let env_provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "deepseek".to_string());
        let env_api_key = std::env::var("LLM_API_KEY").unwrap_or_default();
        let env_model =
            std::env::var("LLM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".to_string());
        let env_base_url = std::env::var("LLM_BASE_URL").ok().filter(|s| !s.is_empty());
        let env_flash_model =
            std::env::var("LLM_FLASH_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".to_string());

        // ── Read from live table `zc_id_prot-llm_config` (highest priority) ──
        // settings 内嵌 enabled/is_default/provider；敏感 api_key 存 enc_fields。
        // provider 读 settings->>'provider'（`_t_` 是 lifecycle 自动维度列，业务禁止使用）。
        let row = sqlx::query_as::<
            _,
            (
                Option<String>,
                Option<serde_json::Value>,
                Option<serde_json::Value>,
            ),
        >(
            r#"SELECT settings->>'provider' as provider_code, enc_fields, settings
               FROM isahl."zc_id_prot-llm_config"
               WHERE (settings->>'enabled')::boolean IS NOT FALSE AND deleted_at IS NULL
               ORDER BY (settings->>'is_default')::boolean DESC, updated_at DESC
               LIMIT 1"#,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;
        // ── Resolve: DB 行优先（解密 enc_fields.api_key），无行时 env 兜底 ──
        let (provider_code, api_key, settings) = match row {
            Some((provider_code, enc_fields, settings)) => {
                // api_key from enc_fields.api_key (AES-256-GCM 加密，enc: 前缀)
                let ak = enc_fields
                    .as_ref()
                    .and_then(|c| c.get("api_key").and_then(|v| v.as_str()))
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        if let Some(payload) = s.strip_prefix("enc:") {
                            system_config::crypto::decrypt(payload).unwrap_or_else(|_| {
                                common::telemetry::warn!("llm: api_key 解密失败（按明文处理）");
                                s.to_string()
                            })
                        } else {
                            s.to_string()
                        }
                    })
                    .unwrap_or(env_api_key.clone());
                (provider_code.unwrap_or(env_provider), ak, settings)
            }
            None => (env_provider, env_api_key, None),
        };

        // ── 共享构建（settings 缺失字段：env 兜底 → provider 默认值）──
        let mut env_fallback = std::collections::HashMap::new();
        env_fallback.insert("model".to_string(), env_model);
        env_fallback.insert("flash_model".to_string(), env_flash_model);
        if let Some(bu) = env_base_url {
            env_fallback.insert("base_url".to_string(), bu);
        }

        build_llm_service(
            &provider_code,
            &api_key,
            settings.as_ref(),
            Some(&env_fallback),
        )
    }
}
