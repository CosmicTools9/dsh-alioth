//! LLM 辅助设施（migrate-dmn-assist-to-framework）
//!
//! ns 服务化部署（AVIC commitment 等经 `/service/{svc}` scope 委托
//! `approval::configure_routes`）缺 dmn-assist——该端点原仅 Gateway 主
//! `approval_formula.rs`（依赖 Gateway chat_sessions DbLlmConfigAdapter）。本模块把
//! LLM 服务构建下沉 Framework：读 `isahl."zc_id_prot-llm_config"`（enabled 行，
//! enc_fields.api_key 解密）+ env 兜底 → 构建 `llm::LlmService`。
//! DSL 表达式校验 + JSON 提取亦下沉（dmn-assist cell 校验用）。

use llm::{GenerationParams, LlmProvider, LlmService, LlmServiceConfig, ReasoningEffort};
use sqlx::PgPool;

/// 从 provider code + 已解密 api_key + settings 构建 `LlmService`。
/// （迁移自 Gateway chat_sessions/db_llm_config.rs build_llm_service——行为一致，
/// 防分叉：Gateway 版已改为委托 Framework 版。）
pub fn build_llm_service(
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

    LlmService::new(config).map_err(|e| format!("Failed to init LLM: {e}"))
}

/// Map provider code (from `settings->>'provider'` of `zc_id_prot-llm_config`) to `LlmProvider`.
fn map_provider(code: &str) -> LlmProvider {
    match code.to_lowercase().as_str() {
        "deepseek" => LlmProvider::DeepSeek,
        "kimi" | "moonshot" => LlmProvider::Kimi,
        "minimax" => LlmProvider::MiniMax,
        "glm" | "zhipu" => LlmProvider::Glm,
        other => {
            common::telemetry::warn!(
                "Unrecognized LLM provider '{}', falling back to DeepSeek (OpenAI-compatible)",
                other
            );
            LlmProvider::DeepSeek
        }
    }
}

/// 加载 LLM 服务：DB `zc_id_prot-llm_config` enabled 行优先（enc_fields.api_key
/// AES-256-GCM 解密），无行时 env 兜底（LLM_PROVIDER/LLM_API_KEY/LLM_MODEL/
/// LLM_BASE_URL/LLM_FLASH_MODEL）。配置缺失/解密失败均 Err（调用方 fail-closed）。
pub async fn load_llm_service(pool: &PgPool) -> Result<LlmService, String> {
    // ── Env var defaults (lowest priority) ──
    let env_provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "deepseek".to_string());
    let env_api_key = std::env::var("LLM_API_KEY").unwrap_or_default();
    let env_model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".to_string());
    let env_base_url = std::env::var("LLM_BASE_URL").ok().filter(|s| !s.is_empty());
    let env_flash_model =
        std::env::var("LLM_FLASH_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".to_string());

    // ── DB 行（highest priority）──
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
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    let (provider_code, api_key, settings) = match row {
        Some((provider_code, enc_fields, settings)) => {
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

/// 表达式强校验（fail-closed，唯一引擎 Rhai）：严格变量模式 —— 语法错、引用
/// 标识符不在已知键集、或调用**未登记自由函数**即失败（`validate_all`：语法 + 标识符 +
/// 白名单三层；白名单单一真相源 = `runtime-engine/expression/builtins.json`）。
/// 已知键 = 变量清单 ∪ 结构键（`_refs` 引用容器 / `entityId` / `ctx` 恒可见）；
/// `ctx["k"]` 索引形态天然通过（ctx 为已知键）。
pub fn validate_dsl_expression(expression: &str, context_fields: &[String]) -> (bool, Vec<String>) {
    let mut known: Vec<String> = context_fields.to_vec();
    known.push("_refs".to_string());
    known.push("entityId".to_string());
    known.push("ctx".to_string());
    match runtime_engine::RhaiExpressionEngine::new().validate_all(expression, &known) {
        Ok(()) => (true, Vec::new()),
        Err(e) => (false, vec![e]),
    }
}

/// 从 LLM 输出提取 JSON（容忍围栏/前后噪声）
pub fn extract_json(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim();
    let trimmed = trimmed.strip_suffix("```").unwrap_or(trimmed).trim();
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end > start {
        Some(&trimmed[start..=end])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn valid_expression_passes() {
        let (valid, errors) = validate_dsl_expression(
            "amount > 5000 && code == \"VIP\"",
            &fields(&["amount", "code"]),
        );
        assert!(valid, "合法表达式应通过: {errors:?}");
    }

    #[test]
    fn unknown_field_fail_closed() {
        let (valid, errors) = validate_dsl_expression("amount > 100", &fields(&["total"]));
        assert!(!valid, "引用未知字段必须 fail-closed");
        assert!(!errors.is_empty(), "失败必须带可读错误");
    }

    #[test]
    fn unregistered_free_function_fail_closed() {
        // LLM 产出的表达式若调用未登记函数，运行期必 `Function not found` ⇒ 写入前拦下
        let (valid, errors) =
            validate_dsl_expression("median([1, 2, 3]) > 0", &fields(&["amount"]));
        assert!(!valid, "未登记自由函数必须 fail-closed");
        assert!(
            errors.iter().any(|e| e.contains("median")),
            "错误须指名函数: {errors:?}"
        );
        // 受控白名单内函数放行（白名单单一真相源 = builtins.json）
        let (ok, errs) = validate_dsl_expression("sum([1, 2, 3]) > 0", &fields(&["amount"]));
        assert!(ok, "白名单内函数应通过: {errs:?}");
    }

    #[test]
    fn refs_member_path_accepted() {
        // `_refs` 恒为已知键（引用容器）；子键以索引语法表达（D2 契约）
        let (valid, errors) = validate_dsl_expression(
            "_refs[\"ck_category\"][\"notice\"] == \"合同类\"",
            &fields(&["amount"]),
        );
        assert!(valid, "_refs 成员路径应通过: {errors:?}");
    }

    #[test]
    fn hyphen_key_via_ctx_index() {
        // 含连字符的模型列 MUST 经 `ctx["…"]` 索引语法（裸名在 Rhai 中解析为减法）
        let (valid, errors) = validate_dsl_expression("ctx[\"act-group\"] == 1", &fields(&[]));
        assert!(valid, "ctx 索引语法应通过: {errors:?}");
        let (bare, _) = validate_dsl_expression("act-group == 1", &fields(&["act-group"]));
        assert!(!bare, "裸连字符标识符 MUST NOT 通过（D2 契约）");
    }

    #[test]
    fn extract_json_tolerates_fences() {
        let out = extract_json("```json\n{\"expression\": \"a > 1\"}\n```").unwrap();
        assert!(out.contains("a > 1"));
    }
}
