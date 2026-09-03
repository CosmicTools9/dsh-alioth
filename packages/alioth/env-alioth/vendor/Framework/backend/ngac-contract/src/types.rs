use serde::{Deserialize, Serialize};

/// NGAC Policy Decision Point check request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdpCheckRequest {
    #[serde(with = "common::serde_zuid")]
    pub user_id: i64,
    pub resource: String,
    pub action: String,
}

/// NGAC Policy Decision Point check response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdpCheckResponse {
    pub permitted: bool,
    pub reason: String,
}

/// NGAC 策略版本探针响应（fix-ngac-decision-consistency D4）。
///
/// Gateway PEP 以此版本为 per-worker 决策/列缓存的失效信号。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyVersionResponse {
    pub version: i64,
}

/// NGAC access decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    Permit,
    Deny,
    NotApplicable,
}

impl std::fmt::Display for Decision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Decision::Permit => write!(f, "Permit"),
            Decision::Deny => write!(f, "Deny"),
            Decision::NotApplicable => write!(f, "NotApplicable"),
        }
    }
}

/// NGAC decision batch check request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdpCheckBatchRequest {
    #[serde(with = "common::serde_zuid")]
    pub user_id: i64,
    pub checks: Vec<CheckItem>,
}

/// Single check item for batch requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckItem {
    pub resource: String,
    pub action: String,
}

/// NGAC decision batch check response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdpCheckBatchResponse {
    pub results: Vec<PdpCheckResponse>,
}

/// Error type for NGAC contract operations.
#[derive(Debug, thiserror::Error)]
pub enum NgacError {
    #[error("HTTP request failed: {0}")]
    HttpError(String),
    #[error("Serialization failed: {0}")]
    SerializationError(String),
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),
}

/// NGAC list/query request — retrieves visible resource IDs for row-level filtering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdpListRequest {
    #[serde(with = "common::serde_zuid")]
    pub user_id: i64,
    pub resource_type: String,
    pub action: String,
}

/// NGAC list/query response — contains visible resource IDs for RLS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdpListResponse {
    pub permitted: bool,
    pub reason: String,
    /// Resource IDs the user can access. `None` means all resources visible.
    #[serde(with = "common::serde_zuid::opt_seq")]
    pub visible_ids: Option<Vec<i64>>,
}

/// NGAC 列级授权请求——查询用户对某资源类型可访问的列集合。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdpColumnsRequest {
    #[serde(with = "common::serde_zuid")]
    pub user_id: i64,
    pub resource_type: String,
}

/// NGAC 列级授权响应。
///
/// `columns`: 用户被授权读取的列（DTO 字段名）。`["*"]` 表示全部列（通配/无列级策略）。
/// 授权语义来自 association 的 `read:{col}`（具体列）与 `read:*`（通配）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdpColumnsResponse {
    pub permitted: bool,
    pub reason: String,
    /// 授权列集合（DTO 字段名）；`["*"]` 表示全部
    pub columns: Vec<String>,
}
