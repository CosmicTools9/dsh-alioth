//! Admin 身份提供方（identity provider）管理 handlers

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool, Row};

/// Provider response
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ProviderResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub provider_type: String,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub userinfo_endpoint: Option<String>,
    pub enabled: bool,
    pub config: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Create provider request
#[derive(Debug, Deserialize)]
pub struct CreateProviderRequest {
    pub name: String,
    pub provider_type: String,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub userinfo_endpoint: Option<String>,
    pub client_id: String,
    pub client_secret: String,
    pub scopes: Option<Vec<String>>,
    pub enabled: Option<bool>,
    /// OIDC/LDAP 等类型的专属配置（jsonb）：authorization_endpoint / token_endpoint /
    /// client_id / server_url / bind_dn / base_dn 等
    pub config: Option<serde_json::Value>,
}

/// Update provider request
#[derive(Debug, Deserialize)]
pub struct UpdateProviderRequest {
    pub name: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub enabled: Option<bool>,
    /// OIDC/LDAP 等类型的专属配置（jsonb）
    pub config: Option<serde_json::Value>,
}

use super::require_admin;
use crate::auth::AuthState;

// ============================================================================
// Providers
// ============================================================================

/// GET /api/admin/providers
/// List identity providers.
pub async fn list_providers(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let providers = sqlx::query_as::<_, ProviderResponse>(
        r#"
        SELECT id, name, provider_type, authorization_endpoint, token_endpoint,
               userinfo_endpoint, enabled, config, created_at
        FROM isahl_auth.identity_providers
        ORDER BY name
        "#,
    )
    .fetch_all(pool.get_ref())
    .await;

    match providers {
        Ok(rows) => HttpResponse::Ok().json(serde_json::json!({"providers": rows})),
        Err(e) => {
            log::error!("list_providers DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to list providers"}))
        }
    }
}

/// POST /api/admin/providers
/// Create a new identity provider.
pub async fn create_provider(
    req: HttpRequest,
    body: web::Json<CreateProviderRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let enabled = body.enabled.unwrap_or(true);

    let result = sqlx::query(
        r#"
        INSERT INTO isahl_auth.identity_providers (name, provider_type, authorization_endpoint, token_endpoint,
            userinfo_endpoint, client_id, client_secret_encrypted, scopes, enabled, config, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text[], $9, $10, NOW())
        RETURNING id
        "#,
    )
    .bind(&body.name)
    .bind(&body.provider_type)
    .bind(&body.authorization_endpoint)
    .bind(&body.token_endpoint)
    .bind(&body.userinfo_endpoint)
    .bind(&body.client_id)
    .bind(&body.client_secret)
    .bind(&body.scopes)
    .bind(enabled)
    .bind(&body.config)
    .fetch_one(pool.get_ref())
    .await;

    match result {
        Ok(row) => {
            let id: i64 = row.get("id");
            HttpResponse::Created().json(serde_json::json!({
                "id": id,
                "name": body.name,
                "provider_type": body.provider_type,
                "enabled": enabled,
            }))
        }
        Err(e) => {
            log::error!("create_provider DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to create provider"}))
        }
    }
}

/// PUT /api/admin/providers/{id}
/// Update an existing identity provider.
pub async fn update_provider(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateProviderRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let provider_id = path.into_inner();

    let mut sets: Vec<&str> = Vec::new();
    if body.name.is_some() {
        sets.push("name = $2");
    }
    if body.client_id.is_some() {
        sets.push("client_id = $3");
    }
    if body.client_secret.is_some() {
        sets.push("client_secret_encrypted = $4");
    }
    if body.scopes.is_some() {
        sets.push("scopes = $5");
    }
    if body.enabled.is_some() {
        sets.push("enabled = $6");
    }
    if body.config.is_some() {
        sets.push("config = $7");
    }
    if sets.is_empty() {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"error": "No fields to update"}));
    }
    let sql = format!(
        "UPDATE isahl_auth.identity_providers SET {} WHERE id = $1",
        sets.join(", ")
    );

    let result = sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(provider_id)
        .bind(&body.name)
        .bind(&body.client_id)
        .bind(&body.client_secret)
        .bind(&body.scopes)
        .bind(body.enabled)
        .bind(&body.config)
        .execute(pool.get_ref())
        .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            HttpResponse::Ok().json(serde_json::json!({"status": "updated", "id": provider_id}))
        }
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "Provider not found"})),
        Err(e) => {
            log::error!("update_provider DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to update provider"}))
        }
    }
}

/// GET /api/admin/providers/{id}
/// Fetch a single identity provider by id.
pub async fn get_provider(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let provider_id = path.into_inner();
    let row = sqlx::query_as::<_, ProviderResponse>(
        r#"
        SELECT id, name, provider_type, authorization_endpoint, token_endpoint,
               userinfo_endpoint, enabled, config, created_at
        FROM isahl_auth.identity_providers
        WHERE id = $1
        "#,
    )
    .bind(provider_id)
    .fetch_optional(pool.get_ref())
    .await;

    match row {
        Ok(Some(p)) => HttpResponse::Ok().json(p),
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({"error": "Provider not found"}))
        }
        Err(e) => {
            log::error!("get_provider DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to get provider"}))
        }
    }
}

/// DELETE /api/admin/providers/{id}
/// Delete an identity provider.
pub async fn delete_provider(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let provider_id = path.into_inner();
    let result = sqlx::query("DELETE FROM isahl_auth.identity_providers WHERE id = $1")
        .bind(provider_id)
        .execute(pool.get_ref())
        .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => HttpResponse::NoContent().finish(),
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "Provider not found"})),
        Err(e) => {
            log::error!("delete_provider DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to delete provider"}))
        }
    }
}

/// POST /api/admin/providers/{id}/toggle
/// Enable/disable an identity provider (flips the current `enabled` flag).
pub async fn toggle_provider(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let provider_id = path.into_inner();
    let result = sqlx::query_scalar::<_, bool>(
        "UPDATE isahl_auth.identity_providers SET enabled = NOT enabled WHERE id = $1 \
         RETURNING enabled",
    )
    .bind(provider_id)
    .fetch_optional(pool.get_ref())
    .await;

    match result {
        Ok(Some(enabled)) => {
            HttpResponse::Ok().json(serde_json::json!({"id": provider_id, "enabled": enabled}))
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({"error": "Provider not found"}))
        }
        Err(e) => {
            log::error!("toggle_provider DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to toggle provider"}))
        }
    }
}

/// POST /api/admin/providers/{id}/test
/// Validate a provider's configuration without performing a live login.
/// Returns 200 with a `valid` flag and per-field issues; never errors on a
/// misconfigured provider (that is the point of the test).
pub async fn test_provider(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let provider_id = path.into_inner();
    let row: Option<(String, Option<serde_json::Value>)> = sqlx::query_as(
        "SELECT provider_type, config FROM isahl_auth.identity_providers WHERE id = $1",
    )
    .bind(provider_id)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);

    let (provider_type, config) = match row {
        Some(r) => r,
        None => {
            return HttpResponse::NotFound()
                .json(serde_json::json!({"error": "Provider not found"}))
        }
    };

    let mut issues: Vec<String> = Vec::new();
    let cfg = config.unwrap_or(serde_json::Value::Null);

    match provider_type.as_str() {
        "oidc" | "oauth" => {
            for field in ["authorization_endpoint", "token_endpoint", "client_id"] {
                let v = cfg.get(field).and_then(|v| v.as_str()).unwrap_or("");
                if v.trim().is_empty() {
                    issues.push(format!("missing config.{}", field));
                }
            }
        }
        "ldap" => {
            let v = cfg.get("server_url").and_then(|v| v.as_str()).unwrap_or("");
            if v.trim().is_empty() {
                issues.push("missing config.server_url".to_string());
            }
        }
        other => issues.push(format!("unknown provider_type: {}", other)),
    }

    let valid = issues.is_empty();
    HttpResponse::Ok().json(serde_json::json!({
        "id": provider_id,
        "provider_type": provider_type,
        "valid": valid,
        "issues": issues,
    }))
}
