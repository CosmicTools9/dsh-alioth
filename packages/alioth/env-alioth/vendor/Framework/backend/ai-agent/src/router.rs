use crate::registry::AgentRegistry;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ============================================
// Domain Types
// ============================================

/// 路由上下文
#[derive(Debug, Clone)]
pub struct RoutingContext {
    pub user_message: String,
    pub page_context: Option<serde_json::Value>,
    pub conversation_history: Vec<(String, String)>,
    pub suggested_agent: Option<String>,
    pub locale: String,
}

/// 路由决策结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub agent_code: String,
    pub confidence: f64,
    pub reason: String,
    pub level: RoutingLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingLevel {
    L1Rule,
    L2Llm,
    Fallback,
}

// ============================================
// RoutingStrategy Seam
// ============================================

/// 路由策略接口。
///
/// 每个策略接收相同的上下文和注册表，独立决定是否返回 RoutingDecision。
/// CompositeStrategy 将多个策略链式组合，直到某个策略返回 confident 结果。
#[async_trait]
pub trait RoutingStrategy: Send + Sync {
    async fn route(
        &self,
        ctx: &RoutingContext,
        registry: &AgentRegistry,
        llm: Option<&llm::LlmService>,
    ) -> Option<RoutingDecision>;
}

// ============================================
// KeywordStrategy — L1 规则路由
// ============================================

/// 基于关键词 + 正则 + 页面上下文建议的规则路由。
///
/// 评分权重从 AgentConfig.routing_weights 读取，不再硬编码。
pub struct KeywordStrategy;

#[async_trait]
impl RoutingStrategy for KeywordStrategy {
    async fn route(
        &self,
        ctx: &RoutingContext,
        registry: &AgentRegistry,
        _llm: Option<&llm::LlmService>,
    ) -> Option<RoutingDecision> {
        let msg_lower = ctx.user_message.to_lowercase();
        let mut scores: Vec<(String, f64, String)> = Vec::new();

        for code in registry.codes() {
            if code == "general" {
                continue;
            }
            let agent_obj = registry.get(&code)?;
            let config = agent_obj.config();
            let weights = &config.routing_weights;

            let mut score = 0.0;
            let mut reasons = Vec::new();

            // 关键词匹配
            for kw in agent_obj.intent_keywords() {
                if msg_lower.contains(&kw.to_lowercase()) {
                    score += weights.keyword_match;
                    reasons.push(format!("关键词命中: {}", kw));
                }
            }

            // 正则匹配
            for pat in agent_obj.intent_patterns() {
                if pat.is_match(&msg_lower) {
                    score += weights.pattern_match;
                    reasons.push("正则模式命中".to_string());
                }
            }

            // 页面建议加成
            if let Some(ref suggested) = ctx.suggested_agent {
                if suggested == &config.code {
                    score += weights.page_context_bonus;
                    reasons.push("页面上下文推荐".to_string());
                }
            }

            if score > 0.0 {
                let normalized = (score / weights.max_possible_score).min(1.0);
                scores.push((config.code.clone(), normalized, reasons.join(", ")));
            }
        }

        if scores.is_empty() {
            return None;
        }

        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let (code, confidence, reason) = scores.into_iter().next().unwrap();

        let threshold = registry
            .get(&code)
            .map(|a| a.config().routing_weights.threshold)
            .unwrap_or(0.5);

        if confidence >= threshold {
            Some(RoutingDecision {
                agent_code: code,
                confidence,
                reason,
                level: RoutingLevel::L1Rule,
            })
        } else {
            None
        }
    }
}

// ============================================
// LlmClassifierStrategy — L2 LLM 分类器
// ============================================

/// 基于 LLM 轻量分类器的路由策略。
pub struct LlmClassifierStrategy;

#[async_trait]
impl RoutingStrategy for LlmClassifierStrategy {
    async fn route(
        &self,
        ctx: &RoutingContext,
        registry: &AgentRegistry,
        llm: Option<&llm::LlmService>,
    ) -> Option<RoutingDecision> {
        let llm = llm?;

        let selectable = registry.list_selectable();
        if selectable.len() <= 1 {
            return None;
        }

        let agent_descriptions: Vec<String> = selectable
            .iter()
            .map(|c| format!("- {} ({}): {}", c.code, c.name, c.description))
            .collect();

        let page_hint = ctx
            .page_context
            .as_ref()
            .and_then(|v| v.get("page").and_then(|p| p.as_str()))
            .unwrap_or("未知页面");

        let prompt = format!(
            r#"你是一个意图分类器。请根据用户消息和页面上下文，选择最合适的 Agent。

## 可选 Agent
{}

## 页面上下文
当前页面: {}

## 对话历史（最近3条）
{}

## 用户最新消息
"{}"

## 任务
请只输出一个 JSON 对象，不要添加任何解释：
{{"agent_code": "选中的agent code", "confidence": 0.0-1.0, "reason": "选择原因（一句话）"}}

注意：
- confidence > 0.7 才视为有效匹配
- 若无法确定，agent_code 填 "general"
"#,
            agent_descriptions.join("\n"),
            page_hint,
            ctx.conversation_history
                .iter()
                .rev()
                .take(3)
                .map(|(r, c)| format!("{}: {}", r, c))
                .collect::<Vec<_>>()
                .join("\n"),
            ctx.user_message.replace('"', "\\\""),
        );

        let response = llm.generate(&prompt).await.ok()?;
        let json_val: serde_json::Value = serde_json::from_str(&response).ok()?;

        let agent_code = json_val.get("agent_code")?.as_str()?.to_string();
        let confidence = json_val.get("confidence")?.as_f64()?;
        let reason = json_val
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("LLM 分类器决策")
            .to_string();

        if confidence >= 0.6 && registry.get(&agent_code).is_some() {
            Some(RoutingDecision {
                agent_code,
                confidence,
                reason,
                level: RoutingLevel::L2Llm,
            })
        } else {
            None
        }
    }
}

// ============================================
// FallbackStrategy — 通用回退
// ============================================

/// 无条件返回 general Agent 的回退策略。
pub struct FallbackStrategy;

impl FallbackStrategy {
    pub fn decision() -> RoutingDecision {
        RoutingDecision {
            agent_code: "general".to_string(),
            confidence: 1.0,
            reason: "无规则或LLM命中，回退到通用助手".to_string(),
            level: RoutingLevel::Fallback,
        }
    }
}

#[async_trait]
impl RoutingStrategy for FallbackStrategy {
    async fn route(
        &self,
        _ctx: &RoutingContext,
        _registry: &AgentRegistry,
        _llm: Option<&llm::LlmService>,
    ) -> Option<RoutingDecision> {
        Some(Self::decision())
    }
}

// ============================================
// CompositeStrategy — 策略链
// ============================================

/// 将多个 RoutingStrategy 按优先级链式组合。
///
/// 依次调用每个策略，直到某个策略返回 confident 结果。
/// 对于 FallbackStrategy 总是返回结果，因此通常放在链尾作为保底。
pub struct CompositeStrategy {
    strategies: Vec<Box<dyn RoutingStrategy>>,
}

impl CompositeStrategy {
    /// 默认策略链：Keyword → LLM → Fallback
    pub fn default_chain() -> Self {
        Self {
            strategies: vec![
                Box::new(KeywordStrategy),
                Box::new(LlmClassifierStrategy),
                Box::new(FallbackStrategy),
            ],
        }
    }

    /// 用自定义策略链创建
    pub fn new(strategies: Vec<Box<dyn RoutingStrategy>>) -> Self {
        Self { strategies }
    }
}

#[async_trait]
impl RoutingStrategy for CompositeStrategy {
    async fn route(
        &self,
        ctx: &RoutingContext,
        registry: &AgentRegistry,
        llm: Option<&llm::LlmService>,
    ) -> Option<RoutingDecision> {
        for strategy in &self.strategies {
            if let Some(decision) = strategy.route(ctx, registry, llm).await {
                // Fallback 策略总是通过；其他策略需要检查 confidence
                match decision.level {
                    RoutingLevel::Fallback => return Some(decision),
                    RoutingLevel::L1Rule if decision.confidence >= 0.5 => return Some(decision),
                    RoutingLevel::L2Llm if decision.confidence >= 0.6 => return Some(decision),
                    _ => continue,
                }
            }
        }
        None
    }
}

// ============================================
// AgentRouter — 门面
// ============================================

/// Agent 路由器。
///
/// 对外 API 保持稳定（`new` + `route`），内部通过 RoutingStrategy 实现可插拔。
pub struct AgentRouter {
    registry: AgentRegistry,
    strategy: Box<dyn RoutingStrategy>,
}

impl AgentRouter {
    /// 使用默认策略链创建路由器
    pub fn new(registry: AgentRegistry) -> Self {
        Self {
            registry,
            strategy: Box::new(CompositeStrategy::default_chain()),
        }
    }

    /// 使用自定义策略创建路由器（用于测试或特殊场景）
    pub fn with_strategy(mut self, strategy: Box<dyn RoutingStrategy>) -> Self {
        self.strategy = strategy;
        self
    }

    /// 主路由入口
    pub async fn route(
        &self,
        ctx: &RoutingContext,
        llm: Option<&llm::LlmService>,
    ) -> RoutingDecision {
        let decision = self.strategy.route(ctx, &self.registry, llm).await;
        decision.unwrap_or_else(FallbackStrategy::decision)
    }
}
