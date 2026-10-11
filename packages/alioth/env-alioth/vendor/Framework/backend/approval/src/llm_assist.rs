//! LLM 辅助设施（migrate-dmn-assist-to-framework；consolidate-ai-chat-chain 4.0 收敛为单实现）
//!
//! 本模块是**唯一**的 LLM 服务装配实现：读 `isahl."zc_id_prot-llm_config"`
//! （生效行经 `common::config_resolution` 解析，enc_fields.api_key AES-256-GCM 解密），
//! 加 env 兜底后构建 `llm::LlmService`（含 model/flash_model 派生的角色映射与
//! 「未配置」哨兵）。
//! 消费方：Gateway `chat_sessions::adapters::db_llm_config`（`DbLlmConfigAdapter::load_service`
//! 委托本模块）与 ns 服务化部署（AVIC commitment 等经 `/service/{svc}` scope 委托
//! `approval::configure_routes`）的 dmn-assist。MUST NOT 在别处复制装配逻辑。
//! DSL 表达式校验 + JSON 提取亦在此（dmn-assist cell 校验用）。

use llm::{GenerationParams, LlmProvider, LlmService, LlmServiceConfig, ReasoningEffort};
use sqlx::PgPool;

/// 「LLM 未配置」内部哨兵前缀（fix-llm-unconfigured-visibility）。
///
/// 只在**进程内**错误串上流转，供端点受理侧分类为 `LLM_NOT_CONFIGURED`；
/// MUST NOT 进入任何用户可见文本——落库/回响应前用 `visible_message()` 剥离。
/// 配置缺失是用户可自行修复的前提缺失，与 DB/SQL 内部错误（`chat-ai-error-sanitized`
/// 的泛化面）不同，故以哨兵显式区分。
pub const LLM_UNCONFIGURED_MARKER: &str = "llm-unconfigured:";

/// 是否为「LLM 未配置」类错误（仅哨兵判定，不做文案匹配）。
pub fn is_unconfigured(err: &str) -> bool {
    err.starts_with(LLM_UNCONFIGURED_MARKER)
}

/// 剥离内部哨兵 —— 用户可见文本入口（用户可见串 MUST NOT 含哨兵）。
pub fn visible_message(err: &str) -> &str {
    err.strip_prefix(LLM_UNCONFIGURED_MARKER)
        .unwrap_or(err)
        .trim()
}

/// 从 provider code + 已解密 api_key + settings 构建 `LlmService`（唯一实现）。
/// `load_llm_service`（DB 优先/env 兜底）与系统配置「测试连接」端点共用——
/// 测试端点从表单草稿或已存行（服务端解密）拿到同构输入后调用本函数，
/// 禁止复制配置解析逻辑（REUSE_FIRST）。
/// `env_fallback`：DB settings 缺失字段时的环境变量兜底（model/flash_model/base_url）。
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
        return Err(format!(
            "{} LLM_API_KEY not configured. Add an LLM provider in System Config > LLM or set the LLM_API_KEY environment variable.",
            LLM_UNCONFIGURED_MARKER
        ));
    }

    // roles 由 model / flash_model 派生——必须在结构体字面量移动两者**之前**计算
    // （2026-09-10 修复：HEAD 上 roles 内联在字面量里，导致 model/flash_model 先被
    // 移入字段、再被借用 → E0382，整个 alioth-gateway lib 无法编译）。
    let roles = LlmServiceConfig::resolve_roles(&model, &flash_model);
    let config = LlmServiceConfig {
        provider,
        api_key: api_key.to_string(),
        model,
        base_url,
        flash_model,
        timeout_seconds,
        max_retries,
        generation_params,
        roles,
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
/// LLM_BASE_URL/LLM_FLASH_MODEL）。配置缺失/解密失败均 Err（调用方 fail-closed）；
/// 「未配置」类错误串带 `LLM_UNCONFIGURED_MARKER` 哨兵，供调用方分类
/// （`is_unconfigured`），回用户可见文本前 MUST `visible_message` 剥离。
pub async fn load_llm_service(pool: &PgPool) -> Result<LlmService, String> {
    // ── Env var defaults (lowest priority) ──
    let env_provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "deepseek".to_string());
    let env_api_key = std::env::var("LLM_API_KEY").unwrap_or_default();
    let env_model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".to_string());
    let env_base_url = std::env::var("LLM_BASE_URL").ok().filter(|s| !s.is_empty());
    let env_flash_model =
        std::env::var("LLM_FLASH_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".to_string());

    // ── DB 行（highest priority；生效解析唯一实现 = `common::config_resolution`）──
    let active = common::config_resolution::resolve_active(
        pool,
        common::config_resolution::ConfigFamily::Llm,
        &system_config::crypto::decrypt_prefixed,
    )
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    let (provider_code, api_key, settings) = match active {
        Some(cfg) => {
            let ak = cfg
                .credentials
                .get("api_key")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from)
                .unwrap_or(env_api_key.clone());
            (cfg.provider.unwrap_or(env_provider), ak, Some(cfg.settings))
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

    /// 4.0：缺 api_key ⇒ 错误串带未配置哨兵（端点可分类为 `LLM_NOT_CONFIGURED`）。
    #[test]
    fn unconfigured_error_carries_sentinel() {
        let err = match build_llm_service("deepseek", "", None, None) {
            Ok(_) => panic!("缺 api_key MUST fail-closed"),
            Err(e) => e,
        };
        assert!(err.starts_with(LLM_UNCONFIGURED_MARKER), "{err}");
        assert!(is_unconfigured(&err), "{err}");
        assert_eq!(
            visible_message(&err),
            "LLM_API_KEY not configured. Add an LLM provider in System Config > LLM or set the LLM_API_KEY environment variable."
        );
        // 无哨兵文本原样透传（不误伤）
        assert!(!is_unconfigured("DB error: connection refused"));
        assert_eq!(visible_message("plain failure"), "plain failure");
    }

    /// 4.0：`roles` 由 model/flash_model 派生（`resolve_roles`），MUST NOT 为 `Default` 空表。
    #[test]
    fn roles_derive_from_model_and_flash_model() {
        let settings = serde_json::json!({
            "model": "test-main-model",
            "flash_model": "test-flash-model",
        });
        let service = build_llm_service("deepseek", "sk-test", Some(&settings), None)
            .expect("完整输入应构建成功");
        let config = service.config();
        assert_eq!(config.model, "test-main-model");
        assert_eq!(config.flash_model, "test-flash-model");
        let expected = LlmServiceConfig::resolve_roles(&config.model, &config.flash_model);
        assert_eq!(
            config.roles.len(),
            expected.len(),
            "roles 必须由 model/flash_model 派生，不得为 Default 空表"
        );
        assert!(!config.roles.is_empty(), "Default = 空表 ⇒ 角色映射失效");
        for role in [
            llm::ModelRole::Slow,
            llm::ModelRole::Task,
            llm::ModelRole::Vision,
            llm::ModelRole::Advisor,
            llm::ModelRole::Default,
        ] {
            let got = config.roles.get(&role).expect("resolve_roles 补齐全部角色");
            let want = expected.get(&role).expect("resolve_roles 补齐全部角色");
            assert_eq!(got.model, want.model, "角色 {role:?} 的模型名必须派生");
        }
    }
}
