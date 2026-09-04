// 注意：认证和授权相关 API 已移动到 SSO 服务
// Gateway 现在仅作为业务应用的统一入口和授权网关
#[cfg(feature = "sso")]
pub mod admin_ngac_assist;
pub mod contacts;
pub mod profile;

pub mod approval_formula;
pub mod approvals;
pub mod chat_sessions;
pub mod dashboard;
pub mod entity_binding;
pub mod files;
pub mod global_overview;
pub mod inbox;
pub mod legal_search;
pub mod standard_search;
pub mod system_config_llm_test;
pub mod system_push;
pub mod version;
