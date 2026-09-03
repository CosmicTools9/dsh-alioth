//! LLM crate 单元测试
//!
//! 测试类型系统（配置、序列化、错误处理），不涉及实际 HTTP 调用。

use llm::types::recommended_max_tokens;
use llm::types::{
    GenerationParams, LlmProvider, LlmResponse, LlmServiceConfig, ReasoningEffort, ToolCall,
    ToolDefinition,
};

// ═══════════════════════════════════════════════════════════════════════════════
// GenerationParams
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_generation_params_default() {
    let params = GenerationParams::default();
    assert!((params.temperature - 1.0).abs() < f64::EPSILON);
    assert_eq!(params.max_tokens, llm::DEFAULT_MAX_TOKENS);
    assert!((params.top_p - 1.0).abs() < f64::EPSILON);
    assert!((params.frequency_penalty - 0.0).abs() < f64::EPSILON);
    assert!((params.presence_penalty - 0.0).abs() < f64::EPSILON);
    assert_eq!(params.reasoning_effort, ReasoningEffort::Medium);
}

#[test]
fn test_generation_params_serde() {
    let json = r#"{"temperature":0.5,"max_tokens":2048,"top_p":0.9,"frequency_penalty":0.1,"presence_penalty":0.2,"reasoning_effort":"high"}"#;
    let params: GenerationParams = serde_json::from_str(json).unwrap();
    assert!((params.temperature - 0.5).abs() < f64::EPSILON);
    assert_eq!(params.max_tokens, 2048);
    assert!((params.top_p - 0.9).abs() < f64::EPSILON);
    assert_eq!(params.reasoning_effort, ReasoningEffort::High);
}

#[test]
fn test_reasoning_effort_display() {
    assert_eq!(ReasoningEffort::Minimal.to_string(), "minimal");
    assert_eq!(ReasoningEffort::Low.to_string(), "low");
    assert_eq!(ReasoningEffort::Medium.to_string(), "medium");
    assert_eq!(ReasoningEffort::High.to_string(), "high");
    assert_eq!(ReasoningEffort::XHigh.to_string(), "xhigh");
}

#[test]
fn test_reasoning_effort_api_value() {
    // minimal/low → "low" (lowest API value)
    assert_eq!(ReasoningEffort::Minimal.as_api_value(), Some("low"));
    assert_eq!(ReasoningEffort::Low.as_api_value(), Some("low"));
    // medium → None (default; omit to preserve prefix cache)
    assert_eq!(ReasoningEffort::Medium.as_api_value(), None);
    // high/xhigh → "high" (highest API value)
    assert_eq!(ReasoningEffort::High.as_api_value(), Some("high"));
    assert_eq!(ReasoningEffort::XHigh.as_api_value(), Some("high"));
}

#[test]
fn test_reasoning_effort_serde() {
    let json = r#""xhigh""#;
    let effort: ReasoningEffort = serde_json::from_str(json).unwrap();
    assert_eq!(effort, ReasoningEffort::XHigh);
    assert_eq!(serde_json::to_string(&effort).unwrap(), r#""xhigh""#);
}

#[test]
fn test_reasoning_effort_serde_minimal() {
    let json = r#""minimal""#;
    let effort: ReasoningEffort = serde_json::from_str(json).unwrap();
    assert_eq!(effort, ReasoningEffort::Minimal);
}

#[test]
fn test_reasoning_effort_serde_snake_case() {
    let json = r#""low""#;
    let effort: ReasoningEffort = serde_json::from_str(json).unwrap();
    assert_eq!(effort, ReasoningEffort::Low);
}

#[test]
fn test_recommended_max_tokens() {
    assert_eq!(recommended_max_tokens("ontology_planning"), 16384);
    assert_eq!(recommended_max_tokens("code_generation"), 32768);
    assert_eq!(recommended_max_tokens("unknown"), 4096);
}

#[test]
fn test_generation_params_with_overrides() {
    let params = GenerationParams::default();
    let overridden =
        params.with_overrides(Some(0.1), Some(8192), Some("high"), Some("json_object"));
    assert!((overridden.temperature - 0.1).abs() < f64::EPSILON);
    assert_eq!(overridden.max_tokens, 8192);
    assert_eq!(overridden.reasoning_effort, ReasoningEffort::High);
    assert_eq!(overridden.response_format, Some("json_object".to_string()));
}
#[test]
fn test_generation_params_with_overrides_partial() {
    let params = GenerationParams::default();
    // Only override reasoning_effort, keep temp and tokens
    let overridden = params.with_overrides(None, None, Some("low"), None);
    assert!((overridden.temperature - 1.0).abs() < f64::EPSILON);
    assert_eq!(overridden.max_tokens, llm::DEFAULT_MAX_TOKENS);
    assert_eq!(overridden.reasoning_effort, ReasoningEffort::Low);
    assert_eq!(overridden.response_format, None);
}

// ═══════════════════════════════════════════════════════════════════════════════
// LlmProvider
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_llm_provider_serde_deepseek() {
    let json = r#""deep_seek""#;
    let provider: LlmProvider = serde_json::from_str(json).unwrap();
    assert_eq!(provider, LlmProvider::DeepSeek);
}

// ═══════════════════════════════════════════════════════════════════════════════
// ToolDefinition / ToolCall
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_tool_definition_serde() {
    let td = ToolDefinition {
        name: "search_web".to_string(),
        description: "Search the web for information".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            },
            "required": ["query"]
        }),
    };
    let json = serde_json::to_string(&td).unwrap();
    let parsed: ToolDefinition = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.name, "search_web");
    assert_eq!(parsed.description, "Search the web for information");
}

#[test]
fn test_tool_call_serde() {
    let tc = ToolCall {
        id: "call_001".to_string(),
        name: "search_web".to_string(),
        arguments: serde_json::json!({"query": "Rust LLM framework"}),
    };
    let json = serde_json::to_string(&tc).unwrap();
    let parsed: ToolCall = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.id, "call_001");
    assert_eq!(parsed.name, "search_web");
    assert_eq!(parsed.arguments["query"], "Rust LLM framework");
}

// ═══════════════════════════════════════════════════════════════════════════════
// LlmServiceConfig
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_llm_service_config_from_env_defaults() {
    // Clear env vars to test fallback values.
    for v in [
        "LLM_PROVIDER",
        "LLM_API_KEY",
        "LLM_MODEL",
        "LLM_BASE_URL",
        "LLM_TIMEOUT_SECONDS",
        "LLM_MAX_RETRIES",
        "LLM_TEMPERATURE",
        "LLM_MAX_TOKENS",
        "LLM_REASONING_EFFORT",
        "LLM_RESPONSE_FORMAT",
    ] {
        std::env::remove_var(v);
    }
    let config = LlmServiceConfig::from_env();
    assert_eq!(config.provider, LlmProvider::DeepSeek);
    assert_eq!(config.model, "deepseek-v4-pro");
    assert_eq!(config.timeout_seconds, 300);
    assert_eq!(config.max_retries, 3);
    assert!((config.generation_params.temperature - 1.0).abs() < f64::EPSILON);
    assert_eq!(config.generation_params.max_tokens, llm::DEFAULT_MAX_TOKENS);
}

#[test]
fn test_llm_service_config_from_env_deepseek() {
    std::env::set_var("LLM_PROVIDER", "deep_seek");
    std::env::remove_var("LLM_API_KEY");
    std::env::remove_var("LLM_MODEL");
    std::env::remove_var("LLM_BASE_URL");
    std::env::remove_var("LLM_TIMEOUT_SECONDS");
    std::env::remove_var("LLM_MAX_RETRIES");
    std::env::remove_var("LLM_TEMPERATURE");
    std::env::remove_var("LLM_MAX_TOKENS");

    let config = LlmServiceConfig::from_env();
    assert_eq!(config.provider, LlmProvider::DeepSeek);

    // cleanup
    std::env::remove_var("LLM_PROVIDER");
}

#[test]
fn test_llm_service_config_from_env_always_deepseek() {
    // from_env always returns DeepSeek — verify default
    let config = LlmServiceConfig::from_env();
    assert_eq!(config.provider, LlmProvider::DeepSeek);
}

// ═══════════════════════════════════════════════════════════════════════════════
// LlmResponse
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_llm_response_text() {
    let resp = LlmResponse::Text("Hello, world!".to_string());
    match &resp {
        LlmResponse::Text(t) => assert_eq!(t, "Hello, world!"),
        _ => panic!("Expected Text variant"),
    }
}

#[test]
fn test_llm_response_tool_calls() {
    let calls = vec![ToolCall {
        id: "1".to_string(),
        name: "fn".to_string(),
        arguments: serde_json::json!({}),
    }];
    let resp = LlmResponse::ToolCalls(calls);
    match &resp {
        LlmResponse::ToolCalls(tc) => assert_eq!(tc.len(), 1),
        _ => panic!("Expected ToolCalls variant"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// LlmError (provider 模块)
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_llm_error_display() {
    use llm::BackendError as LlmError;

    let err = LlmError::Transport("timeout".to_string());
    assert!(err.to_string().contains("timeout"));

    let err = LlmError::Parse("invalid json".to_string());
    assert!(err.to_string().contains("invalid json"));

    let err = LlmError::Config("missing key".to_string());
    assert!(err.to_string().contains("missing key"));
}
