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

    /// 租用 per-user agent 实例（add-agent-pool-user-memory）：从 session 反查
    /// owner，按 (user_id, agent_code) 建实例（池键隔离）。失败仅 warn——
    /// 池实例只服务 memory 注入，不影响主链。
    async fn rent_pool_instance(&self, session_id: i64, agent_code: &str) {
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
                .get_or_create(owner, agent_code)
                .await
                .is_none()
            {
                common::telemetry::warn!(
                    "agent pool: failed to create instance for user {} agent {}",
                    owner,
                    agent_code
                );
            }
        }
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
        // D2.7 pinned_agent：会话 pin 命中且 registry 存在 → 直接返回，跳过
        // router.route 与 routing state 写入（pin 由 switch-agent 设置/清除）。
        // SQL NULL 与缺失等价（无 pin）；实例租用保持 memory 连续性。
        let pinned: Option<String> = sqlx::query_scalar::<_, String>(
            r#"SELECT agent_state->>'pinned_agent'
               FROM isahl."zc_id_thre-ai_session" WHERE id = $1"#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .filter(|s| !s.is_empty());
        if let Some(code) = pinned {
            self.refresh_registry_if_needed().await;
            let exists = self.with_registry(|r| r.get(&code).is_some()).await;
            if exists {
                self.rent_pool_instance(session_id, &code).await;
                return Ok(code);
            }
        }

        self.refresh_registry_if_needed().await;
        let registry = self.with_registry(|r| r.clone()).await;
        let router = AgentRouter::new(registry);

        let suggested_agent = page_context.as_ref().and_then(|v| {
            // 前端 AIChatContext 传输形态为嵌套 {pageContext:{suggestedAgent}}，
            // 兼容顶层直传与嵌套两种形态（实测嵌套形态下顶层读取永 None →
            // 页面建议加成失效，填单场景被路由到 general）。
            v.get("suggestedAgent")
                .or_else(|| v.get("pageContext").and_then(|pc| pc.get("suggestedAgent")))
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

        // R3（D2.6）：routing 决策环形保留最近 10 条（append_bounded_state 读改写
        // 截断；条目=单次决策）。无界 append 路径已消除；写失败上抛（既有语义）。
        let _ = super::db_session::append_bounded_state(
            &self.pool,
            session_id,
            "routing",
            serde_json::json!({
                "agent": decision.agent_code.clone(),
                "confidence": decision.confidence,
                "reason": decision.reason.clone(),
                "level": format!("{:?}", decision.level),
                "at": chrono::Utc::now().timestamp(),
            }),
            super::db_session::ROUTING_STATE_CAP,
        )
        .await
        .map_err(|e| format!("Failed to save routing state: {}", e))?;

        self.rent_pool_instance(session_id, &decision.agent_code)
            .await;

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

    async fn sync_user_memory(
        &self,
        user_id: i64,
        agent_code: &str,
        memory: serde_json::Value,
    ) -> Result<(), String> {
        // 池实例不存在（agent 非内置/未租用）→ 无实例可同步，主链不阻断
        if let Some(inst) = self.agent_pool.get_or_create(user_id, agent_code).await {
            inst.set_memory(memory).await;
        }
        Ok(())
    }
}
