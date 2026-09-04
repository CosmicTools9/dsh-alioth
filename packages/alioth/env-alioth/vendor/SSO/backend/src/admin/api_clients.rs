//! OpenAPI 统一调用方注册表管理 API（Admin 级别）
//!
//! 管理 `isahl_auth.api_clients`：
//! - 创建 client（oauth2 或 apikey），同步创建服务用户 + 默认 free 订阅
//! - 列出 client（含订阅/服务用户信息）
//! - 更新 client 配置
//! - 吊销（软删除）client
//!
//! 对齐 `openspec/changes/add-openapi-external-access/`：
//! 每个 client 关联一个服务用户（`isahl_auth.auth_users`，user_type='service'），
//! 服务令牌经 Gateway PEP 解析为 `svc_user_id` 走 NGAC PDP 决策。

use actix_web::{web, HttpRequest, HttpResponse};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use crate::auth::api_clients_common::{
    create_api_client as shared_create_api_client, generate_api_key, generate_client_secret,
    rotate_client_secret, soft_delete_client, ClientOpError, CreateClientError, RotatedSecret,
};
// 仅 admin 单测引用（规则一致性断言）
#[cfg(test)]
use crate::auth::api_clients_common::generate_secret_for;
use crate::auth::AuthState;

// ── Models ────────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct ApiClientResponse {
    pub id: i64,
    pub client_id: String,
    pub client_type: String,
    pub client_name: String,
    pub scopes: Vec<String>,
    pub fk_service_user: i64,
    pub enabled: bool,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    // 列为 timestamptz——NaiveDateTime 解码失败会被 unwrap_or_default 静默吞成空列表
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, serde::Serialize)]
pub struct ApiClientCreatedResponse {
    pub id: i64,
    pub client_id: String,
    pub client_type: String,
    pub client_name: String,
    /// 仅在创建时返回一次（oauth2=client_secret / apikey=api_key 明文）
    pub secret: String,
    pub fk_service_user: i64,
    pub enabled: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateApiClientRequest {
    /// 唯一标识；apikey 类型可不填（自动生成 ak_ 前缀密钥）
    pub client_id: Option<String>,
    /// apikey | oauth2
    pub client_type: String,
    pub client_name: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateApiClientRequest {
    pub client_name: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub enabled: Option<bool>,
    pub expires_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
}

#[derive(Debug, serde::Serialize)]
pub struct PaginationMeta {
    pub limit: i64,
    pub offset: i64,
    pub total: i64,
}

#[derive(Debug, serde::Deserialize)]
pub struct PaginationParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/admin/api-clients
pub async fn list_api_clients(
    _req: HttpRequest,
    query: web::Query<PaginationParams>,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let offset = query.offset.unwrap_or(0).max(0);

    let clients: Vec<ApiClientResponse> = sqlx::query_as(
        r#"
        SELECT id, client_id, client_type, client_name, scopes, fk_service_user,
               enabled, expires_at, last_used_at, created_at
        FROM isahl_auth.api_clients
        WHERE deleted_at IS NULL
        ORDER BY id
        LIMIT $1 OFFSET $2
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_else(|e| {
        // 诚实失败：静默吞错曾让解码 bug 表现为 total>0 但 clients=[] 假象
        log::error!("list_api_clients query failed: {}", e);
        Vec::new()
    });

    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM isahl_auth.api_clients WHERE deleted_at IS NULL")
            .fetch_one(pool.get_ref())
            .await
            .unwrap_or(0);

    HttpResponse::Ok().json(serde_json::json!({
        "clients": clients,
        "pagination": PaginationMeta { limit, offset, total },
    }))
}

/// POST /api/admin/api-clients
///
/// 创建 OpenAPI 调用方。同步创建服务用户（NGAC 主体）+ 默认 free 订阅。
/// 返回明文 secret（仅此一次）。
pub async fn create_api_client(
    _req: HttpRequest,
    body: web::Json<CreateApiClientRequest>,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    let client_type = body.client_type.to_ascii_lowercase();
    if client_type != "apikey" && client_type != "oauth2" {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "client_type must be 'apikey' or 'oauth2'"
        }));
    }

    // 生成身份标识 + secret
    let (client_id, secret) = match client_type.as_str() {
        "apikey" => {
            let key = generate_api_key();
            (body.client_id.clone().unwrap_or_else(|| key.clone()), key)
        }
        _ => {
            let sid = body.client_id.clone().unwrap_or_else(|| {
                format!(
                    "client-{}",
                    Uuid::new_v4()
                        .to_string()
                        .chars()
                        .take(12)
                        .collect::<String>()
                )
            });
            (sid, generate_client_secret())
        }
    };

    let created = match shared_create_api_client(
        pool.get_ref(),
        client_id.clone(),
        &client_type,
        body.client_name.as_deref().unwrap_or(""),
        secret.clone(),
        body.scopes.clone().unwrap_or_default(),
        body.expires_at,
    )
    .await
    {
        Ok(c) => c,
        Err(CreateClientError::ClientIdTaken) => {
            return HttpResponse::Conflict().json(serde_json::json!({
                "error": "Client ID already exists"
            }));
        }
        Err(CreateClientError::Insert(e)) => {
            log::error!("create_api_client insert error: {}", e);
            return HttpResponse::Conflict().json(serde_json::json!({
                "error": "Client ID may already exist",
                "detail": e.to_string(),
            }));
        }
        Err(CreateClientError::Hash(e)) => {
            log::error!("create_api_client hash error: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to hash secret",
            }));
        }
        Err(CreateClientError::ServiceUser(e)) => {
            log::error!("create_api_client service user error: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to create service user",
            }));
        }
        Err(CreateClientError::Subscription(e)) => {
            log::error!("create_api_client default subscription error: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to create subscription",
            }));
        }
        Err(CreateClientError::Begin(e) | CreateClientError::Commit(e)) => {
            log::error!("create_api_client tx error: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Transaction failed",
            }));
        }
    };

    HttpResponse::Created().json(ApiClientCreatedResponse {
        id: created.id,
        client_id,
        client_type,
        client_name: body.client_name.clone().unwrap_or_default(),
        secret,
        fk_service_user: created.fk_service_user,
        enabled: true,
    })
}

/// POST /api/admin/api-clients/{id}/rotate-secret
///
/// 轮换调用方密钥（openapi-client-secret-rotation）：按 client_type 生成新明文
/// （规则与 create_api_client 一致，见 `generate_secret_for`），argon2id 哈希落库，
/// 明文仅在此响应中返回一次；旧 secret 立即失效（哈希已被覆盖）。
///
/// apikey 型 client_id 即密钥明文（auth 按 `left(client_id, 8)` 前缀索引），
/// 轮换时必须同步替换 client_id，否则新密钥无法通过前缀定位；oauth2 型
/// client_id 为稳定标识，仅覆盖 secret_hash。
pub async fn rotate_api_client_secret(
    _req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    let client_row_id = path.into_inner();

    match rotate_client_secret(pool.get_ref(), client_row_id).await {
        Ok(Some(RotatedSecret { client_id, secret })) => {
            HttpResponse::Ok().json(serde_json::json!({
                "data": {
                    "client_id": client_id,
                    "secret": secret,
                },
            }))
        }
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Client not found",
        })),
        Err(ClientOpError::Hash(e)) => {
            log::error!("rotate_api_client_secret hash error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to hash secret",
            }))
        }
        Err(ClientOpError::Db(e)) => {
            log::error!("rotate_api_client_secret error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Rotate failed",
            }))
        }
    }
}

/// PUT /api/admin/api-clients/{id}
pub async fn update_api_client(
    _req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateApiClientRequest>,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    let client_id = path.into_inner();

    let mut sets: Vec<String> = Vec::new();
    let mut idx = 1usize;
    let mut q = String::from("UPDATE isahl_auth.api_clients SET ");
    if body.client_name.is_some() {
        sets.push(format!("client_name = ${}", idx));
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
    if body.expires_at.is_some() {
        sets.push(format!("expires_at = ${}", idx));
        idx += 1;
    }
    if sets.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "No update fields provided"
        }));
    }
    q.push_str(&sets.join(", "));
    q.push_str(&format!(" WHERE id = ${} AND deleted_at IS NULL", idx));

    let mut query = sqlx::query(AssertSqlSafe(q.as_str()));
    if let Some(name) = &body.client_name {
        query = query.bind(name);
    }
    if let Some(scopes) = &body.scopes {
        query = query.bind(scopes);
    }
    if let Some(enabled) = body.enabled {
        query = query.bind(enabled);
    }
    if let Some(exp) = &body.expires_at {
        query = query.bind(exp);
    }
    query = query.bind(client_id);

    match query.execute(pool.get_ref()).await {
        Ok(r) if r.rows_affected() > 0 => HttpResponse::Ok().json(serde_json::json!({
            "updated": true,
            "id": client_id,
        })),
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Client not found",
        })),
        Err(e) => {
            log::error!("update_api_client error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Update failed",
            }))
        }
    }
}

/// DELETE /api/admin/api-clients/{id}
///
/// 吊销（软删除）client。服务用户保留（历史审计可解析），不再可签发令牌。
pub async fn delete_api_client(
    _req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    let client_id = path.into_inner();

    match soft_delete_client(pool.get_ref(), client_id).await {
        Ok(n) if n > 0 => {
            HttpResponse::Ok().json(serde_json::json!({ "deleted": true, "id": client_id }))
        }
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Client not found",
        })),
        Err(e) => {
            log::error!("delete_api_client error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Delete failed",
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_format() {
        let k = generate_api_key();
        assert!(k.starts_with("ak_"), "API Key 必须以 ak_ 开头");
        assert_eq!(k.len(), 3 + 43, "ak_ + 32 bytes base64url 无填充 = 43 字符");
    }

    #[test]
    fn client_secret_format() {
        let s = generate_client_secret();
        assert_eq!(s.len(), 36, "UUID v4 格式");
    }

    #[test]
    fn rotate_secret_rules_match_create() {
        // apikey → ak_ 前缀密钥（与 create 的 generate_api_key 同规则）
        let apikey = generate_secret_for("apikey");
        assert!(apikey.starts_with("ak_"), "apikey 明文必须以 ak_ 开头");
        assert_eq!(
            apikey.len(),
            3 + 43,
            "ak_ + 32 bytes base64url 无填充 = 43 字符"
        );
        // oauth2 → UUID v4（与 create 的 generate_client_secret 同规则）
        let oauth2 = generate_secret_for("oauth2");
        assert_eq!(oauth2.len(), 36, "UUID v4 格式");
        assert_eq!(
            oauth2.chars().filter(|c| *c == '-').count(),
            4,
            "UUID 含 4 个连字符"
        );
        // 两次轮换不产生相同明文
        assert_ne!(generate_secret_for("apikey"), apikey, "轮换必须生成新密钥");
        assert_ne!(generate_secret_for("oauth2"), oauth2, "轮换必须生成新密钥");
    }
}
