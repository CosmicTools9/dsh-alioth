//! 领域事件总线（Domain Event Bus）
//!
//! 提供因子间异步解耦通信的基础设施。Factor handler 通过 publish/subscribe
//! 进行事件驱动的跨因子协作，不走 module.json 声明式注册。
//!
//! ## 实现策略
//!
//! - **单实例部署**：使用 `InMemoryEventBus`（基于 `tokio::sync::broadcast`），
//!   所有事件在 Gateway 进程内内存投递。
//! - **多实例部署**：使用 `RchessEventBus`（基于 r-chess `ClusterBroadcastChannel`），
//!   事件通过集群广播跨节点投递。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::broadcast;
use tokio::sync::RwLock as TokioRwLock;

// ─── DomainEvent ────────────────────────────────────────────────────────────

/// 领域事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainEvent {
    /// 事件类型，格式：{EntityName}{Action}（PascalCase）
    pub event_type: String,
    /// 事件来源（因子标识或服务名，如 "orders"）
    pub source: String,
    /// 关联实体 ID
    #[serde(with = "crate::serde_zuid")]
    pub entity_id: i64,
    /// 事件负载（自定义数据）
    pub payload: serde_json::Value,
    /// 事件发生时间（UTC）
    pub timestamp: DateTime<Utc>,
}

impl DomainEvent {
    /// 创建新事件
    pub fn new(
        event_type: impl Into<String>,
        source: impl Into<String>,
        entity_id: i64,
        payload: impl Serialize,
    ) -> Result<Self, serde_json::Error> {
        Ok(Self {
            event_type: event_type.into(),
            source: source.into(),
            entity_id,
            payload: serde_json::to_value(payload)?,
            timestamp: Utc::now(),
        })
    }
}

// ─── EventBusError ──────────────────────────────────────────────────────────

/// 事件总线错误
#[derive(Debug, thiserror::Error)]
pub enum EventBusError {
    #[error("channel not registered: {0}")]
    ChannelNotRegistered(String),
    #[error("serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("database error: {0}")]
    Database(String),
    #[error("internal error: {0}")]
    Internal(String),
    #[error("subscribe failed: {0}")]
    SubscribeFailed(String),
}

// ─── DomainEventBus trait ────────────────────────────────────────────────────

/// 领域事件总线 trait
///
/// Gateway 启动时初始化具体实现并注入到各 factor handler 中。
#[async_trait]
pub trait DomainEventBus: Send + Sync + 'static {
    /// 发布事件到指定频道
    async fn publish(&self, channel: &str, event: &DomainEvent) -> Result<(), EventBusError>;

    /// 订阅指定频道的事件
    async fn subscribe(
        &self,
        channel: &str,
    ) -> Result<broadcast::Receiver<DomainEvent>, EventBusError>;
}

// ─── InMemoryEventBus ───────────────────────────────────────────────────────

/// 内存通道实现（单实例部署用）
///
/// 基于 `tokio::sync::broadcast` 实现，事件在进程内投递。
/// 不校验发布者声明——module.json 事件注册已移除，factor handler 直接发布。
#[derive(Debug)]
pub struct InMemoryEventBus {
    /// 频道名称 → broadcast sender
    channels: TokioRwLock<HashMap<String, broadcast::Sender<DomainEvent>>>,
    /// 每个频道的默认缓冲区大小
    channel_capacity: usize,
}

impl InMemoryEventBus {
    pub fn new() -> Self {
        Self::with_capacity(256)
    }

    pub fn with_capacity(channel_capacity: usize) -> Self {
        Self {
            channels: TokioRwLock::new(HashMap::new()),
            channel_capacity,
        }
    }

    async fn get_or_create_sender(&self, channel: &str) -> broadcast::Sender<DomainEvent> {
        let channels = self.channels.read().await;
        if let Some(sender) = channels.get(channel) {
            return sender.clone();
        }
        drop(channels);

        let mut channels = self.channels.write().await;
        // 双检锁：再次检查
        channels
            .entry(channel.to_string())
            .or_insert_with(|| broadcast::channel(self.channel_capacity).0)
            .clone()
    }
}

impl Default for InMemoryEventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DomainEventBus for InMemoryEventBus {
    async fn publish(&self, channel: &str, event: &DomainEvent) -> Result<(), EventBusError> {
        let sender = self.get_or_create_sender(channel).await;
        let _ = sender.send(event.clone());
        Ok(())
    }

    async fn subscribe(
        &self,
        channel: &str,
    ) -> Result<broadcast::Receiver<DomainEvent>, EventBusError> {
        let sender = self.get_or_create_sender(channel).await;
        Ok(sender.subscribe())
    }
}
