use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use ai_agent::agents::AgentConfig;
use ai_agent::registry::AgentRegistry;
use ai_agent::router::{AgentRouter, RoutingContext};

use crate::api::chat_sessions::ports::AgentDispatchPort;

/// Agent Registry 缓存 TTL：60 秒
const REGISTRY_TTL: Duration = Duration::from_secs(60);

pub struct AgentRouterAdapter {
    pool: PgPool,
    registry: Arc<RwLock<(AgentRegistry, Instant)>>,
    /// per-user agent 实例池（add-agent-pool-user-memory）
    agent_pool: ai_agent::pool::AgentPool,
    /// 用户 memory 存储
    memory_store: super::super::memory_store::UserMemoryStore,
}

impl AgentRouterAdapter {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool: pool.clone(),
            registry: Arc::new(RwLock::new((
                AgentRegistry::new(),
                Instant::now() - REGISTRY_TTL,
            ))),
            agent_pool: ai_agent::pool::AgentPool::new(),
            memory_store: super::super::memory_store::UserMemoryStore::new(pool),
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
        page_context: Option<Value>,
        history: &[(String, String)],
        locale: &str,
        llm: &llm::LlmService,
    ) -> Result<String, String> {
        self.refresh_registry_if_needed().await;
        let registry = self.with_registry(|r| r.clone()).await;
        let router = AgentRouter::new(registry);

        let suggested_agent = page_context.as_ref().and_then(|v| {
            v.get("suggestedAgent")
                .and_then(|s| s.as_str())
                .map(String::from)
        });

        let routing_ctx = RoutingContext {
            user_message: user_message.to_string(),
            page_context,
            conversation_history: history.to_vec(),
            suggested_agent,
            locale: locale.to_string(),
        };

        let decision = router.route(&routing_ctx, Some(llm)).await;

        let _ = sqlx::query(
            r#"UPDATE isahl."zc_id_thre-ai_session"
               SET agent_state = agent_state || $1
               WHERE id = $2"#,
        )
        .bind(serde_json::json!({
            "routing": {
                "confidence": decision.confidence,
                "reason": decision.reason,
                "level": format!("{:?}", decision.level)
            }
        }))
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to save routing state: {}", e))?;

        // 租用 per-user agent 实例（add-agent-pool-user-memory）：
        // 从 session 反查 owner，按 (user_id, agent_code) 建实例（池键隔离）。
        let session_owner: Option<i64> = sqlx::query_scalar(
            r#"SELECT created_by_id FROM isahl."zc_id_thre-ai_session" WHERE id = $1"#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten();
        if let Some(owner) = session_owner {
            if self
                .agent_pool
                .get_or_create(owner, &decision.agent_code)
                .await
                .is_none()
            {
                common::telemetry::warn!(
                    "agent pool: failed to create instance for user {} agent {}",
                    owner,
                    decision.agent_code
                );
            }
        }

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

    async fn load_user_memory(&self, user_id: i64) -> Result<serde_json::Value, String> {
        self.memory_store.load(user_id).await
    }
}
