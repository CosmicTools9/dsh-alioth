//! 兼容性重新导出
//!
//! 此模块保留以避免破坏下游 `use common::api_response::...` 的引用。
//! `L1ApiResponse` / `L1JsonResponse` 已移除，请统一使用 `ApiResponse` / `JsonResponse`。

pub use crate::data::{ApiResponse, JsonResponse};
