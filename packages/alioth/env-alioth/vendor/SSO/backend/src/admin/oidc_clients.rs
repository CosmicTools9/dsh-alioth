//! OIDC 客户端注册管理 API
//!
//! Admin 级别的 OIDC RP 多租户管理：
//! - 列出所有已注册 OIDC 客户端
//! - 注册新客户端（生成 client_secret）
//! - 更新客户端配置
//! - 删除（软删除）客户端

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};

use crate::auth::client_secret::hash_client_secret_async;
use crate::auth::jwt::validate_access_token;
use crate::auth::AuthState;

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn require_admin(
    req: &HttpRequest,
    pool: &PgPool,
    auth_state: &AuthState,
) -> Result<i64, HttpResponse> {
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
        HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Database error: {}", e)
        }))
    })?;

    if !is_admin {
        return Err(HttpResponse::Forbidden()
            .json(serde_json::json!({"error": "Admin privilege required"})));
    }

    Ok(user_id)
}

fn generate_client_secret() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ── Models ────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct OidcClientResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub scopes: Vec<String>,
    pub enabled: bool,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

#[derive(Debug, Serialize)]
pub struct OidcClientCreatedResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    /// 仅在创建时返回一次
    pub client_secret: String,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateOidcClientRequest {
    pub client_id: String,
    pub client_name: Option<String>,
    pub redirect_uris: Vec<String>,
    pub scopes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateOidcClientRequest {
    pub client_name: Option<String>,
    pub redirect_uris: Option<Vec<String>>,
    pub scopes: Option<Vec<String>>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct PaginationMeta {
    #[serde(with = "common::serde_zuid")]
    pub limit: i64,
    #[serde(with = "common::serde_zuid")]
    pub offset: i64,
    #[serde(with = "common::serde_zuid")]
    pub total: i64,
}

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub limit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub offset: Option<i64>,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/admin/oidc/clients
pub async fn list_oidc_clients(
    req: HttpRequest,
    query: web::Query<PaginationParams>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let offset = query.offset.unwrap_or(0).max(0);

    let clients: Vec<OidcClientResponse> = sqlx::query_as(
        r#"
        SELECT id, client_id, client_name, redirect_uris, scopes, enabled, created_at, updated_at
        FROM isahl_auth.oidc_clients
        WHERE deleted_at IS NULL
        ORDER BY id
        LIMIT $1 OFFSET $2
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_default();

    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM isahl_auth.oidc_clients WHERE deleted_at IS NULL")
            .fetch_one(pool.get_ref())
            .await
            .unwrap_or(0);

    HttpResponse::Ok().json(serde_json::json!({
        "clients": clients,
        "pagination": PaginationMeta { limit, offset, total },
    }))
}

/// POST /api/admin/oidc/clients
///
/// 注册新 OIDC 客户端。返回生成的 client_secret（仅此一次）。
pub async fn create_oidc_client(
    req: HttpRequest,
    body: web::Json<CreateOidcClientRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let secret = generate_client_secret();
    let secret_hash = match hash_client_secret_async(secret.clone()).await {
        Ok(h) => h,
        Err(e) => {
            log::error!("create_oidc_client hash error: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to hash client_secret",
            }));
        }
    };

    let result = sqlx::query_as::<_, (i64,)>(
        r#"
        INSERT INTO isahl_auth.oidc_clients (client_id, client_name, client_secret_hash, redirect_uris, scopes, enabled)
        VALUES ($1, $2, $3, $4::TEXT[], $5::TEXT[], TRUE)
        RETURNING id
        "#,
    )
    .bind(&body.client_id)
    .bind(body.client_name.as_deref().unwrap_or(""))
    .bind(&secret_hash)
    .bind(&body.redirect_uris)
    .bind(body.scopes.clone().unwrap_or_default())
    .fetch_one(pool.get_ref())
    .await;

    match result {
        Ok((id,)) => HttpResponse::Created().json(OidcClientCreatedResponse {
            id,
            client_id: body.client_id.clone(),
            client_name: body.client_name.clone().unwrap_or_default(),
            redirect_uris: body.redirect_uris.clone(),
            client_secret: secret,
            enabled: true,
        }),
        Err(e) => {
            log::error!("create_oidc_client error: {}", e);
            HttpResponse::Conflict().json(serde_json::json!({
                "error": "Client ID may already exist",
                "detail": e.to_string(),
            }))
        }
    }
}

/// PUT /api/admin/oidc/clients/{id}
pub async fn update_oidc_client(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateOidcClientRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let client_id = path.into_inner();

    // Build dynamic update
    let mut sets: Vec<String> = Vec::new();
    let mut idx = 1;

    if body.client_name.is_some() {
        sets.push(format!("client_name = ${}", idx));
        idx += 1;
    }
    if body.redirect_uris.is_some() {
        sets.push(format!("redirect_uris = ${}::TEXT[]", idx));
        idx += 1;
    }
    if body.scopes.is_some() {
        sets.push(format!("scopes = ${}::TEXT[]", idx));
        idx += 1;
    }
    if body.enabled.is_some() {
        sets.push(format!("enabled = ${}", idx));
        idx += 1;
    }

    if sets.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "No fields to update"
        }));
    }

    sets.push("updated_at = NOW()".to_string());

    let sql = format!(
        "UPDATE isahl_auth.oidc_clients SET {} WHERE id = ${} AND deleted_at IS NULL",
        sets.join(", "),
        idx
    );

    let mut query = sqlx::query(AssertSqlSafe(sql.as_str()));

    if let Some(ref name) = body.client_name {
        query = query.bind(name);
    }
    if let Some(ref uris) = body.redirect_uris {
        query = query.bind(uris);
    }
    if let Some(ref scopes) = body.scopes {
        query = query.bind(scopes);
    }
    if let Some(enabled) = body.enabled {
        query = query.bind(enabled);
    }
    query = query.bind(client_id);

    match query.execute(pool.get_ref()).await {
        Ok(r) if r.rows_affected() > 0 => {
            HttpResponse::Ok().json(serde_json::json!({"status": "updated"}))
        }
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "Client not found"})),
        Err(e) => {
            log::error!("update_oidc_client error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({"error": "Update failed"}))
        }
    }
}

/// DELETE /api/admin/oidc/clients/{id}
pub async fn delete_oidc_client(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let client_id = path.into_inner();

    match sqlx::query(
        "UPDATE isahl_auth.oidc_clients SET deleted_at = NOW(), updated_at = NOW() WHERE id = $1 AND deleted_at IS NULL"
    )
    .bind(client_id)
    .execute(pool.get_ref())
    .await
    {
        Ok(r) if r.rows_affected() > 0 => {
            HttpResponse::Ok().json(serde_json::json!({"status": "deleted"}))
        }
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "Client not found"})),
        Err(e) => {
            log::error!("delete_oidc_client error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({"error": "Delete failed"}))
        }
    }
}
