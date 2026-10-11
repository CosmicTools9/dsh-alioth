use async_trait::async_trait;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use ai_agent::agents::AgentConfig;
use ai_agent::registry::AgentRegistry;
use ai_agent::router::{AgentRouter, RoutingContext};

use crate::api::chat_sessions::context_envelope::ChatContextEnvelope;
use crate::api::chat_sessions::memory_scope;
use crate::api::chat_sessions::ports::{AgentDispatchPort, RoutingHistoryPort};

/// Agent Registry 缓存 TTL：60 秒
const REGISTRY_TTL: Duration = Duration::from_secs(60);

pub struct AgentRouterAdapter {
    pool: PgPool,
    routing_history: Arc<dyn RoutingHistoryPort>,
    registry: Arc<RwLock<(AgentRegistry, Instant)>>,
}

impl AgentRouterAdapter {
    /// `routing_history` = 路由决策历史写径（D11）：adapter 不 import 具体 adapter，
    /// 只依赖 `ports::RoutingHistoryPort`（生产实现 `DbRoutingHistory`）。
    pub fn new(pool: PgPool, routing_history: Arc<dyn RoutingHistoryPort>) -> Self {
        Self {
            pool: pool.clone(),
            routing_history,
            registry: Arc::new(RwLock::new((
                AgentRegistry::new(),
                Instant::now() - REGISTRY_TTL,
            ))),
        }
    }

    /// 如果缓存过期，从数据库刷新 Agent 配置
    async fn refresh_registry_if_needed(&self) {
        let should_refresh = {
            let lock = self.registry.read().await;
            lock.1.elapsed() > REGISTRY_TTL
        };

        if should_refresh {
            let mut lock = self.registry.write().await;
            // 双重检查，避免多个并发请求同时刷新
            if lock.1.elapsed() > REGISTRY_TTL {
                if let Err(e) = lock.0.load_configs_from_db(&self.pool).await {
                    common::telemetry::warn!("Failed to load agent configs from DB: {}", e);
                }
                lock.1 = Instant::now();
            }
        }
    }

    /// 读取缓存的 Registry（不刷新）
    async fn with_registry<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&AgentRegistry) -> R,
    {
        let lock = self.registry.read().await;
        f(&lock.0)
    }
}

#[async_trait]
impl AgentDispatchPort for AgentRouterAdapter {
    async fn resolve_agent(
        &self,
        session_id: i64,
        user_message: &str,
        envelope: &ChatContextEnvelope,
        history: &[(String, String)],
        locale: &str,
        llm: &llm::LlmService,
    ) -> Result<String, String> {
        // D2.7 pinned_agent：会话 pin 命中且 registry 存在 → 直接返回，跳过
        // router.route 与 routing state 写入（pin 由 switch-agent 设置/清除）。
        // SQL NULL 与缺失等价（无 pin）；实例租用保持 memory 连续性。
        // D10：pinned 读取单实现（memory_scope::read_pinned_agent），不再本层复写 SQL。
        let pinned = memory_scope::read_pinned_agent(&self.pool, session_id)
            .await
            .unwrap_or(None);
        if let Some(code) = pinned {
            self.refresh_registry_if_needed().await;
            let exists = self.with_registry(|r| r.get(&code).is_some()).await;
            if exists {
                return Ok(code);
            }
        }

        self.refresh_registry_if_needed().await;
        let registry = self.with_registry(|r| r.clone()).await;
        let router = AgentRouter::new(registry);

        let routing_ctx = RoutingContext {
            user_message: user_message.to_string(),
            page_context: envelope.page_context.clone(),
            conversation_history: history.to_vec(),
            suggested_agent: envelope.suggested_agent.clone(),
            locale: locale.to_string(),
        };

        let decision = router.route(&routing_ctx, Some(llm)).await;

        // R3（D2.6）：routing 决策环形保留最近 10 条（写径经 ports::RoutingHistoryPort
        // ——D11 单向依赖；条目=单次决策）。写失败上抛（既有语义）。
        self.routing_history
            .append_routing_decision(
                session_id,
                serde_json::json!({
                    "agent": decision.agent_code.clone(),
                    "confidence": decision.confidence,
                    "reason": decision.reason.clone(),
                    "level": format!("{:?}", decision.level),
                    "at": chrono::Utc::now().timestamp(),
                }),
            )
            .await
            .map_err(|e| format!("Failed to save routing state: {}", e))?;

        Ok(decision.agent_code)
    }

    async fn get_agent_config(&self, code: &str) -> Result<AgentConfig, String> {
        self.refresh_registry_if_needed().await;
        let lock = self.registry.read().await;
        lock.0
            .merged_config(code)
            .ok_or_else(|| format!("Agent '{}' not found", code))
    }

    async fn agent_exists(&self, code: &str) -> bool {
        self.with_registry(|r| r.get(code).is_some()).await
    }

    async fn list_agent_configs(&self) -> Result<Vec<AgentConfig>, String> {
        self.refresh_registry_if_needed().await;
        let lock = self.registry.read().await;
        Ok(lock.0.list_selectable())
    }
}
