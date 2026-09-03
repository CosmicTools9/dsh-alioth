//! Alioth Framework Common Library
//!
//! ## 模块分类
//!
//! ### 核心类型（所有模块 backend 依赖）
//! - [`error`] — 统一错误类型 `AliothError`、`ErrorResponse`
//! - [`data`] — `ApiResponse`、`PaginatedResponse`、`ListQuery`
//! - [`context`] — 请求上下文、用户身份提取
//! - [`scalar`] — 标量引用转换服务 + 值对象类型
//! - [`serde_zuid`] — ZUID 序列化辅助
//! - [`ontology`] — 编译期本体维度绑定（表名 → ZUID）
//!
//! ### 基础设施 seam（Gateway/Meta/SSO 入口使用）
//! - [`cors`] — CORS 配置构建（`build_cors()`）
//! - [`server`] — 监听地址绑定错误增强（`bind_error()`）
//! - [`validation`] — PostgreSQL 标识符安全校验
//! - [`middleware`] — `NgacPepMiddleware` JWT 认证中间件
//! ### Domain traits
//! - [`cluster`] — 集群管理 trait
//! - [`device`] — 设备命令/消息 trait
//! - [`event_bus`] — 领域事件总线
//! - [`messaging`] — 消息服务 trait
// api_response 模块保留为兼容性入口
pub mod api_response;
pub mod audit;
pub mod cluster;
pub mod context;
pub mod cors;
pub mod data;
pub mod device;
pub mod dim_registry;
pub mod dk_context;
pub mod email;
pub mod error;
pub mod event_bus;
pub mod messaging;
pub mod middleware;
pub mod ngac_org;
pub mod ontology;
pub mod permissions;
pub mod plan_execution;
pub mod progress;
pub mod scalar;
pub mod search;
pub mod serde_zuid;
pub mod server;
pub mod sms;
pub mod status;
pub mod system_user;
pub mod telemetry;
pub mod testing;
pub mod validation;

// 重新导出常用的类型和函数
pub use context::{
    extract_user_email, extract_user_id, AuditFields, RequestContext, RequestContextExt,
    SYSTEM_USER_ID,
};
pub use cors::build_cors;
pub use data::{ApiResponse, JsonResponse, ListQuery, PaginatedResponse};
pub use email::{EmailConfig, EmailService, SmtpEmailService};
pub use error::{AliothError, ErrorResponse, ErrorSource, Result};
pub use middleware::{AuthContext, NgacPepMiddleware, PublicRouteMatcher, RateLimitMiddleware};
pub use sms::{CloudSmsService, SmsService};
pub use validation::{validate_pg_ident, validate_qualified_ident};
