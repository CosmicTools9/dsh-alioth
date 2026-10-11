//! Admin 用户 CRUD handlers
//!
//! `GET/POST /api/admin/users`、`GET/PUT/DELETE /api/admin/users/{id}`、
//! enable / unlock / reset-password 子端点。

use actix_web::{web, HttpRequest, HttpResponse};
use common::search::like_pattern;
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
    /// LDAP 绑定原文（非 LDAP 账号为 null）；判据 = `ldap IS NOT NULL`
    pub ldap: Option<serde_json::Value>,
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
    // 搜索模式：空串保持 `$1 = ''` 守卫（不过滤；若空串也绑定转义模式则守卫失效，
    // NULL username/display_name 行会被误排除）；非空时绑定 LIKE 转义模式
    //（元字符 `\ % _` 按字面量匹配，单源 = `common::search::like_pattern`）。
    let q_pattern = if q.is_empty() {
        String::new()
    } else {
        like_pattern(q)
    };
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

    // 排序列白名单映射（ORDER BY 不可参数化——列名/方向禁止拼接用户输入；表达式
    // 均带表别名 u，与下方投影查询的 FROM 别名一致）
    let sort_col = match search.sort_by.as_deref() {
        Some("name") => "LOWER(COALESCE(u.display_name, u.username, ''))",
        Some("email") => "LOWER(COALESCE((SELECT e.email FROM isahl_auth.auth_user_emails e WHERE e.fk_user = u.id AND e.deleted_at IS NULL ORDER BY e.is_primary DESC, e.id LIMIT 1), ''))",
        Some("created_at") => "u.created_at",
        // status 排序与前端启用判定一致（is_active + status 合成布尔）
        Some("status") => "(u.is_active AND COALESCE(u.status::text, 'active') <> 'disabled')",
        Some("type") => "(u.ldap IS NOT NULL)",
        _ => "u.id",
    };
    let order_by = format!("{sort_col} {dir}, u.id {dir}");
    // 搜索/筛选条件走 NULL 短路（$='' 不过滤），单条 SQL 覆盖全部组合。
    // 注入审计（AssertSqlSafe 前置条件）：order_by 仅由 sort_col/dir 组成，
    // 二者均为上方 match 白名单产出的 &'static str，用户输入不进 SQL 文本。
    let users = sqlx::query_as::<_, UserResponse>(sqlx::AssertSqlSafe(format!(
        r#"
        SELECT u.id,
               COALESCE(u.display_name, u.username) AS name,
               u.username,
               (SELECT e.email FROM isahl_auth.auth_user_emails e
                 WHERE e.fk_user = u.id AND e.deleted_at IS NULL
                 ORDER BY e.is_primary DESC, e.id LIMIT 1) AS email,
               u.display_name,
               u.status::text AS status,
               u.is_active,
               u.ldap,
               u.created_at, u.updated_at
        FROM isahl_auth.auth_users u
        WHERE ($1 = '' OR u.username ILIKE $1 OR u.display_name ILIKE $1
               OR EXISTS (SELECT 1 FROM isahl_auth.auth_user_emails e
                          WHERE e.fk_user = u.id AND e.deleted_at IS NULL
                            AND e.email ILIKE $1))
          AND ($2 = '' OR ($2 = 'active' AND u.is_active AND COALESCE(u.status::text, 'active') <> 'disabled')
               OR ($2 = 'disabled' AND NOT (u.is_active AND COALESCE(u.status::text, 'active') <> 'disabled')))
          AND ($3 = '' OR ($3 = 'ldap' AND u.ldap IS NOT NULL)
               OR ($3 = 'local' AND u.ldap IS NULL))
        ORDER BY {order_by}
        LIMIT $4 OFFSET $5
        "#,
    )))
    .bind(q_pattern.as_str())
    .bind(status)
    .bind(user_type)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool.get_ref())
    .await;

    let total: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM isahl_auth.auth_users u
        WHERE ($1 = '' OR u.username ILIKE $1 OR u.display_name ILIKE $1
               OR EXISTS (SELECT 1 FROM isahl_auth.auth_user_emails e
                          WHERE e.fk_user = u.id AND e.deleted_at IS NULL
                            AND e.email ILIKE $1))
          AND ($2 = '' OR ($2 = 'active' AND u.is_active AND COALESCE(u.status::text, 'active') <> 'disabled')
               OR ($2 = 'disabled' AND NOT (u.is_active AND COALESCE(u.status::text, 'active') <> 'disabled')))
          AND ($3 = '' OR ($3 = 'ldap' AND u.ldap IS NOT NULL)
               OR ($3 = 'local' AND u.ldap IS NULL))
        "#,
    )
    .bind(q_pattern.as_str())
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

    // 事务：用户行 + 首邮箱行（唯一存储 = auth_user_emails）原子落库
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("create_user: begin tx error: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to create user"}));
        }
    };

    // username 为唯一身份锚点；请求 `name` 值落 username（显示名读径 =
    // COALESCE(display_name, username)）
    let result = sqlx::query_as::<_, (i64,)>(
        r#"
        INSERT INTO isahl_auth.auth_users (username, password_hash, display_name, created_at, updated_at, created_by_id)
        VALUES ($1, $2, $3, NOW(), NOW(), $4)
        RETURNING id
        "#,
    )
    .bind(&body.name)
    .bind(&password_hash)
    .bind(&body.display_name)
    .bind(admin_id)
    .fetch_one(&mut *tx)
    .await;

    let id = match result {
        Ok((id,)) => id,
        Err(e) => {
            log::error!("create_user DB error: {}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to create user"}));
        }
    };

    // 邮箱唯一存储 = isahl_auth.auth_user_emails（首邮箱 is_primary = TRUE）
    if let Err(e) = sqlx::query(
        "INSERT INTO isahl_auth.auth_user_emails (fk_user, email, is_primary, created_at, updated_at) \
         VALUES ($1, $2, TRUE, NOW(), NOW()) \
         ON CONFLICT (fk_user, email) WHERE deleted_at IS NULL DO UPDATE \
             SET is_primary = TRUE, deleted_at = NULL, updated_at = NOW()",
    )
    .bind(id)
    .bind(&body.email)
    .execute(&mut *tx)
    .await
    {
        log::error!("create_user email insert error: {}", e);
        let _ = tx.rollback().await;
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": "Failed to create user"}));
    }

    // NGAC 基础身份接线（approval-endpoint-gates::user-ua-on-all-provisioning-channels）：
    // 管理员建号即挂 user UA（此前仅下次 Gateway 重启兜底）；与建号同事务，best-effort。
    let _ = common::registration_approval::ensure_user_ua(&mut tx, id).await;

    if let Err(e) = tx.commit().await {
        log::error!("create_user commit error: {}", e);
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": "Failed to create user"}));
    }

    HttpResponse::Created().json(serde_json::json!({
        "id": id.to_string(),
        "name": body.name,
        "email": body.email,
        "display_name": body.display_name,
    }))
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
        SELECT u.id,
               COALESCE(u.display_name, u.username) AS name,
               u.username,
               (SELECT e.email FROM isahl_auth.auth_user_emails e
                 WHERE e.fk_user = u.id AND e.deleted_at IS NULL
                 ORDER BY e.is_primary DESC, e.id LIMIT 1) AS email,
               u.display_name,
               u.status::text AS status,
               u.is_active,
               u.ldap,
               u.created_at, u.updated_at
        FROM isahl_auth.auth_users u
        WHERE u.id = $1
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
    // `name` 与 `display_name` 同落 display_name 列（显示名读径 = COALESCE(display_name,
    // username)）；两者均提供时以 display_name 优先。
    let display_name_new = body.display_name.as_deref().or(body.name.as_deref());
    // 静态分派（列集可枚举 ⇒ 不拼列位；`dynamic-table-name` 门禁要求编译期静态 SQL）：
    // 三个可选字段的 2^3 组合各对应一条完整静态语句（$1..$7 绑定位序不变）。
    let update_stmt: &'static str = match (
        display_name_new.is_some(),
        body.status.is_some(),
        body.is_active.is_some(),
    ) {
        (true, true, true) => "UPDATE isahl_auth.auth_users SET display_name = $2, status = $5::isahl_auth.auth_user_status, is_active = $6, updated_at = NOW(), updated_by_id = $7 WHERE id = $1",
        (true, true, false) => "UPDATE isahl_auth.auth_users SET display_name = $2, status = $5::isahl_auth.auth_user_status, updated_at = NOW(), updated_by_id = $7 WHERE id = $1",
        (true, false, true) => "UPDATE isahl_auth.auth_users SET display_name = $2, is_active = $6, updated_at = NOW(), updated_by_id = $7 WHERE id = $1",
        (true, false, false) => "UPDATE isahl_auth.auth_users SET display_name = $2, updated_at = NOW(), updated_by_id = $7 WHERE id = $1",
        (false, true, true) => "UPDATE isahl_auth.auth_users SET status = $5::isahl_auth.auth_user_status, is_active = $6, updated_at = NOW(), updated_by_id = $7 WHERE id = $1",
        (false, true, false) => "UPDATE isahl_auth.auth_users SET status = $5::isahl_auth.auth_user_status, updated_at = NOW(), updated_by_id = $7 WHERE id = $1",
        (false, false, true) => "UPDATE isahl_auth.auth_users SET is_active = $6, updated_at = NOW(), updated_by_id = $7 WHERE id = $1",
        (false, false, false) => {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"error": "No fields to update"}));
        }
    };

    // 绑定位序保持 $1..$7 不变（$3 = email、$4 = display_name 由同列/邮箱表路径消费）
    let mut q = sqlx::query(AssertSqlSafe(update_stmt)).bind(user_id_param);
    q = q.bind(display_name_new);
    q = q.bind(&body.email);
    q = q.bind(&body.display_name);
    q = q.bind(&body.status);
    q = q.bind(body.is_active);
    q = q.bind(admin_id);

    let result = q.execute(pool.get_ref()).await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            // 邮箱唯一存储 = isahl_auth.auth_user_emails：先降级旧主邮箱，再落新主邮箱
            if let Some(email) = body.email.as_deref() {
                if let Err(e) = write_primary_email(pool.get_ref(), user_id_param, email).await {
                    log::error!("update_user email write error: {}", e);
                    return HttpResponse::InternalServerError()
                        .json(serde_json::json!({"error": "Failed to update user email"}));
                }
            }
            HttpResponse::Ok().json(
                serde_json::json!({"status": "updated", "user_id": user_id_param.to_string()}),
            )
        }
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "User not found"})),
        Err(e) => {
            log::error!("update_user DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to update user"}))
        }
    }
}

/// 写账号主邮箱（唯一存储 = `isahl_auth.auth_user_emails`）：
/// 先降级旧主邮箱，再 upsert 目标邮箱行并置 `is_primary = TRUE`。
async fn write_primary_email(pool: &PgPool, user_id: i64, email: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE isahl_auth.auth_user_emails SET is_primary = FALSE, updated_at = NOW() \
         WHERE fk_user = $1 AND deleted_at IS NULL AND is_primary AND email <> $2",
    )
    .bind(user_id)
    .bind(email)
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO isahl_auth.auth_user_emails (fk_user, email, is_primary, created_at, updated_at) \
         VALUES ($1, $2, TRUE, NOW(), NOW()) \
         ON CONFLICT (fk_user, email) WHERE deleted_at IS NULL DO UPDATE \
             SET is_primary = TRUE, deleted_at = NULL, updated_at = NOW()",
    )
    .bind(user_id)
    .bind(email)
    .execute(pool)
    .await?;

    Ok(())
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
