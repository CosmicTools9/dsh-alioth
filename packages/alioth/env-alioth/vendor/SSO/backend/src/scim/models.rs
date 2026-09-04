//! SCIM 2.0 数据模型（RFC 7643）
//!
//! 仅实现本服务所需的资源与列表/错误结构。字段名严格遵循 SCIM 规范（camelCase）。

use serde::{Deserialize, Serialize};

/// SCIM `name` 复杂属性。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScimName {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatted: Option<String>,
    #[serde(rename = "givenName", skip_serializing_if = "Option::is_none")]
    pub given_name: Option<String>,
    #[serde(rename = "familyName", skip_serializing_if = "Option::is_none")]
    pub family_name: Option<String>,
}

/// SCIM `emails[]` 元素。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScimEmail {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub email_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<bool>,
}

/// SCIM `members[]`（Group 成员引用）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScimMemberRef {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub member_type: Option<String>,
    #[serde(rename = "$ref", skip_serializing_if = "Option::is_none")]
    pub ref_: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

/// SCIM `meta` 元数据。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScimMeta {
    #[serde(rename = "resourceType", skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(rename = "lastModified", skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

/// SCIM User 资源。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScimUser {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schemas: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "externalId", skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    #[serde(rename = "userName", skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<ScimName>,
    #[serde(rename = "displayName", skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emails: Option<Vec<ScimEmail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<ScimMemberRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ScimMeta>,
}

/// SCIM Group 资源。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScimGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schemas: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "displayName", skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<ScimMemberRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ScimMeta>,
}

/// SCIM PATCH 操作（RFC 7644）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchOperation {
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
}

/// SCIM PATCH 请求体。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct ScimPatchRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schemas: Option<Vec<String>>,
    pub Operations: Vec<PatchOperation>,
}

/// SCIM 列表响应（ListResponse）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct ListResponse<T> {
    pub schemas: Vec<String>,
    pub totalResults: usize,
    pub startIndex: usize,
    pub itemsPerPage: usize,
    pub Resources: Vec<T>,
}

/// SCIM 错误响应。
#[derive(Debug, Clone, Serialize)]
pub struct ScimError {
    pub schemas: Vec<String>,
    pub detail: String,
    pub status: String,
}

impl ScimError {
    pub fn new(status: &str, detail: &str) -> Self {
        Self {
            schemas: vec!["urn:ietf:params:scim:api:messages:2.0:Error".to_string()],
            detail: detail.to_string(),
            status: status.to_string(),
        }
    }
}

/// 统一的 SCIM 错误响应构造。
pub fn error_response(
    status: actix_web::http::StatusCode,
    detail: &str,
) -> actix_web::HttpResponse {
    actix_web::HttpResponse::build(status).json(ScimError::new(status.as_str(), detail))
}
