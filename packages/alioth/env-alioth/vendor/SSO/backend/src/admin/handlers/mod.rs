//! SSO Admin API handlers
//!
//! All handlers require the caller to have the 'admin' NGAC user attribute.
//! Authentication is via JWT from Cookie or Authorization header.
//! Pagination is supported on list endpoints.
//!
//! M26 结构性拆分：按业务域拆为子模块（纯重构，公开路径 `admin::handlers::*` 不变）：
//! - `users`：用户 CRUD
//! - `ngac`：NGAC 用户/对象属性、association、prohibition
//! - `identity`：实名认证审核
//! - `providers`：身份提供方管理
//! - `sessions`：会话管理

use actix_web::{HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::auth::jwt::validate_access_token;
use crate::auth::AuthState;

pub mod bootstrap;
pub mod identity;
pub mod ngac;
pub mod providers;
pub mod sessions;
pub mod users;

// Re-export：保持 `admin::handlers::<name>` 与路由表引用路径稳定。
pub use bootstrap::*;
pub use identity::*;
pub use ngac::*;
pub use providers::*;
pub use sessions::*;
pub use users::*;

/// Pagination query parameters
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub limit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub offset: Option<i64>,
}

/// Pagination metadata returned in list responses
#[derive(Debug, Serialize)]
struct PaginationMeta {
    #[serde(with = "common::serde_zuid")]
    pub limit: i64,
    #[serde(with = "common::serde_zuid")]
    pub offset: i64,
    #[serde(with = "common::serde_zuid")]
    pub total: i64,
}

/// Validate admin access for the current request.
/// Extracts JWT, decodes it, and verifies the caller has the 'admin' NGAC user attribute.
/// Returns the authenticated user's ID on success, or an error HttpResponse.
///
/// `pub`（refactor-ngac-admin-nl-graph D2）：Gateway 原生 nl-assist 端点进程内复用
/// 同一 admin 门控语义——禁止第二份实现。
pub async fn require_admin(
    req: &HttpRequest,
    pool: &PgPool,
    auth_state: &AuthState,
) -> Result<i64, HttpResponse> {
    require_admin_claims(req, pool, auth_state)
        .await
        .map(|(id, _)| id)
}

/// `require_admin` 的 claims 携带变体：返回 (user_id, Claims)。
/// 策略变更审计（audit_writer）需要 JWT `sid` 填 `session_id` 列。
pub(crate) async fn require_admin_claims(
    req: &HttpRequest,
    pool: &PgPool,
    auth_state: &AuthState,
) -> Result<(i64, crate::auth::jwt::Claims), HttpResponse> {
    let claims = validate_access_token(req, &auth_state.verification_keys())
        .await
        .map_err(|e| {
            log::error!("Admin auth: token validation failed: {}", e);
            HttpResponse::Unauthorized()
                .json(serde_json::json!({"error": "Invalid or missing authentication token"}))
        })?;

    let user_id: i64 = claims.sub.parse().map_err(|_| {
        HttpResponse::Unauthorized()
            .json(serde_json::json!({"error": "Invalid user ID in token claims"}))
    })?;

    let is_admin: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM isahl_auth.ngac_user_rr_attribute ur
            JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute
            WHERE ur.fk_user = $1
              AND ua.o_name = 'admin'
              AND ur.deleted_at IS NULL
        )
        "#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        log::error!("Admin auth: DB query failed: {}", e);
        HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": "Database query failed"}))
    })?;

    if !is_admin {
        return Err(
            HttpResponse::Forbidden().json(serde_json::json!({"error": "Admin access required"}))
        );
    }

    Ok((user_id, claims))
}
