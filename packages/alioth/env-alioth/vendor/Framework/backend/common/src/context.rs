//! 请求上下文模块
//!
//! 提供统一的请求上下文管理，包含当前登录用户信息、审计字段等
//! 用于 Gateway、SSO 和业务模块(Modules)之间的用户身份传递

use actix_web::{HttpMessage, HttpRequest};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 请求上下文
///
/// 包含当前请求的用户信息和审计相关数据
/// 通过 Actix-web 的 Extensions 在 middleware 和 handlers 之间传递
///
/// # 使用场景
///
/// - Gateway: PEP 中间件验证 JWT 后创建并插入
/// - SSO: 认证成功后创建并插入
/// - Modules: 从请求中提取用户ID写入审计字段
///
/// # 示例
///
/// ```rust,ignore
/// use common::context::{RequestContext, RequestContextExt};
/// use actix_web::HttpRequest;
///
/// fn handler(req: HttpRequest) {
///     // 获取用户ID用于审计
///     let user_id = req.audit_user_id();
///     
///     // 或获取完整上下文
///     if let Some(ctx) = req.context() {
///         println!("User: {} (ID: {})", ctx.email, ctx.user_id);
///     }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestContext {
    /// 当前登录用户 ID (ZUID)
    #[serde(with = "crate::serde_zuid")]
    pub user_id: i64,
    /// 用户邮箱
    pub email: String,
    /// 用户显示名称
    pub username: String,
    /// 是否为超级管理员
    #[serde(default)]
    pub is_superuser: bool,
    /// 用户组织ID（可选）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "crate::serde_zuid::opt")]
    pub org_id: Option<i64>,
    /// 租户ID（可选，用于多租户场景）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "crate::serde_zuid::opt")]
    pub tenant_id: Option<i64>,
    /// NGAC 可见资源 ID 集合（行级安全 RLS）
    /// - `Some(vec![])` → 用户对所有资源都无权访问
    /// - `Some([1, 2, 3])` → 只能看到这些 ID
    /// - `None` → 未应用 RLS（admin / 列表端点未拦截）
    pub visible_resource_ids: HashMap<String, Vec<i64>>,
}

impl RequestContext {
    /// 创建新的请求上下文
    ///
    /// # Arguments
    /// * `user_id` - 用户ID (ZUID)
    /// * `email` - 用户邮箱
    pub fn new(user_id: i64, email: impl Into<String>) -> Self {
        Self {
            user_id,
            email: email.into(),
            username: String::new(),
            is_superuser: false,
            org_id: None,
            tenant_id: None,
            visible_resource_ids: HashMap::new(),
        }
    }

    /// Create request context with username
    pub fn with_username(
        user_id: i64,
        email: impl Into<String>,
        username: impl Into<String>,
    ) -> Self {
        Self {
            user_id,
            email: email.into(),
            username: username.into(),
            is_superuser: false,
            org_id: None,
            tenant_id: None,
            visible_resource_ids: HashMap::new(),
        }
    }

    /// 创建带管理员标识的请求上下文
    pub fn with_username_and_admin(
        user_id: i64,
        email: impl Into<String>,
        username: impl Into<String>,
        is_superuser: bool,
    ) -> Self {
        Self {
            user_id,
            email: email.into(),
            username: username.into(),
            is_superuser,
            org_id: None,
            tenant_id: None,
            visible_resource_ids: HashMap::new(),
        }
    }

    /// 创建带组织信息的请求上下文
    pub fn with_org(user_id: i64, email: impl Into<String>, org_id: i64) -> Self {
        Self {
            user_id,
            email: email.into(),
            username: String::new(),
            is_superuser: false,
            org_id: Some(org_id),
            tenant_id: None,
            visible_resource_ids: HashMap::new(),
        }
    }

    /// 创建带租户信息的请求上下文
    pub fn with_tenant(user_id: i64, email: impl Into<String>, tenant_id: i64) -> Self {
        Self {
            user_id,
            email: email.into(),
            username: String::new(),
            is_superuser: false,
            org_id: None,
            tenant_id: Some(tenant_id),
            visible_resource_ids: HashMap::new(),
        }
    }

    /// 从 HttpRequest 中提取请求上下文
    ///
    /// # Returns
    /// - `Some(RequestContext)` 如果请求已认证
    /// - `None` 如果请求未认证（无扩展信息）
    pub fn from_request(req: &HttpRequest) -> Option<Self> {
        req.extensions().get::<Self>().cloned()
    }

    /// 获取用户 ID（用于审计字段）
    ///
    /// # Returns
    /// - `Some(user_id)` 如果用户已登录
    /// - `None` 如果未登录（系统操作）
    pub fn audit_user_id(&self) -> Option<i64> {
        Some(self.user_id)
    }

    /// 设置组织ID
    pub fn set_org_id(&mut self, org_id: i64) {
        self.org_id = Some(org_id);
    }

    /// 设置租户ID
    pub fn set_tenant_id(&mut self, tenant_id: i64) {
        self.tenant_id = Some(tenant_id);
    }

    /// 设置指定资源类型的可见 ID 集合（NGAC RLS）
    /// - `ids = []` → 用户无任何资源的访问权
    /// - `ids = [1, 2, 3]` → 用户仅能看到这些 ID
    pub fn set_visible_resource_ids(&mut self, resource_type: impl Into<String>, ids: Vec<i64>) {
        self.visible_resource_ids.insert(resource_type.into(), ids);
    }

    /// 获取指定资源类型的可见 ID 集合
    pub fn get_visible_resource_ids(&self, resource_type: &str) -> Option<&Vec<i64>> {
        self.visible_resource_ids.get(resource_type)
    }
}

/// 便捷 trait 用于在 HttpRequest 上操作 RequestContext
///
/// # Example
/// ```rust
/// use actix_web::HttpRequest;
/// use common::context::RequestContextExt;
///
/// async fn handler(req: HttpRequest) {
///     // 快速获取用户ID用于审计
///     let user_id = req.audit_user_id();
///     
///     // 检查是否有上下文
///     if let Some(ctx) = req.context() {
///         // 处理认证用户请求
///     } else {
///         // 处理匿名请求
///     }
/// }
/// ```
pub trait RequestContextExt {
    /// 获取请求上下文
    fn context(&self) -> Option<RequestContext>;

    /// 获取用户 ID（用于审计字段）
    fn audit_user_id(&self) -> Option<i64>;

    /// 获取用户邮箱
    fn user_email(&self) -> Option<String>;

    /// 检查是否有上下文（是否已认证）
    fn is_authenticated(&self) -> bool;
}

/// 系统保留身份 ID：系统自动操作（SLA 自动驳回、流程推进、门禁自动执行等）的审计归属。
///
/// id=1 位于系统保留小整数区（1-999）；业务实体 id 由 `gen_next_uid` 生成（≥2^48），
/// `auth_users` 亦为 ZUID 大整数，与 SYSTEM_USER_ID 无冲突。
pub const SYSTEM_USER_ID: i64 = 1;

impl RequestContextExt for HttpRequest {
    fn context(&self) -> Option<RequestContext> {
        RequestContext::from_request(self)
    }

    fn audit_user_id(&self) -> Option<i64> {
        self.context().and_then(|ctx| ctx.audit_user_id())
    }

    fn user_email(&self) -> Option<String> {
        self.context().map(|ctx| ctx.email)
    }

    fn is_authenticated(&self) -> bool {
        self.context().is_some()
    }
}

/// 从请求扩展中提取用户ID的辅助函数
///
/// 这是一个便捷的独立函数，用于在不导入 trait 的情况下快速获取用户ID
///
/// # Example
/// ```rust
/// use actix_web::HttpRequest;
/// use common::context::extract_user_id;
///
/// async fn handler(req: HttpRequest) {
///     let user_id = extract_user_id(&req);
///     // user_id: Option<i64>
/// }
/// ```
pub fn extract_user_id(req: &HttpRequest) -> Option<i64> {
    req.extensions()
        .get::<RequestContext>()
        .map(|ctx| ctx.user_id)
        .or_else(|| req.extensions().get::<i64>().copied())
}

/// 要求用户必须已认证，否则返回 Unauthorized 错误
pub fn require_auth(req: &HttpRequest) -> Result<i64, crate::AliothError> {
    extract_user_id(req)
        .ok_or_else(|| crate::AliothError::Unauthorized("Authentication required".to_string()))
}

/// 从请求扩展中提取用户邮箱的辅助函数
pub fn extract_user_email(req: &HttpRequest) -> Option<String> {
    req.extensions()
        .get::<RequestContext>()
        .map(|ctx| ctx.email.clone())
}

/// 从请求扩展中提取登录账号（username）的辅助函数
///
/// 用于需要把「操作人账号」上送外部系统的场景（如 FSSC creator 必须为苍穹人员编码）。
pub fn extract_username(req: &HttpRequest) -> Option<String> {
    req.extensions()
        .get::<RequestContext>()
        .map(|ctx| ctx.username.clone())
}

/// 审计字段结构体
///
/// 用于 Repository 层统一处理审计字段
#[derive(Debug, Clone, Default)]
pub struct AuditFields {
    /// 创建者ID
    pub created_by_id: Option<i64>,
    /// 更新者ID
    pub updated_by_id: Option<i64>,
}

impl AuditFields {
    /// 从请求上下文中创建审计字段
    pub fn from_request(req: &HttpRequest) -> Self {
        let user_id = extract_user_id(req);
        Self {
            created_by_id: user_id,
            updated_by_id: user_id,
        }
    }

    /// 仅设置更新者（用于更新操作）
    pub fn for_update(req: &HttpRequest) -> Self {
        Self {
            created_by_id: None,
            updated_by_id: extract_user_id(req),
        }
    }

    /// 创建新的审计字段（用于创建操作）
    pub fn for_create(req: &HttpRequest) -> Self {
        let user_id = extract_user_id(req);
        Self {
            created_by_id: user_id,
            updated_by_id: user_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_context_new() {
        let ctx = RequestContext::new(12345, "user@example.com");
        assert_eq!(ctx.user_id, 12345);
        assert_eq!(ctx.email, "user@example.com");
        assert!(ctx.org_id.is_none());
        assert!(ctx.tenant_id.is_none());
    }

    #[test]
    fn test_request_context_with_username() {
        let ctx = RequestContext::with_username(2, "user@example.com", "testuser");
        assert_eq!(ctx.username, "testuser");
    }

    #[test]
    fn test_request_context_with_org() {
        let ctx = RequestContext::with_org(12345, "user@example.com", 100);
        assert_eq!(ctx.org_id, Some(100));
    }

    #[test]
    fn test_request_context_with_tenant() {
        let ctx = RequestContext::with_tenant(12345, "user@example.com", 200);
        assert_eq!(ctx.tenant_id, Some(200));
    }

    #[test]
    fn test_audit_user_id() {
        let ctx = RequestContext::new(42, "user@example.com");
        assert_eq!(ctx.audit_user_id(), Some(42));
    }

    #[test]
    fn test_audit_fields() {
        let audit = AuditFields {
            created_by_id: Some(1),
            updated_by_id: Some(2),
        };
        assert_eq!(audit.created_by_id, Some(1));
        assert_eq!(audit.updated_by_id, Some(2));
    }

    #[test]
    fn test_audit_fields_default() {
        let audit = AuditFields::default();
        assert!(audit.created_by_id.is_none());
        assert!(audit.updated_by_id.is_none());
    }
}
