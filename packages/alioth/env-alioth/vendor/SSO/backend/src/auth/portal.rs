//! 门户上下文 API + OpenAPI 密钥自助门户
//!
//! 1. 门户上下文：NGAC 属性驱动的双 Portal 决策（GATEWAY_DESIGN_SPEC.md §3.3），
//!    经 `login::configure` 的 `/portal-context` 路由对外（本模块不再重复注册）。
//! 2. 密钥自助门户（OPENAPI_SPEC §7 P3 缺口）：JWT 自然人自助
//!    列表 / 创建 / 轮换 / 吊销**本人**的 `api_clients`，与管理面（admin/api_clients.rs）
//!    共用 `api_clients_common` 的密钥生成 / 事务 / 订阅逻辑。
//!
//! 归属模型（见 openspec/changes/openapi-self-service-portal/ design.md D2）：
//! 每个自助 client 同步创建独立服务用户（NGAC 主体，§2.2），归属信息以
//! `client_name = "user:<uid>:" + 用户名称` 前缀记录（DDL 无 owner 列且本 mission
//! 禁改 migrations）；列表/鉴权均按此前缀过滤，响应中剥离前缀返回显示名。

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};

use super::api_clients_common;
use super::extract_user_id;
use super::AuthState;

#[derive(Debug, Serialize)]
pub struct PortalContext {
    /// 用户的 portal-scope 列表
    pub portal_scope: Vec<String>,
    /// 策略引擎计算的默认着陆门户
    pub portal_default: String,
    /// 登录后着陆路径
    pub landing_path: String,
    /// 布局模式
    pub layout_mode: String,
}

/// 从 NGAC 属性名推导 portal-scope
fn compute_portal_context(attrs: &[String]) -> PortalContext {
    let has_workbench = attrs.iter().any(|a| a == "admin" || a == "operator");
    let has_storefront = attrs
        .iter()
        .any(|a| a == "user" || a == "customer" || a == "storefront");

    let mut portal_scope: Vec<String> = Vec::new();
    if has_workbench {
        portal_scope.push("workbench".to_string());
    }
    if has_storefront {
        portal_scope.push("storefront".to_string());
    }
    if portal_scope.is_empty() {
        // 用户无 NGAC 属性时默认 workbench（开发/回退模式）
        portal_scope.push("workbench".to_string());
    }

    let (portal_default, landing_path, layout_mode) = if has_storefront && !has_workbench {
        (
            "storefront".to_string(),
            "/modules/shop/store/products".to_string(),
            "consumer".to_string(),
        )
    } else {
        (
            "workbench".to_string(),
            "/".to_string(),
            "operator".to_string(),
        )
    };

    PortalContext {
        portal_scope,
        portal_default,
        landing_path,
        layout_mode,
    }
}

/// GET /auth/portal-context
///
/// 返回策略计算后的门户上下文。
/// 前端据此决定 landing page、布局模式、路由过滤。
/// 注意：本 handler 经 `login::configure` 的 `/portal-context` 路由对外；
/// `configure_routes`（本模块）只注册密钥自助门户路由。
pub async fn get_portal_context(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    state: web::Data<super::AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    // 查询用户的 NGAC 属性
    let attrs: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT DISTINCT ua.o_name
        FROM isahl_auth.ngac_user_rr_attribute rel
        JOIN isahl_auth.ngac_user_attribute ua ON ua.id = rel.fk_user_attribute
        WHERE rel.fk_user = $1
          AND (rel.deleted_at IS NULL)
          AND (rel.expires_at IS NULL OR rel.expires_at > NOW())
        "#,
    )
    .bind(user_id)
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_default();

    let ctx = compute_portal_context(&attrs);

    HttpResponse::Ok().json(ctx)
}

// ═══════════════════════════════════════════════════════════════════════════════
// 密钥自助门户（OpenAPI 自助面，P3 缺口）
// ═══════════════════════════════════════════════════════════════════════════════

/// 归属前缀：`user:<uid>:`——自助创建的 client 用 client_name 前缀标记归属。
/// uid 为数字，不含 LIKE 通配符，无注入风险。
fn ownership_prefix(user_id: i64) -> String {
    format!("user:{}:", user_id)
}

#[derive(Debug, Deserialize)]
pub struct CreateSelfApiClientRequest {
    /// apikey | oauth2，缺省 apikey
    pub client_type: Option<String>,
    pub client_name: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SelfApiClientResponse {
    pub id: i64,
    pub client_id: String,
    pub client_type: String,
    /// 剥离归属前缀后的显示名（DB 中为 `user:<uid>:<name>` 全量，审计可溯源）
    pub client_name: String,
    pub scopes: Vec<String>,
    pub fk_service_user: i64,
    pub enabled: bool,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct SelfApiClientCreatedResponse {
    pub id: i64,
    pub client_id: String,
    pub client_type: String,
    pub client_name: String,
    /// 仅在创建时返回一次（oauth2=client_secret / apikey=api_key 明文）
    pub secret: String,
    pub fk_service_user: i64,
    pub enabled: bool,
}

/// 校验 client 归属：存在（未软删）→ Ok；不存在 → 404；非本人 → 403。
/// 越权边界：只能操作 `client_name` 归属前缀为 `user:<uid>:` 的 client。
async fn own_client_check(
    pool: &sqlx::PgPool,
    user_id: i64,
    client_row_id: i64,
) -> Result<(), HttpResponse> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT client_name FROM isahl_auth.api_clients \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(client_row_id)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);
    let Some((client_name,)) = row else {
        return Err(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Client not found",
        })));
    };
    if !client_name.starts_with(&ownership_prefix(user_id)) {
        return Err(HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Client does not belong to current user",
        })));
    }
    Ok(())
}

/// GET /auth/self/api-clients
///
/// 当前登录用户自己的 api_client 列表（按归属前缀过滤，绝不返回他用户）。
pub async fn list_self_api_clients(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let prefix = ownership_prefix(user_id);

    let clients: Vec<SelfApiClientResponse> = sqlx::query_as(
        r#"
        SELECT id, client_id, client_type, client_name, scopes, fk_service_user,
               enabled, expires_at, last_used_at, created_at
        FROM isahl_auth.api_clients
        WHERE deleted_at IS NULL AND client_name LIKE $1
        ORDER BY id
        "#,
    )
    .bind(format!("{}%", prefix))
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_else(|e| {
        // 诚实失败：与 admin 面一致，查询失败记录日志并返回空列表
        log::error!("list_self_api_clients query failed: {}", e);
        Vec::new()
    });

    let clients = clients
        .into_iter()
        .map(|mut c| {
            c.client_name = c
                .client_name
                .strip_prefix(&prefix)
                .unwrap_or(&c.client_name)
                .to_string();
            c
        })
        .collect::<Vec<_>>();

    HttpResponse::Ok().json(serde_json::json!({ "clients": clients }))
}

/// POST /auth/self/api-clients
///
/// 创建本人 OpenAPI 调用方。client_type 缺省 apikey；同步创建服务用户
/// （NGAC 主体）+ 默认 free 订阅（同一事务）；归属前缀强制写入 client_name；
/// 明文 secret 仅此一次返回。
pub async fn create_self_api_client(
    req: HttpRequest,
    body: web::Json<CreateSelfApiClientRequest>,
    pool: web::Data<sqlx::PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let client_type = body
        .client_type
        .clone()
        .unwrap_or_else(|| "apikey".to_string())
        .to_ascii_lowercase();
    if client_type != "apikey" && client_type != "oauth2" {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "client_type must be 'apikey' or 'oauth2'"
        }));
    }

    // 自助面 client_id 一律自动生成（不暴露自定义 client_id，见 design.md D4）
    let (client_id, secret) = match client_type.as_str() {
        "apikey" => {
            let key = api_clients_common::generate_api_key();
            (key.clone(), key)
        }
        _ => {
            let sid = format!(
                "client-{}",
                uuid::Uuid::new_v4()
                    .to_string()
                    .chars()
                    .take(12)
                    .collect::<String>()
            );
            (sid, api_clients_common::generate_client_secret())
        }
    };

    let prefix = ownership_prefix(user_id);
    let client_name = format!("{}{}", prefix, body.client_name.clone().unwrap_or_default());

    let created = match api_clients_common::create_api_client(
        pool.get_ref(),
        client_id.clone(),
        &client_type,
        &client_name,
        secret.clone(),
        body.scopes.clone().unwrap_or_default(),
        body.expires_at,
    )
    .await
    {
        Ok(c) => c,
        Err(api_clients_common::CreateClientError::ClientIdTaken) => {
            return HttpResponse::Conflict().json(serde_json::json!({
                "error": "Client ID already exists",
            }));
        }
        Err(e) => {
            log::error!("create_self_api_client error: {:?}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to create client",
            }));
        }
    };

    HttpResponse::Created().json(SelfApiClientCreatedResponse {
        id: created.id,
        client_id,
        client_type,
        client_name: body.client_name.clone().unwrap_or_default(),
        secret,
        fk_service_user: created.fk_service_user,
        enabled: true,
    })
}

/// POST /auth/self/api-clients/{id}/rotate-secret
///
/// 轮换本人 client 的密钥（apikey 同步替换 client_id，oauth2 仅覆盖 secret_hash）。
/// 明文仅在此响应中返回一次；旧 secret 立即失效。
pub async fn rotate_self_api_client_secret(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<sqlx::PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let client_row_id = path.into_inner();

    if let Err(resp) = own_client_check(pool.get_ref(), user_id, client_row_id).await {
        return resp;
    }

    match api_clients_common::rotate_client_secret(pool.get_ref(), client_row_id).await {
        Ok(Some(rotated)) => HttpResponse::Ok().json(serde_json::json!({
            "data": {
                "client_id": rotated.client_id,
                "secret": rotated.secret,
            },
        })),
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Client not found",
        })),
        Err(api_clients_common::ClientOpError::Hash(e)) => {
            log::error!("rotate_self_api_client_secret hash error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to hash secret",
            }))
        }
        Err(api_clients_common::ClientOpError::Db(e)) => {
            log::error!("rotate_self_api_client_secret error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Rotate failed",
            }))
        }
    }
}

/// DELETE /auth/self/api-clients/{id}
///
/// 吊销（软删除）本人 client，并挂起其 active 订阅。
/// 服务用户保留（历史审计可解析），不再可签发令牌。
pub async fn delete_self_api_client(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<sqlx::PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let client_row_id = path.into_inner();

    if let Err(resp) = own_client_check(pool.get_ref(), user_id, client_row_id).await {
        return resp;
    }

    match api_clients_common::soft_delete_client(pool.get_ref(), client_row_id).await {
        Ok(n) if n > 0 => HttpResponse::Ok().json(serde_json::json!({
            "deleted": true,
            "id": client_row_id,
        })),
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Client not found",
        })),
        Err(e) => {
            log::error!("delete_self_api_client error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Delete failed",
            }))
        }
    }
}

/// 密钥自助门户路由（相对路径注册，lib.rs 在 `/auth` 与 `/api/auth` scope 双挂载，
/// 见 design.md D1）。portal-context 仍由 `login::configure` 的 `/portal-context`
/// 路由对外，不在此重复注册。
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/self/api-clients", web::get().to(list_self_api_clients))
        .route("/self/api-clients", web::post().to(create_self_api_client))
        .route(
            "/self/api-clients/{id}/rotate-secret",
            web::post().to(rotate_self_api_client_secret),
        )
        .route(
            "/self/api-clients/{id}",
            web::delete().to(delete_self_api_client),
        );
}
