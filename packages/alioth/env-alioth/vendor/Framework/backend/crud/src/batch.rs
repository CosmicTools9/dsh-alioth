//! 批量操作模块
//!
//! 提供标准化的批量创建/更新/删除请求和响应结构

use serde::{Deserialize, Serialize};

/// 批量创建请求
#[derive(Debug, Clone, Deserialize)]
pub struct BatchCreateRequest<C> {
    pub items: Vec<C>,
}

/// 批量删除请求
#[derive(Debug, Clone, Deserialize)]
pub struct BatchDeleteRequest {
    #[serde(with = "common::serde_zuid::seq")]
    pub ids: Vec<i64>,
}

/// 批量操作响应（success/failed 为计数，MUST 数字序列化——非 zuid；
/// 曾误用 serde_zuid 字符串化，batch_tests 断言已暴露）
#[derive(Debug, Clone, Serialize)]
pub struct BatchResponse {
    pub success: i64,
    pub failed: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

impl BatchResponse {
    pub fn new(success: i64, failed: i64) -> Self {
        Self {
            success,
            failed,
            errors: Vec::new(),
        }
    }

    pub fn with_errors(success: i64, failed: i64, errors: Vec<String>) -> Self {
        Self {
            success,
            failed,
            errors,
        }
    }
}
