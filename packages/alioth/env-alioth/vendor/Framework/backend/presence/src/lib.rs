//! framework-presence — 在线状态追踪服务
//!
//! 轻量级 in-memory presence tracker，用于 Gateway 层面的在线状态检测。
//! 通过 heartbeat 机制更新最后活跃时间，超时后自动标记离线。
//!
//! # 线程安全
//! 内部使用 `Arc<RwLock<HashMap>>`，支持跨 tokio task 共享。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// 默认在线超时（5 分钟无 heartbeat 视为离线）
pub const DEFAULT_ONLINE_TIMEOUT_SECS: u64 = 300;

/// 默认清理间隔（每 60 秒清理一次过期条目）
pub const DEFAULT_CLEANUP_INTERVAL_SECS: u64 = 60;

/// 在线状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnlineStatus {
    /// 用户/联系人 ID
    pub user_id: i64,
    /// 是否在线
    pub is_online: bool,
    /// 最后一次 heartbeat 时间（UNIX 时间戳秒）
    pub last_seen_at: Option<u64>,
}

/// 在线状态追踪器
#[derive(Debug, Clone)]
pub struct PresenceTracker {
    inner: Arc<RwLock<HashMap<i64, Instant>>>,
    timeout: Duration,
}

impl PresenceTracker {
    /// 创建新的 PresenceTracker
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(DEFAULT_ONLINE_TIMEOUT_SECS))
    }

    /// 创建指定超时的 PresenceTracker
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            timeout,
        }
    }

    /// 记录 heartbeat（更新最后活跃时间）
    pub async fn heartbeat(&self, user_id: i64) {
        let mut map = self.inner.write().await;
        map.insert(user_id, Instant::now());
    }

    /// 批量查询在线状态
    pub async fn get_statuses(&self, user_ids: &[i64]) -> Vec<OnlineStatus> {
        let map = self.inner.read().await;
        let now = Instant::now();
        user_ids
            .iter()
            .map(|&uid| {
                let last_seen = map.get(&uid).copied();
                match last_seen {
                    Some(t) => OnlineStatus {
                        user_id: uid,
                        is_online: now.duration_since(t) < self.timeout,
                        last_seen_at: Some(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                        ),
                    },
                    None => OnlineStatus {
                        user_id: uid,
                        is_online: false,
                        last_seen_at: None,
                    },
                }
            })
            .collect()
    }

    /// 查询单个用户在线状态
    pub async fn is_online(&self, user_id: i64) -> bool {
        let statuses = self.get_statuses(&[user_id]).await;
        statuses.first().map(|s| s.is_online).unwrap_or(false)
    }

    /// 启动后台清理任务（移除超时条目）
    pub fn start_cleanup_task(self, interval: Duration) {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                let mut map = self.inner.write().await;
                let now = Instant::now();
                map.retain(|_, last_seen| now.duration_since(*last_seen) < self.timeout);
            }
        });
    }
}

impl Default for PresenceTracker {
    fn default() -> Self {
        Self::new()
    }
}
