use ai_agent::router::RoutingLevel;
use ai_agent::tools::ToolCall;
use ai_agent::*;
use std::collections::HashMap;

/// Test that AgentConfig default values are sensible
#[test]
fn test_agent_config_default() {
    let cfg = AgentConfig::default();
    assert!(cfg.code.is_empty());
    assert!(cfg.user_selectable);
    assert_eq!(cfg.sort_order, 0);
    assert_eq!(cfg.max_execution_steps, 5);
    assert_eq!(cfg.icon.as_str(), "Bot");
    assert_eq!(cfg.category.as_str(), "general");
}

#[test]
fn test_agent_result_new() {
    let result = AgentResult::new("test-agent", "Hello, world!", None, false, vec![]);
    assert_eq!(result.agent_code, "test-agent");
    assert_eq!(result.content, "Hello, world!");
    assert!(!result.requires_input);
    assert!(result.structured.is_none());
    assert!(result.suggested_actions.is_empty());
    assert_eq!(result.confidence, 0.9);
}

#[test]
fn test_agent_result_serde() {
    let result = AgentResult {
        content: "analysis complete".to_string(),
        structured: Some(serde_json::json!({"score": 95})),
        requires_input: false,
        suggested_actions: vec!["review".to_string(), "approve".to_string()],
        agent_code: "data-analysis".to_string(),
        confidence: 0.85,
        token_usage: Some(TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
        }),
        routing_info: Some(RoutingInfo {
            agent_code: "data-analysis".to_string(),
            confidence: 0.85,
            reason: "keyword match".to_string(),
            level: "l1_rule".to_string(),
        }),
    };

    let json = serde_json::to_string(&result).unwrap();
    let back: AgentResult = serde_json::from_str(&json).unwrap();
    assert_eq!(back.content, "analysis complete");
    assert_eq!(back.confidence, 0.85);
    assert_eq!(back.token_usage.unwrap().total_tokens, 150);
}

#[test]
fn test_agent_capability_serde() {
    let caps = vec![
        AgentCapability::MultimodalUnderstanding,
        AgentCapability::DataAnalysis,
        AgentCapability::FormFilling,
        AgentCapability::FlowDesign,
        AgentCapability::Simulation,
        AgentCapability::DocumentAutomation,
        AgentCapability::GeneralConversation,
    ];
    for cap in &caps {
        let json = serde_json::to_string(cap).unwrap();
        let back: AgentCapability = serde_json::from_str(&json).unwrap();
        assert_eq!(*cap, back);
    }
}

#[test]
fn test_execution_target_serde() {
    assert_eq!(
        serde_json::to_string(&ExecutionTarget::Backend).unwrap(),
        "\"backend\""
    );
    assert_eq!(
        serde_json::to_string(&ExecutionTarget::Frontend).unwrap(),
        "\"frontend\""
    );
}

#[test]
fn test_tool_choice_serde() {
    assert_eq!(
        serde_json::to_string(&ToolChoice::Auto).unwrap(),
        "\"auto\""
    );
    assert_eq!(
        serde_json::to_string(&ToolChoice::Required).unwrap(),
        "\"required\""
    );
    assert_eq!(
        serde_json::to_string(&ToolChoice::None).unwrap(),
        "\"none\""
    );
}

#[test]
fn test_db_access_level_serde() {
    let levels = vec![
        DbAccessLevel::None,
        DbAccessLevel::ReadOnly,
        DbAccessLevel::SchemaRestricted,
        DbAccessLevel::ReadWrite,
    ];
    // Verify serde serialization for each level
    for l in &levels {
        let json = serde_json::to_string(l).unwrap();
        let back: DbAccessLevel = serde_json::from_str(&json).unwrap();
        // Re-serialize both to compare strings since PartialEq is not derived
        let original_json = serde_json::to_string(l).unwrap();
        let back_json = serde_json::to_string(&back).unwrap();
        assert_eq!(original_json, back_json);
    }
}

#[test]
fn test_routing_decision_construction() {
    let decision = RoutingDecision {
        agent_code: "data-analysis".to_string(),
        confidence: 0.85,
        reason: "keyword: analyze".to_string(),
        level: RoutingLevel::L1Rule,
    };
    // Verify serde serialization works
    let json = serde_json::to_string(&decision.level).unwrap();
    assert_eq!(json, "\"l1_rule\"");
}

#[test]
fn test_routing_decision_serde() {
    let decision = RoutingDecision {
        agent_code: "simulation".to_string(),
        confidence: 0.72,
        reason: "user mentioned simulation".to_string(),
        level: RoutingLevel::L2Llm,
    };
    let json = serde_json::to_string(&decision).unwrap();
    let back: RoutingDecision = serde_json::from_str(&json).unwrap();
    assert_eq!(back.agent_code, "simulation");
}

#[test]
fn test_agent_registry_new() {
    let registry = AgentRegistry::new();
    // Default registry should have built-in agents
    assert!(!registry.codes().is_empty());
    assert!(registry.get("general").is_some());
}

#[test]
fn test_agent_registry_from_map() {
    let agents: HashMap<String, Box<dyn Agent>> = HashMap::new();
    let registry = AgentRegistry::from_map(agents);
    // Empty map should produce empty codes
    assert!(registry.codes().is_empty());
}

// ============================================
// RoutingStrategy Tests
// ============================================

struct MockAgent {
    config: AgentConfig,
    keywords: Vec<String>,
    patterns: Vec<regex::Regex>,
}

impl MockAgent {
    fn new(code: &str) -> Self {
        let config = AgentConfig {
            code: code.to_string(),
            name: format!("{}-name", code),
            routing_weights: RoutingWeights {
                keyword_match: 1.0,
                pattern_match: 2.0,
                page_context_bonus: 3.0,
                max_possible_score: 2.0,
                threshold: 0.5,
            },
            ..Default::default()
        };
        Self {
            config,
            keywords: vec![],
            patterns: vec![],
        }
    }

    fn with_keywords(mut self, kws: Vec<&str>) -> Self {
        self.keywords = kws.into_iter().map(String::from).collect();
        self
    }

    fn with_pattern(mut self, pat: &str) -> Self {
        self.patterns.push(regex::Regex::new(pat).unwrap());
        self
    }
}

#[async_trait::async_trait]
impl Agent for MockAgent {
    fn config(&self) -> &AgentConfig {
        &self.config
    }

    fn intent_keywords(&self) -> Vec<String> {
        self.keywords.clone()
    }

    fn intent_patterns(&self) -> Vec<regex::Regex> {
        self.patterns.clone()
    }
}

fn mock_registry_with(agents: Vec<MockAgent>) -> AgentRegistry {
    let mut map: HashMap<String, Box<dyn Agent>> = HashMap::new();
    for agent in agents {
        let code = agent.config.code.clone();
        map.insert(code, Box::new(agent));
    }
    AgentRegistry::from_map(map)
}

#[tokio::test]
async fn test_keyword_strategy_matches_by_keyword() {
    let registry = mock_registry_with(vec![
        MockAgent::new("form_filling").with_keywords(vec!["填单", "报销"]),
        MockAgent::new("data_analysis").with_keywords(vec!["分析", "统计"]),
    ]);

    let strategy = KeywordStrategy;
    let ctx = RoutingContext {
        user_message: "帮我填报销单".to_string(),
        page_context: None,
        conversation_history: vec![],
        suggested_agent: None,
        locale: "zh-CN".to_string(),
    };

    let decision = strategy.route(&ctx, &registry, None).await.unwrap();
    assert_eq!(decision.agent_code, "form_filling");
    assert!(decision.confidence >= 0.5);
    assert!(matches!(decision.level, RoutingLevel::L1Rule));
}

#[tokio::test]
async fn test_keyword_strategy_matches_by_pattern() {
    let registry = mock_registry_with(vec![
        MockAgent::new("form_filling").with_pattern(r"(?i)报销|发票")
    ]);

    let strategy = KeywordStrategy;
    let ctx = RoutingContext {
        user_message: "我要报销差旅费".to_string(),
        page_context: None,
        conversation_history: vec![],
        suggested_agent: None,
        locale: "zh-CN".to_string(),
    };

    let decision = strategy.route(&ctx, &registry, None).await.unwrap();
    assert_eq!(decision.agent_code, "form_filling");
    assert!(decision.reason.contains("正则"));
}

#[tokio::test]
async fn test_keyword_strategy_page_context_bonus() {
    let registry = mock_registry_with(vec![
        MockAgent::new("data_analysis").with_keywords(vec!["分析"])
    ]);

    let strategy = KeywordStrategy;
    let ctx = RoutingContext {
        user_message: "看看数据".to_string(),
        page_context: Some(serde_json::json!({"page": "报表中心"})),
        conversation_history: vec![],
        suggested_agent: Some("data_analysis".to_string()),
        locale: "zh-CN".to_string(),
    };

    let decision = strategy.route(&ctx, &registry, None).await.unwrap();
    assert_eq!(decision.agent_code, "data_analysis");
    assert!(decision.reason.contains("页面上下文"));
}

#[tokio::test]
async fn test_keyword_strategy_no_match_returns_none() {
    let registry = mock_registry_with(vec![
        MockAgent::new("form_filling").with_keywords(vec!["填单"])
    ]);

    let strategy = KeywordStrategy;
    let ctx = RoutingContext {
        user_message: "今天天气怎么样".to_string(),
        page_context: None,
        conversation_history: vec![],
        suggested_agent: None,
        locale: "zh-CN".to_string(),
    };

    assert!(strategy.route(&ctx, &registry, None).await.is_none());
}

#[tokio::test]
async fn test_fallback_strategy_always_returns_general() {
    let registry = AgentRegistry::new();
    let strategy = FallbackStrategy;
    let ctx = RoutingContext {
        user_message: "anything".to_string(),
        page_context: None,
        conversation_history: vec![],
        suggested_agent: None,
        locale: "zh-CN".to_string(),
    };

    let decision = strategy.route(&ctx, &registry, None).await.unwrap();
    assert_eq!(decision.agent_code, "general");
    assert!(matches!(decision.level, RoutingLevel::Fallback));
}

#[tokio::test]
async fn test_composite_strategy_uses_first_confident() {
    let registry = mock_registry_with(vec![
        MockAgent::new("form_filling").with_keywords(vec!["填单"])
    ]);

    // 当 KeywordStrategy 命中时，不应走到 FallbackStrategy
    let composite = CompositeStrategy::default_chain();
    let ctx = RoutingContext {
        user_message: "帮我填单".to_string(),
        page_context: None,
        conversation_history: vec![],
        suggested_agent: None,
        locale: "zh-CN".to_string(),
    };

    let decision = composite.route(&ctx, &registry, None).await.unwrap();
    assert_eq!(decision.agent_code, "form_filling");
    assert!(matches!(decision.level, RoutingLevel::L1Rule));
}

#[tokio::test]
async fn test_composite_strategy_falls_through_to_fallback() {
    let registry = mock_registry_with(vec![
        MockAgent::new("form_filling").with_keywords(vec!["填单"])
    ]);

    let composite = CompositeStrategy::default_chain();
    let ctx = RoutingContext {
        user_message: " unrelated ".to_string(),
        page_context: None,
        conversation_history: vec![],
        suggested_agent: None,
        locale: "zh-CN".to_string(),
    };

    let decision = composite.route(&ctx, &registry, None).await.unwrap();
    assert_eq!(decision.agent_code, "general");
    assert!(matches!(decision.level, RoutingLevel::Fallback));
}

#[tokio::test]
async fn test_agent_router_api_stable() {
    let registry = mock_registry_with(vec![
        MockAgent::new("form_filling").with_keywords(vec!["填单"])
    ]);

    let router = AgentRouter::new(registry);
    let ctx = RoutingContext {
        user_message: "帮我填单".to_string(),
        page_context: None,
        conversation_history: vec![],
        suggested_agent: None,
        locale: "zh-CN".to_string(),
    };

    let decision = router.route(&ctx, None).await;
    assert_eq!(decision.agent_code, "form_filling");
}

#[test]
fn test_tool_call_serde() {
    let call = ToolCall {
        id: "call_abc".to_string(),
        name: "search_db".to_string(),
        arguments: serde_json::json!({"table": "products"}),
    };
    let json = serde_json::to_string(&call).unwrap();
    let back: ToolCall = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "search_db");
}

#[test]
fn test_tool_definition_serde() {
    let def = ToolDefinition {
        name: "search_products".to_string(),
        description: "Search products by name".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            }
        }),
        execution_target: ExecutionTarget::Backend,
    };
    let json = serde_json::to_string(&def).unwrap();
    let back: ToolDefinition = serde_json::from_str(&json).unwrap();
    assert_eq!(back.description, "Search products by name");
    assert!(matches!(back.execution_target, ExecutionTarget::Backend));
}

// ============================================
// ToolOrchestrator Tests
// ============================================

use ai_agent::agents::tool_orchestrator::{
    FakeLlmAdapter, FakeToolAdapter, ToolOrchestrator, ToolRunContext,
};
use ai_agent::tools::ToolResult;

#[tokio::test]
async fn test_tool_orchestrator_returns_text_on_first_turn() {
    let llm = Box::new(FakeLlmAdapter::new(vec![llm::LlmResponse::Text(
        "final answer".to_string(),
    )]));
    let tools = Box::new(FakeToolAdapter::new(vec![], HashMap::new()));
    let orchestrator = ToolOrchestrator::new(llm, tools);
    let ctx = ToolRunContext {
        initial_prompt: "hello".to_string(),
        session_id: 1,
        user_id: None,
        allowed_schemas: vec![],
    };
    let result = orchestrator.run(&ctx).await.unwrap();
    assert_eq!(result.final_text, "final answer");
    assert!(!result.truncated);
    assert_eq!(result.steps_taken, 1);
}

#[tokio::test]
async fn test_tool_orchestrator_truncates_at_max_steps() {
    let llm = Box::new(FakeLlmAdapter::new(vec![
        llm::LlmResponse::ToolCalls(vec![llm::ToolCall {
            id: "1".to_string(),
            name: "query".to_string(),
            arguments: serde_json::json!({}),
        }]),
        llm::LlmResponse::ToolCalls(vec![llm::ToolCall {
            id: "2".to_string(),
            name: "query".to_string(),
            arguments: serde_json::json!({}),
        }]),
        llm::LlmResponse::ToolCalls(vec![llm::ToolCall {
            id: "3".to_string(),
            name: "query".to_string(),
            arguments: serde_json::json!({}),
        }]),
    ]));
    let mut results = HashMap::new();
    results.insert(
        "query".to_string(),
        ToolResult {
            tool_call_id: "1".to_string(),
            name: "query".to_string(),
            success: true,
            output: serde_json::json!("ok"),
            error: None,
        },
    );
    let tools = Box::new(FakeToolAdapter::new(
        vec![ToolDefinition {
            name: "query".to_string(),
            description: "query".to_string(),
            parameters: serde_json::json!({}),
            execution_target: ExecutionTarget::Backend,
        }],
        results,
    ));
    let orchestrator = ToolOrchestrator::new(llm, tools).with_max_steps(2);
    let ctx = ToolRunContext {
        initial_prompt: "hello".to_string(),
        session_id: 1,
        user_id: None,
        allowed_schemas: vec![],
    };
    let result = orchestrator.run(&ctx).await.unwrap();
    assert!(result.truncated);
    assert_eq!(result.steps_taken, 2);
    assert_eq!(result.tool_calls.len(), 2);
}
