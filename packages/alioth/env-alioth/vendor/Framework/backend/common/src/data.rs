//! Alioth 统一数据结构
//!
//! 提供跨模块共享的标准化数据结构，包括：
//! - `ApiResponse<T>` / `JsonResponse<T>` — 标准 API 响应（{success, data}）
//! - `PaginatedResponse<T>` — 分页响应
//! - `ListQuery` — 列表查询参数（分页 + 过滤 + 排序）

use actix_web::{body::BoxBody, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize, Serializer};

// ═══════════════════════════════════════════════════════════════════════════════
// ApiResponse<T> — 应用层标准响应 {success, data}
// ═══════════════════════════════════════════════════════════════════════════════

/// 应用层标准成功响应结构
///
/// Meta、Gateway 等应用层统一使用此结构包装成功响应，序列化为 `{success, data}`。
#[derive(Debug, Clone, Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: T,
}

impl<T> ApiResponse<T> {
    /// 构造成功响应
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data,
        }
    }
}

/// 应用层自动响应包装器
///
/// 实现 `actix_web::Responder`，自动将内部数据包装为 `ApiResponse` 并返回 HTTP 200。
pub struct JsonResponse<T>(pub T);

impl<T: Serialize> Responder for JsonResponse<T> {
    type Body = BoxBody;

    fn respond_to(self, _req: &HttpRequest) -> HttpResponse<Self::Body> {
        HttpResponse::Ok().json(ApiResponse::success(self.0))
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// PaginatedResponse<T>
// ═══════════════════════════════════════════════════════════════════════════════

/// 分页响应结构
///
/// 所有后端服务统一使用此结构返回分页数据。
/// 序列化时与前端 `PaginatedData<T>` 对齐，同时保留后端向后兼容字段。
#[derive(Debug, Clone)]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

impl<T> PaginatedResponse<T> {
    pub fn new(items: Vec<T>, total: i64, page: i64, page_size: i64) -> Self {
        Self {
            items,
            total,
            page,
            page_size,
        }
    }

    /// 计算总页数
    pub fn total_pages(&self) -> i64 {
        if self.total == 0 || self.page_size == 0 {
            0
        } else {
            (self.total + self.page_size - 1) / self.page_size
        }
    }
}

impl<T: Serialize> Serialize for PaginatedResponse<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("PaginatedResponse", 7)?;
        state.serialize_field("list", &self.items)?;
        state.serialize_field("items", &self.items)?;
        state.serialize_field("total", &self.total)?;
        state.serialize_field("page", &self.page)?;
        state.serialize_field("pageSize", &self.page_size)?;
        state.serialize_field("page_size", &self.page_size)?;
        state.serialize_field("totalPages", &self.total_pages())?;
        state.end()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// ListQuery — 列表查询参数
// ═══════════════════════════════════════════════════════════════════════════════

fn default_page() -> i64 {
    1
}

fn default_page_size() -> i64 {
    20
}

/// 列表查询参数（分页 + 过滤 + 排序）
///
/// 从 `alioth-crud` 上移至 `alioth-common`，供所有模块和 Gateway 直接依赖。
#[derive(Debug, Clone, Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_page")]
    #[serde(with = "crate::serde_zuid")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    #[serde(with = "crate::serde_zuid")]
    pub page_size: i64,
    #[serde(rename = "filter_field", default)]
    pub filter_field: Option<String>,
    #[serde(rename = "filter_op", default)]
    pub filter_op: Option<String>,
    #[serde(rename = "filter_value", default)]
    pub filter_value: Option<String>,
    #[serde(rename = "sort_field", default)]
    pub sort_field: Option<String>,
    #[serde(rename = "sort_order", default)]
    pub sort_order: Option<String>,
}

impl ListQuery {
    /// 计算 SQL OFFSET
    pub fn offset(&self) -> i64 {
        (self.page - 1) * self.page_size
    }
}

impl Default for ListQuery {
    /// 默认查询：第 1 页，每页 20 条（与 serde 反序列化默认一致）
    fn default() -> Self {
        ListQuery {
            page: default_page(),
            page_size: default_page_size(),
            filter_field: None,
            filter_op: None,
            filter_value: None,
            sort_field: None,
            sort_order: None,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_std_api_response_success() {
        let response = ApiResponse::success(json!({"id": 42}));
        let parsed = serde_json::to_value(&response).unwrap();
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["data"]["id"], 42);
    }

    #[test]
    fn test_paginated_response_serialization() {
        let response = PaginatedResponse::new(vec!["a", "b", "c"], 100, 2, 20);
        let json = serde_json::to_value(&response).unwrap();

        assert_eq!(json["list"], serde_json::json![["a", "b", "c"]]);
        assert_eq!(json["items"], serde_json::json![["a", "b", "c"]]);
        assert_eq!(json["total"], 100);
        assert_eq!(json["page"], 2);
        assert_eq!(json["pageSize"], 20);
        assert_eq!(json["page_size"], 20);
        assert_eq!(json["totalPages"], 5);
    }

    #[test]
    fn test_list_query_offset() {
        let q = ListQuery {
            page: 3,
            page_size: 20,
            filter_field: None,
            filter_op: None,
            filter_value: None,
            sort_field: None,
            sort_order: None,
        };
        assert_eq!(q.offset(), 40);
    }

    // ── PaginatedResponse::total_pages boundary tests ─────────────────────

    #[test]
    fn test_total_pages_zero_total() {
        let resp = PaginatedResponse::new(vec!["a"], 0, 1, 20);
        assert_eq!(resp.total_pages(), 0);
    }

    #[test]
    fn test_total_pages_zero_page_size() {
        let resp = PaginatedResponse::new(vec!["a"], 100, 1, 0);
        assert_eq!(resp.total_pages(), 0);
    }

    #[test]
    fn test_total_pages_both_zero() {
        let resp = PaginatedResponse::<()>::new(vec![], 0, 1, 0);
        assert_eq!(resp.total_pages(), 0);
    }

    #[test]
    fn test_total_pages_exact_division() {
        let resp = PaginatedResponse::new(vec!["a"; 20], 20, 1, 20);
        assert_eq!(resp.total_pages(), 1);
    }

    #[test]
    fn test_total_pages_round_up() {
        let resp = PaginatedResponse::new(vec!["a"; 21], 21, 1, 20);
        assert_eq!(resp.total_pages(), 2);
    }

    #[test]
    fn test_total_pages_single_item() {
        let resp = PaginatedResponse::new(vec!["a"], 1, 1, 20);
        assert_eq!(resp.total_pages(), 1);
    }

    // ── ApiResponse serialization format verification ─────────────────────

    #[test]
    fn test_api_response_success_structure() {
        let response = ApiResponse::success(42);
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json, serde_json::json!({"success": true, "data": 42}));
    }

    #[test]
    fn test_api_response_success_with_object_data() {
        let response = ApiResponse::success(json!({"key": "value", "num": 10}));
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"success": true, "data": {"key": "value", "num": 10}})
        );
    }
}
