//! Admin 用户 CRUD handlers
//!
//! `GET/POST /api/admin/users`、`GET/PUT/DELETE /api/admin/users/{id}`、
//! enable / unlock / reset-password 子端点。

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};

/// User response (safe fields only, no password_hash)
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct UserResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: Option<String>,
    pub username: Option<String>,
    /// 服务用户（user_type='service'）email 为 NULL（避免占用邮箱），
    /// 故必须 Option——否则 list_users/get_user 遇服务用户即解码 500。
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub status: Option<String>,
    pub is_active: Option<bool>,
    pub is_ldap_user: Option<bool>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Create user request body
#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub name: String,
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

/// Update user request body — all fields optional, only provided ones are updated
#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub is_active: Option<bool>,
    pub status: Option<String>,
}

/// Admin reset password request
#[derive(Debug, Deserialize)]
pub struct AdminResetPasswordRequest {
    pub new_password: String,
}

use super::{require_admin, PaginationMeta, PaginationParams};
use crate::auth::AuthState;

// ============================================================================
// User CRUD
// ============================================================================

/// GET /api/admin/users?limit=50&offset=0&q=search&status=active|disabled&type=local|ldap
///   &sort_by=id|name|email|created_at|status|type&order=asc|desc
/// List users with pagination, free-text search, status/type filters and whitelisted
/// server-side sorting (default limit 50, max 500; invalid filter/sort values fall back
/// to no-filter / id ASC).
pub async fn list_users(
    req: HttpRequest,
    query: web::Query<PaginationParams>,
    search: web::Query<UserSearchParams>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let _ = admin_id;

    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let offset = query.offset.unwrap_or(0).max(0);
    let q = search.q.as_deref().unwrap_or("").trim();
    // 非法筛选值归一为空串（不过滤）——SQL 侧以 $='' 短路，避免整表被排除
    // （等价于原 `match … { Some(s @ ("active" | "disabled")) => s, _ => "" }`）
    let status = search
        .status
        .as_deref()
        .filter(|s| matches!(*s, "active" | "disabled"))
        .unwrap_or("");
    let user_type = search
        .user_type
        .as_deref()
        .filter(|t| matches!(*t, "local" | "ldap"))
        .unwrap_or("");
    let dir = match search.order.as_deref() {
        Some("desc") => "DESC",
        _ => "ASC",
    };

    // 排序列白名单映射（ORDER BY 不可参数化——列名/方向禁止拼接用户输入）
    let sort_col = match search.sort_by.as_deref() {
        Some("name") => "LOWER(COALESCE(display_name, name, username, ''))",
        Some("email") => "LOWER(COALESCE(email, ''))",
        Some("created_at") => "created_at",
        // status 排序与前端启用判定一致（is_active + status 合成布尔）
        Some("status") => "(is_active AND COALESCE(status, 'active') <> 'disabled')",
        Some("type") => "COALESCE(is_ldap_user, false)",
        _ => "id",
    };
    let order_by = format!("{sort_col} {dir}, id {dir}");
    // 搜索/筛选条件走 NULL 短路（$='' 不过滤），单条 SQL 覆盖全部组合。
    // 注入审计（AssertSqlSafe 前置条件）：order_by 仅由 sort_col/dir 组成，
    // 二者均为上方 match 白名单产出的 &'static str，用户输入不进 SQL 文本。
    let users = sqlx::query_as::<_, UserResponse>(sqlx::AssertSqlSafe(format!(
        r#"
        SELECT id, name, username, email, display_name, status, is_active, is_ldap_user, created_at, updated_at
        FROM isahl_auth.auth_users
        WHERE ($1 = '' OR name ILIKE '%' || $1 || '%' OR email ILIKE '%' || $1 || '%'
               OR username ILIKE '%' || $1 || '%' OR display_name ILIKE '%' || $1 || '%')
          AND ($2 = '' OR ($2 = 'active' AND is_active AND COALESCE(status, 'active') <> 'disabled')
               OR ($2 = 'disabled' AND NOT (is_active AND COALESCE(status, 'active') <> 'disabled')))
          AND ($3 = '' OR ($3 = 'ldap' AND COALESCE(is_ldap_user, false))
               OR ($3 = 'local' AND NOT COALESCE(is_ldap_user, false)))
        ORDER BY {order_by}
        LIMIT $4 OFFSET $5
        "#,
    )))
    .bind(q)
    .bind(status)
    .bind(user_type)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool.get_ref())
    .await;

    let total: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM isahl_auth.auth_users
        WHERE ($1 = '' OR name ILIKE '%' || $1 || '%' OR email ILIKE '%' || $1 || '%'
               OR username ILIKE '%' || $1 || '%' OR display_name ILIKE '%' || $1 || '%')
          AND ($2 = '' OR ($2 = 'active' AND is_active AND COALESCE(status, 'active') <> 'disabled')
               OR ($2 = 'disabled' AND NOT (is_active AND COALESCE(status, 'active') <> 'disabled')))
          AND ($3 = '' OR ($3 = 'ldap' AND COALESCE(is_ldap_user, false))
               OR ($3 = 'local' AND NOT COALESCE(is_ldap_user, false)))
        "#,
    )
    .bind(q)
    .bind(status)
    .bind(user_type)
    .fetch_one(pool.get_ref())
    .await
    .unwrap_or(0);

    match users {
        Ok(rows) => HttpResponse::Ok().json(serde_json::json!({
            "users": rows,
            "pagination": PaginationMeta { limit, offset, total },
        })),
        Err(e) => {
            log::error!("list_users DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to list users"}))
        }
    }
}

/// Query params for search / filter / sort（全部可选，缺省 = 不过滤、按 id ASC）
#[derive(Debug, Deserialize, Default)]
pub struct UserSearchParams {
    #[serde(default)]
    pub q: Option<String>,
    /// 状态筛选：active | disabled（非法值视为不过滤）
    #[serde(default)]
    pub status: Option<String>,
    /// 类型筛选：local | ldap（query 参数名 type；非法值视为不过滤）
    #[serde(rename = "type", default)]
    pub user_type: Option<String>,
    /// 排序列：id | name | email | created_at | status | type（非法值回退 id）
    #[serde(default)]
    pub sort_by: Option<String>,
    /// 排序方向：asc | desc（非法值回退 asc）
    #[serde(default)]
    pub order: Option<String>,
}

/// POST /api/admin/users
/// Create a new user.
pub async fn create_user(
    req: HttpRequest,
    body: web::Json<CreateUserRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let password_hash =
        match crate::auth::password::hash_password_async(body.password.clone()).await {
            Ok(h) => h,
            Err(e) => {
                log::error!("create_user: password hash failed: {:?}", e);
                return HttpResponse::InternalServerError()
                    .json(serde_json::json!({"error": "Failed to hash password"}));
            }
        };

    let result = sqlx::query_as::<_, (i64,)>(
        r#"
        INSERT INTO isahl_auth.auth_users (name, email, password_hash, display_name, created_at, updated_at, created_by_id)
        VALUES ($1, $2, $3, $4, NOW(), NOW(), $5)
        RETURNING id
        "#,
    )
    .bind(&body.name)
    .bind(&body.email)
    .bind(&password_hash)
    .bind(&body.display_name)
    .bind(admin_id)
    .fetch_one(pool.get_ref())
    .await;

    match result {
        Ok((id,)) => HttpResponse::Created().json(serde_json::json!({
            "id": id.to_string(),
            "name": body.name,
            "email": body.email,
            "display_name": body.display_name,
        })),
        Err(e) => {
            log::error!("create_user DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to create user"}))
        }
    }
}

/// GET /api/admin/users/{id}
/// Get a single user's details.
pub async fn get_user(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let user_id_param = path.into_inner();

    let user = sqlx::query_as::<_, UserResponse>(
        r#"
        SELECT id, name, username, email, display_name, status, is_active, is_ldap_user, created_at, updated_at
        FROM isahl_auth.auth_users
        WHERE id = $1
        "#,
    )
    .bind(user_id_param)
    .fetch_optional(pool.get_ref())
    .await;

    match user {
        Ok(Some(u)) => HttpResponse::Ok().json(u),
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({"error": "User not found"})),
        Err(e) => {
            log::error!("get_user DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to get user"}))
        }
    }
}

/// PUT /api/admin/users/{id}
/// Update user fields. Only provided fields are updated.
pub async fn update_user(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateUserRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let user_id_param = path.into_inner();

    // Build dynamic UPDATE — only set fields that were provided
    let mut sets: Vec<&str> = Vec::new();
    if body.name.is_some() {
        sets.push("name = $2");
    }
    if body.email.is_some() {
        sets.push("email = $3");
    }
    if body.display_name.is_some() {
        sets.push("display_name = $4");
    }
    if body.status.is_some() {
        sets.push("status = $5");
    }
    if body.is_active.is_some() {
        sets.push("is_active = $6");
    }
    if sets.is_empty() {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"error": "No fields to update"}));
    }
    sets.push("updated_at = NOW()");
    sets.push("updated_by_id = $7");
    let sql = format!(
        "UPDATE isahl_auth.auth_users SET {} WHERE id = $1",
        sets.join(", ")
    );

    let mut q = sqlx::query(AssertSqlSafe(sql.as_str())).bind(user_id_param);
    q = q.bind(&body.name);
    q = q.bind(&body.email);
    q = q.bind(&body.display_name);
    q = q.bind(&body.status);
    q = q.bind(body.is_active);
    q = q.bind(admin_id);

    let result = q.execute(pool.get_ref()).await;

    match result {
        Ok(r) if r.rows_affected() > 0 => HttpResponse::Ok()
            .json(serde_json::json!({"status": "updated", "user_id": user_id_param.to_string()})),
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "User not found"})),
        Err(e) => {
            log::error!("update_user DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to update user"}))
        }
    }
}

/// DELETE /api/admin/users/{id}
/// Disable a user (set is_active = false). Soft-disable preserves audit history.
pub async fn disable_user(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let user_id_param = path.into_inner();

    let result = sqlx::query(
        r#"
        UPDATE isahl_auth.auth_users
        SET is_active = false, status = 'disabled', updated_at = NOW(), updated_by_id = $2
        WHERE id = $1
        "#,
    )
    .bind(user_id_param)
    .bind(admin_id)
    .execute(pool.get_ref())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            // Revoke all active sessions for the disabled user
            let _ = sqlx::query(
                "UPDATE isahl_auth.sso_sessions SET status = 'revoked' WHERE user_id = $1 AND status = 'active'",
            )
            .bind(user_id_param)
            .execute(pool.get_ref())
            .await;
            HttpResponse::Ok().json(
                serde_json::json!({"status": "disabled", "user_id": user_id_param.to_string()}),
            )
        }
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "User not found"})),
        Err(e) => {
            log::error!("disable_user DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to disable user"}))
        }
    }
}

/// POST /api/admin/users/{id}/enable
/// Re-enable a disabled user.
pub async fn enable_user(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let user_id_param = path.into_inner();

    let result = sqlx::query(
        r#"
        UPDATE isahl_auth.auth_users
        SET is_active = true, status = 'active', updated_at = NOW(), updated_by_id = $2
        WHERE id = $1
        "#,
    )
    .bind(user_id_param)
    .bind(admin_id)
    .execute(pool.get_ref())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => HttpResponse::Ok()
            .json(serde_json::json!({"status": "enabled", "user_id": user_id_param.to_string()})),
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "User not found"})),
        Err(e) => {
            log::error!("enable_user DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to enable user"}))
        }
    }
}

/// POST /api/admin/users/{id}/reset-password
/// Admin-initiated password reset.
pub async fn admin_reset_password(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<AdminResetPasswordRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let user_id_param = path.into_inner();

    let password_hash =
        match crate::auth::password::hash_password_async(body.new_password.clone()).await {
            Ok(h) => h,
            Err(e) => {
                log::error!("admin_reset_password: hash failed: {:?}", e);
                return HttpResponse::InternalServerError()
                    .json(serde_json::json!({"error": "Failed to hash password"}));
            }
        };

    let result = sqlx::query(
        r#"
        UPDATE isahl_auth.auth_users
        SET password_hash = $1, updated_at = NOW(), updated_by_id = $3
        WHERE id = $2
        "#,
    )
    .bind(&password_hash)
    .bind(user_id_param)
    .bind(admin_id)
    .execute(pool.get_ref())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            // Invalidate refresh tokens for the user (force re-login)
            let _ = sqlx::query(
                "UPDATE isahl_auth.refresh_tokens SET revoked = true, updated_at = NOW() WHERE user_id = $1",
            )
            .bind(user_id_param)
            .execute(pool.get_ref())
            .await;
            HttpResponse::Ok()
                .json(serde_json::json!({"status": "password_reset", "user_id": user_id_param.to_string()}))
        }
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "User not found"})),
        Err(e) => {
            log::error!("admin_reset_password DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to reset password"}))
        }
    }
}

/// POST /api/admin/users/{id}/unlock
///
/// 管理员解锁被锁定的账户：清零 `failed_login_attempts` 并清除 `locked_until`
/// （SECURITY_SPEC §5 锁定机制的对应解锁入口，受 NGAC admin 属性保护）。
pub async fn admin_unlock_account(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let target_id = path.into_inner();

    let result = sqlx::query(
        "UPDATE isahl_auth.auth_users \
         SET failed_login_attempts = 0, locked_until = NULL, updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(target_id)
    .execute(pool.get_ref())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => HttpResponse::Ok().json(serde_json::json!({
            "status": "unlocked",
            "user_id": target_id.to_string(),
        })),
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "User not found"})),
        Err(e) => {
            log::error!("admin_unlock_account DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to unlock account"}))
        }
    }
}
