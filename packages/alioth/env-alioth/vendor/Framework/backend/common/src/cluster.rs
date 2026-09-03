//! 集群基础设施服务 seam
//!
//! 提供时间轮调度和集群广播能力。
//! 模块通过 `AppClusterService` 访问基础设施，无需了解 Raft、Disruptor 等内部实现。
//!
//! 错误语义标注：
//! - HTTP 层错误 → `AliothError::BadRequest` / `AliothError::NotFound`
//! - 调度子系统错误 → `AliothError::External { source: "Timewheel", ... }`
//! - 集群子系统错误 → `AliothError::External { source: "Cluster", ... }`

use crate::error::AliothError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// 集群基础设施服务 trait。
///
/// Gateway 启动时根据配置注入具体实现。
/// 模块 handler 通过 `web::Data<Option<Arc<dyn AppClusterService>>>` 获取。
#[async_trait]
pub trait AppClusterService: Send + Sync + 'static {
    /// 单次调度任务。
    async fn schedule_once(
        &self,
        after: Duration,
        task: ScheduledTask,
    ) -> Result<TaskId, AliothError>;

    /// 周期性调度任务。
    async fn schedule_recurring(
        &self,
        interval: Duration,
        task: ScheduledTask,
    ) -> Result<TaskId, AliothError>;

    /// 取消已调度任务。
    async fn cancel_task(&self, task_id: TaskId) -> Result<(), AliothError>;

    /// 向集群所有节点广播消息。
    async fn broadcast_cluster(&self, msg: ClusterMessage) -> Result<(), AliothError>;

    /// 获取当前集群节点列表。
    async fn get_nodes(&self) -> Result<Vec<NodeInfo>, AliothError>;
}

/// 任务 ID。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct TaskId(pub u64);

/// 调度任务定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub task_type: String,
    pub payload: serde_json::Value,
}

impl ScheduledTask {
    pub fn new(
        task_type: impl Into<String>,
        payload: impl Serialize,
    ) -> Result<Self, serde_json::Error> {
        Ok(Self {
            task_type: task_type.into(),
            payload: serde_json::to_value(payload)?,
        })
    }
}

/// 集群消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterMessage {
    pub msg_type: String,
    pub payload: serde_json::Value,
}

impl ClusterMessage {
    pub fn new(
        msg_type: impl Into<String>,
        payload: impl Serialize,
    ) -> Result<Self, serde_json::Error> {
        Ok(Self {
            msg_type: msg_type.into(),
            payload: serde_json::to_value(payload)?,
        })
    }
}

/// 集群节点信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub node_id: u64,
    pub endpoint: String,
    pub status: NodeStatus,
}

/// 节点状态。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeStatus {
    Healthy,
    Degraded,
    Offline,
}
