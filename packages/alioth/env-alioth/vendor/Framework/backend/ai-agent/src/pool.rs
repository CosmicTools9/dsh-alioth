//! Per-user Agent 实例池（add-agent-pool-user-memory）。
//!
//! `AgentRegistry` 是静态配置池（内置 + DB 合并），共享无状态实例。
//! 本模块在其之上增加「per-user 实例化 + 生命周期管理」：
//! 每个 `(user_id, agent_code)` 租用独立 `AgentInstance`，实例持用户级
//! memory 空间（RwLock，懒加载）+ 最近访问时间；空闲超 TTL 回收，
//! 容量超上限 LRU 淘汰。池键含 user_id → 天然 per-user 隔离。

use crate::agents::{build_default_registry, Agent};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// 实例空闲回收阈值（默认 30 分钟）
const DEFAULT_TTL: Duration = Duration::from_secs(30 * 60);
/// 池容量上限（默认 1024 实例，超限 LRU 淘汰）
const DEFAULT_CAPACITY: usize = 1024;

/// 单个用户的 agent 实例：包装既有无状态 Agent + 用户 memory 空间。
pub struct AgentInstance {
    agent: Box<dyn Agent>,
    /// 用户级 memory（跨 session 长期记忆，懒加载；由 UserMemoryStore 读写）
    memory: RwLock<serde_json::Value>,
    /// 最近访问时间（TTL 回收依据）
    last_accessed: RwLock<Instant>,
}

impl AgentInstance {
    /// 底层 Agent（只读，配置/路由用）
    pub fn agent(&self) -> &dyn Agent {
        self.agent.as_ref()
    }

    /// 当前 memory 快照（只读——修改应经 store 落库后 set_memory）
    pub async fn memory(&self) -> serde_json::Value {
        self.memory.read().await.clone()
    }

    /// 替换 memory（UserMemoryStore.save 后同步）
    pub async fn set_memory(&self, memory: serde_json::Value) {
        *self.memory.write().await = memory;
    }

    /// 触碰访问时间（每次租用时调用）
    pub async fn touch(&self) {
        *self.last_accessed.write().await = Instant::now();
    }

    /// 最近访问时间（回收/淘汰判断）
    pub(crate) async fn last_accessed(&self) -> Instant {
        *self.last_accessed.read().await
    }
}

/// Per-user Agent 实例池。
///
/// 配置源由外部（AgentRouterAdapter）维护；本池仅用内置 registry 建实例
/// （DB 配置合并在 get_or_create 时经 `build_agent` 可选接入——当前用内置，
/// 配置热更由 TTL 回收后按需重建实例）。
pub struct AgentPool {
    instances: RwLock<HashMap<(i64, String), Arc<AgentInstance>>>,
    ttl: Duration,
    capacity: usize,
}

impl Default for AgentPool {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentPool {
    pub fn new() -> Self {
        Self {
            instances: RwLock::new(HashMap::new()),
            ttl: DEFAULT_TTL,
            capacity: DEFAULT_CAPACITY,
        }
    }

    /// 获取（或创建）指定用户的 agent 实例。
    ///
    /// 池键 `(user_id, agent_code)` → per-user 隔离。创建时从内置 registry
    /// 取 agent 建实例，memory 为空（由 UserMemoryStore 懒加载填充）。
    pub async fn get_or_create(
        &self,
        user_id: i64,
        agent_code: &str,
    ) -> Option<Arc<AgentInstance>> {
        let key = (user_id, agent_code.to_string());

        {
            let lock = self.instances.read().await;
            if let Some(inst) = lock.get(&key) {
                inst.touch().await;
                return Some(inst.clone());
            }
        }

        // 未命中 → 写锁建实例
        let mut lock = self.instances.write().await;
        if let Some(inst) = lock.get(&key) {
            inst.touch().await;
            return Some(inst.clone());
        }

        // 容量检查：超限 LRU 淘汰
        if lock.len() >= self.capacity {
            self.evict_lru(&mut lock).await;
        }

        let agent = self.build_agent(agent_code)?;

        let instance = Arc::new(AgentInstance {
            agent,
            memory: RwLock::new(serde_json::json!({})),
            last_accessed: RwLock::new(Instant::now()),
        });
        lock.insert(key, instance.clone());
        Some(instance)
    }

    /// 回收空闲实例（空闲超 TTL 的从池移除）。
    pub async fn reclaim_idle(&self) {
        let mut lock = self.instances.write().await;
        let ttl = self.ttl;
        let now = Instant::now();
        let mut expired: Vec<(i64, String)> = Vec::new();
        for (k, inst) in lock.iter() {
            let last = inst.last_accessed().await;
            if now.duration_since(last) > ttl {
                expired.push(k.clone());
            }
        }
        for k in expired {
            lock.remove(&k);
        }
    }

    /// 池当前实例数（测试/诊断）。
    pub async fn len(&self) -> usize {
        self.instances.read().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }

    /// LRU 淘汰最久未访问的实例（容量超限时）。
    async fn evict_lru(&self, lock: &mut HashMap<(i64, String), Arc<AgentInstance>>) {
        let mut oldest_key: Option<(i64, String)> = None;
        let mut oldest_time: Option<Instant> = None;
        for (k, inst) in lock.iter() {
            let last = inst.last_accessed().await;
            if oldest_time.map(|t| last < t).unwrap_or(true) {
                oldest_key = Some(k.clone());
                oldest_time = Some(last);
            }
        }
        if let Some(k) = oldest_key {
            lock.remove(&k);
        }
    }

    /// 从内置 registry 构建 agent 实例。
    fn build_agent(&self, code: &str) -> Option<Box<dyn Agent>> {
        let mut builtin = build_default_registry();
        if builtin.contains_key(code) {
            builtin.remove(code)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn per_user_instances_isolated() {
        let pool = AgentPool::new();
        let a = pool.get_or_create(1, "general").await.expect("user A");
        let b = pool.get_or_create(2, "general").await.expect("user B");
        assert!(!Arc::ptr_eq(&a, &b), "不同用户实例必须隔离");
        // 同用户同 agent 复用
        let a2 = pool
            .get_or_create(1, "general")
            .await
            .expect("user A again");
        assert!(Arc::ptr_eq(&a, &a2), "同用户应复用同一实例");
    }

    #[tokio::test]
    async fn different_agents_same_user_distinct() {
        let pool = AgentPool::new();
        let g = pool.get_or_create(1, "general").await.expect("general");
        let f = pool.get_or_create(1, "form_filling").await.expect("form");
        assert!(!Arc::ptr_eq(&g, &f), "不同 agent 实例必须隔离");
    }

    #[tokio::test]
    async fn unknown_agent_returns_none() {
        let pool = AgentPool::new();
        assert!(pool.get_or_create(1, "nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn capacity_evicts_lru() {
        let pool = AgentPool {
            instances: RwLock::new(HashMap::new()),
            ttl: DEFAULT_TTL,
            capacity: 2,
        };
        let _a = pool.get_or_create(1, "general").await.expect("a");
        let _b = pool.get_or_create(2, "general").await.expect("b");
        let _c = pool.get_or_create(3, "general").await.expect("c");
        assert_eq!(pool.len().await, 2, "超容量应淘汰最久未访问");
    }

    #[tokio::test]
    async fn ttl_reclaims_idle() {
        let pool = AgentPool {
            instances: RwLock::new(HashMap::new()),
            ttl: Duration::from_millis(1), // 极短 TTL 便于测试
            capacity: 10,
        };
        let _a = pool.get_or_create(1, "general").await.expect("a");
        assert_eq!(pool.len().await, 1);
        tokio::time::sleep(Duration::from_millis(20)).await;
        pool.reclaim_idle().await;
        assert_eq!(pool.len().await, 0, "空闲超 TTL 应被回收");
    }
}
