//! SCIM User 行映射与 CRUD 端点（`/scim/v2/Users`）。
//! 用户映射到 `isahl_auth.auth_users`。

use actix_web::{web, HttpRequest, HttpResponse};
use sqlx::PgPool;

use super::require_scim_token;
use crate::scim::models::*;

// ── User 行映射 ───────────────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct UserRow {
    id: i64,
    name: String,
    #[sqlx(default)]
    username: Option<String>,
    #[sqlx(default)]
    email: Option<String>,
    #[sqlx(default)]
    display_name: Option<String>,
    #[sqlx(default)]
    status: Option<String>,
    #[sqlx(default)]
    is_active: Option<bool>,
    #[sqlx(default)]
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[sqlx(default)]
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
    #[sqlx(default)]
    external_id: Option<String>,
}

fn to_scim_user(r: &UserRow) -> ScimUser {
    let id_str = r.id.to_string();
    let email = r
        .email
        .clone()
        .or_else(|| r.username.clone())
        .unwrap_or_else(|| r.name.clone());
    let active = r.is_active.unwrap_or(false) && r.status.as_deref() != Some("disabled");
    ScimUser {
        schemas: Some(vec![
            "urn:ietf:params:scim:schemas:core:2.0:User".to_string()
        ]),
        id: Some(id_str.clone()),
        external_id: r.external_id.clone(),
        user_name: Some(email.clone()),
        name: if r.display_name.is_some() {
            Some(ScimName {
                formatted: r.display_name.clone(),
                given_name: r.display_name.clone(),
                family_name: None,
            })
        } else {
            None
        },
        display_name: r.display_name.clone(),
        emails: Some(vec![ScimEmail {
            email_type: Some("work".to_string()),
            value: Some(email.clone()),
            primary: Some(true),
        }]),
        active: Some(active),
        groups: None,
        meta: Some(ScimMeta {
            resource_type: Some("User".to_string()),
            created: r.created_at.map(|t| t.to_rfc3339()),
            last_modified: r.updated_at.map(|t| t.to_rfc3339()),
            location: Some(format!("/scim/v2/Users/{}", id_str)),
        }),
    }
}

fn parse_user_name_filter(filter: &str) -> Option<String> {
    let filter = filter.trim();
    if let Some(rest) = filter.strip_prefix("userName eq") {
        let rest = rest.trim();
        if let Some(stripped) = rest.strip_prefix('"').and_then(|s| s.rsplit_once('"')) {
            return Some(stripped.0.to_string());
        }
    }
    None
}

// ── User CRUD ─────────────────────────────────────────────────────────────────

/// GET /scim/v2/Users —— list / search（filter + 分页）。
pub async fn list_users(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    if let Err(resp) = require_scim_token(&req) {
        return resp;
    }

    let filter_value = query.get("filter").and_then(|f| parse_user_name_filter(f));
    let count: i64 = query
        .get("count")
        .and_then(|c| c.parse::<i64>().ok())
        .unwrap_or(100)
        .clamp(1, 200);
    let start_index: i64 = query
        .get("startIndex")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(1)
        .max(1);
    let offset = start_index - 1;

    // 静态 SQL（项目 sqlx 封装要求字面量，禁止 format! 动态拼接）。
    const LIST_SQL_FILTER: &str =
        "SELECT id, name, username, email, display_name, status, is_active, created_at, updated_at, \
                settings->>'scim_external_id' AS external_id \
         FROM isahl_auth.auth_users \
         WHERE is_active = true AND (email = $1 OR name = $1 OR username = $1) \
         ORDER BY id LIMIT $2 OFFSET $3";
    const LIST_SQL_ALL: &str =
        "SELECT id, name, username, email, display_name, status, is_active, created_at, updated_at, \
                settings->>'scim_external_id' AS external_id \
         FROM isahl_auth.auth_users \
         WHERE is_active = true ORDER BY id LIMIT $1 OFFSET $2";
    const COUNT_SQL_FILTER: &str =
        "SELECT COUNT(*) FROM isahl_auth.auth_users WHERE is_active = true AND (email = $1 OR name = $1 OR username = $1)";
    const COUNT_SQL_ALL: &str = "SELECT COUNT(*) FROM isahl_auth.auth_users WHERE is_active = true";

    let total: i64 = {
        let q = if let Some(v) = &filter_value {
            sqlx::query_scalar::<_, i64>(COUNT_SQL_FILTER).bind(v)
        } else {
            sqlx::query_scalar::<_, i64>(COUNT_SQL_ALL)
        };
        match q.fetch_one(pool.get_ref()).await {
            Ok(t) => t,
            Err(e) => {
                log::error!("scim list_users count error: {}", e);
                return error_response(
                    actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error",
                );
            }
        }
    };

    let rows = {
        let q = if let Some(v) = &filter_value {
            sqlx::query_as::<_, UserRow>(LIST_SQL_FILTER)
                .bind(v)
                .bind(count)
                .bind(offset)
        } else {
            sqlx::query_as::<_, UserRow>(LIST_SQL_ALL)
                .bind(count)
                .bind(offset)
        };
        match q.fetch_all(pool.get_ref()).await {
            Ok(r) => r,
            Err(e) => {
                log::error!("scim list_users error: {}", e);
                return error_response(
                    actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error",
                );
            }
        }
    };

    let resources: Vec<ScimUser> = rows.iter().map(to_scim_user).collect();
    let items_per_page = if total == 0 { 0 } else { resources.len() };

    HttpResponse::Ok().json(ListResponse {
        schemas: vec!["urn:ietf:params:scim:api:messages:2.0:ListResponse".to_string()],
        totalResults: total as usize,
        startIndex: start_index as usize,
        itemsPerPage: items_per_page,
        Resources: resources,
    })
}

pub async fn create_user(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<ScimUser>,
) -> HttpResponse {
    if let Err(resp) = require_scim_token(&req) {
        return resp;
    }

    let email = match body.user_name.as_ref().filter(|s| !s.is_empty()) {
        Some(e) => e.clone(),
        None => {
            return error_response(
                actix_web::http::StatusCode::BAD_REQUEST,
                "userName is required",
            )
        }
    };

    let display_name = body.display_name.clone();
    let active = body.active.unwrap_or(true);
    let status_str = if active { "active" } else { "disabled" };
    let settings = match &body.external_id {
        Some(ext) => serde_json::json!({ "scim_external_id": ext }),
        None => serde_json::json!({}),
    };

    // 幂等：已存在（按 email）则更新，否则插入。
    let existing: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM isahl_auth.auth_users WHERE email = $1 AND is_active = true",
    )
    .bind(&email)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);

    let user_id = if let Some((id,)) = existing {
        let _ = sqlx::query(
            "UPDATE isahl_auth.auth_users \
             SET display_name = COALESCE($2, display_name), status = $3, is_active = $4, \
                 updated_at = NOW() WHERE id = $1",
        )
        .bind(&display_name)
        .bind(status_str)
        .bind(active)
        .bind(id)
        .execute(pool.get_ref())
        .await;
        id
    } else {
        let row: (i64,) = match sqlx::query_as(
            "INSERT INTO isahl_auth.auth_users \
             (name, username, email, password_hash, display_name, status, is_active, settings, created_at, updated_at) \
             VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, NOW(), NOW()) \
             RETURNING id",
        )
        .bind(&email)
        .bind(&email)
        .bind(&email)
        .bind(&display_name)
        .bind(status_str)
        .bind(active)
        .bind(&settings)
        .fetch_one(pool.get_ref())
        .await
        {
            Ok(r) => r,
            Err(e) => {
                log::error!("scim create_user insert error: {}", e);
                return error_response(
                    actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to create user",
                );
            }
        };
        row.0
    };

    // 读取最新行以构造完整 SCIM 表示
    let row: Option<UserRow> = sqlx::query_as(
        "SELECT id, name, username, email, display_name, status, is_active, created_at, updated_at, \
                settings->>'scim_external_id' AS external_id \
         FROM isahl_auth.auth_users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);

    match row {
        Some(r) => HttpResponse::Created().json(to_scim_user(&r)),
        None => error_response(
            actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to read created user",
        ),
    }
}

pub async fn get_user(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<String>,
) -> HttpResponse {
    if let Err(resp) = require_scim_token(&req) {
        return resp;
    }
    let id: i64 = match path.into_inner().parse() {
        Ok(i) => i,
        Err(_) => return error_response(actix_web::http::StatusCode::BAD_REQUEST, "Invalid id"),
    };
    let row: Option<UserRow> = sqlx::query_as(
        "SELECT id, name, username, email, display_name, status, is_active, created_at, updated_at, \
                settings->>'scim_external_id' AS external_id \
         FROM isahl_auth.auth_users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);

    match row {
        Some(r) => HttpResponse::Ok().json(to_scim_user(&r)),
        None => error_response(actix_web::http::StatusCode::NOT_FOUND, "User not found"),
    }
}

pub async fn replace_user(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<String>,
    body: web::Json<ScimUser>,
) -> HttpResponse {
    if let Err(resp) = require_scim_token(&req) {
        return resp;
    }
    let id: i64 = match path.into_inner().parse() {
        Ok(i) => i,
        Err(_) => return error_response(actix_web::http::StatusCode::BAD_REQUEST, "Invalid id"),
    };

    let exists: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM isahl_auth.auth_users WHERE id = $1")
            .bind(id)
            .fetch_optional(pool.get_ref())
            .await
            .unwrap_or(None);
    if exists.is_none() {
        return error_response(actix_web::http::StatusCode::NOT_FOUND, "User not found");
    }

    let active = body.active.unwrap_or(true);
    let status_str = if active { "active" } else { "disabled" };
    let _ = sqlx::query(
        "UPDATE isahl_auth.auth_users \
         SET display_name = COALESCE($2, display_name), status = $3, is_active = $4, updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(&body.display_name)
    .bind(status_str)
    .bind(active)
    .execute(pool.get_ref())
    .await;

    get_user_inner(pool.get_ref(), id).await
}

pub async fn patch_user(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<String>,
    body: web::Json<ScimPatchRequest>,
) -> HttpResponse {
    if let Err(resp) = require_scim_token(&req) {
        return resp;
    }
    let id: i64 = match path.into_inner().parse() {
        Ok(i) => i,
        Err(_) => return error_response(actix_web::http::StatusCode::BAD_REQUEST, "Invalid id"),
    };

    let exists: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM isahl_auth.auth_users WHERE id = $1")
            .bind(id)
            .fetch_optional(pool.get_ref())
            .await
            .unwrap_or(None);
    if exists.is_none() {
        return error_response(actix_web::http::StatusCode::NOT_FOUND, "User not found");
    }

    for op in &body.Operations {
        let value_str = op
            .value
            .as_ref()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .or_else(|| {
                op.value
                    .as_ref()
                    .and_then(|v| v.as_bool().map(|b| b.to_string()))
            });
        let path = op.path.as_deref().unwrap_or("");

        let (sql, bind_val): (&str, String) = match path {
            "active" => (
                "UPDATE isahl_auth.auth_users SET is_active = $2, status = CASE WHEN $2 THEN 'active' ELSE 'disabled' END, updated_at = NOW() WHERE id = $1",
                value_str.clone().unwrap_or_else(|| "true".into()),
            ),
            "displayName" => (
                "UPDATE isahl_auth.auth_users SET display_name = $2, updated_at = NOW() WHERE id = $1",
                value_str.clone().unwrap_or_default(),
            ),
            "name.givenName" | "name.formatted" => (
                "UPDATE isahl_auth.auth_users SET display_name = $2, updated_at = NOW() WHERE id = $1",
                value_str.clone().unwrap_or_default(),
            ),
            _ => {
                log::warn!("scim patch_user: unsupported path '{}', skipping", path);
                continue;
            }
        };
        let is_bool = path == "active";
        let _ = if is_bool {
            let b = value_str.as_deref() == Some("true") || value_str.as_deref() == Some("True");
            sqlx::query(sql)
                .bind(id)
                .bind(b)
                .execute(pool.get_ref())
                .await
        } else {
            sqlx::query(sql)
                .bind(id)
                .bind(&bind_val)
                .execute(pool.get_ref())
                .await
        };
    }

    get_user_inner(pool.get_ref(), id).await
}

pub async fn delete_user(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<String>,
) -> HttpResponse {
    if let Err(resp) = require_scim_token(&req) {
        return resp;
    }
    let id: i64 = match path.into_inner().parse() {
        Ok(i) => i,
        Err(_) => return error_response(actix_web::http::StatusCode::BAD_REQUEST, "Invalid id"),
    };
    let result = sqlx::query(
        "UPDATE isahl_auth.auth_users SET is_active = false, status = 'disabled', updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(id)
    .execute(pool.get_ref())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => HttpResponse::NoContent().finish(),
        Ok(_) => error_response(actix_web::http::StatusCode::NOT_FOUND, "User not found"),
        Err(e) => {
            log::error!("scim delete_user error: {}", e);
            error_response(
                actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Database error",
            )
        }
    }
}

async fn get_user_inner(pool: &PgPool, id: i64) -> HttpResponse {
    let row: Option<UserRow> = sqlx::query_as(
        "SELECT id, name, username, email, display_name, status, is_active, created_at, updated_at, \
                settings->>'scim_external_id' AS external_id \
         FROM isahl_auth.auth_users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);
    match row {
        Some(r) => HttpResponse::Ok().json(to_scim_user(&r)),
        None => error_response(actix_web::http::StatusCode::NOT_FOUND, "User not found"),
    }
}
