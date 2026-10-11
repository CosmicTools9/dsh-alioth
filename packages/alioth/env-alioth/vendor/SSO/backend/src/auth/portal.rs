//! 门户上下文 API + 外部主体 API-KEY 签发面
//!
//! 1. 门户上下文：NGAC 属性驱动的双 Portal 决策（GATEWAY_DESIGN_SPEC.md §3.3），
//!    经 `login::configure` 的 `/portal-context` 路由对外（本模块不再重复注册）。
//! 2. 外部主体签发面（`/auth/external/api-clients`，
//!    change `move-apikey-issuance-to-external-portal`）：**仅**经外部入驻审批的
//!    外部主体（`user_type='external'` 且 `status='active'`）可列表 / 创建 / 轮换 /
//!    吊销**本人**的 `api_clients`，与管理面（admin/api_clients.rs）共用
//!    `api_clients_common` 的密钥生成 / 事务 / 订阅逻辑。取得入口 = OpenActivity 门户；
//!    Gateway MUST NOT 提供「以自持身份申请 API-KEY」的自助入口。
//!
//! 归属模型（见 openspec/changes/openapi-self-service-portal/ design.md D2）：
//! 每个签发 client 同步创建独立服务用户（NGAC 主体，§2.2），归属信息以
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

/// 门户归属推导（GATEWAY_DESIGN_SPEC §3.3）——**唯一实现**。
///
/// 属性驱动：UA 名前缀 `portal-scope:` 的取值判定门户集合
/// （`portal-scope:storefront` / `portal-scope:workbench`；空值/未知值忽略）。
/// 不含合法前缀值 → 规约默认策略（§3.3.2 策略 3）：workbench。
/// 扁平业务角色名（customer/carrier/supplier/…）是**业务域**表达，MUST NOT
/// 参与门户判定——存量账号的 storefront 归属由种子显式补绑（D2/D6）。
/// 消费方：`/api/auth/portal-context`、`/api/auth/me`（attributes）、`/auth/check-access`。
pub fn derive_portal_scope(attrs: &[String]) -> Vec<String> {
    let has = |value: &str| {
        attrs
            .iter()
            .any(|a| a.strip_prefix("portal-scope:") == Some(value))
    };
    let mut scope: Vec<String> = Vec::new();
    if has("workbench") {
        scope.push("workbench".to_string());
    }
    if has("storefront") {
        scope.push("storefront".to_string());
    }
    if scope.is_empty() {
        scope.push("workbench".to_string());
    }
    scope
}

/// storefront-only 用户的落地路径：部署配置 `STOREFRONT_LANDING_PATH`，缺省 `/`。
/// MUST NOT 硬编码应用/模块路径（发布面按 ns 自配）。/auth/check-access 同用。
pub fn storefront_landing_path() -> String {
    std::env::var("STOREFRONT_LANDING_PATH")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/".to_string())
}

/// 从 NGAC 属性推导门户上下文（portal_default / landing_path / layout_mode）。
pub fn compute_portal_context(attrs: &[String]) -> PortalContext {
    let portal_scope = derive_portal_scope(attrs);
    let storefront_only = portal_scope.iter().any(|s| s == "storefront")
        && !portal_scope.iter().any(|s| s == "workbench");

    let (portal_default, landing_path, layout_mode) = if storefront_only {
        (
            "storefront".to_string(),
            storefront_landing_path(),
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

    // 查询用户的 NGAC 属性（判定链 PIP 单源：直派 ∪ 认知派生 ∪ 委托派生 ∪ 祖先闭包 + 时效）
    let attrs: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT DISTINCT ua.o_name
        FROM isahl_auth.effective_ua_ids($1) e
        JOIN isahl_auth.ngac_user_attribute ua ON ua.id = e.ua_id
        WHERE ua.deleted_at IS NULL
        "#,
    )
    .bind(user_id)
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_default();

    let ctx = compute_portal_context(&attrs);

    HttpResponse::Ok().json(ctx)
}

#[cfg(test)]
mod portal_scope_tests {
    use super::*;

    fn attrs(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn storefront_only_scope() {
        let scope = derive_portal_scope(&attrs(&["portal-scope:storefront"]));
        assert_eq!(scope, vec!["storefront".to_string()]);
    }

    #[test]
    fn workbench_only_scope() {
        let scope = derive_portal_scope(&attrs(&["portal-scope:workbench"]));
        assert_eq!(scope, vec!["workbench".to_string()]);
    }

    #[test]
    fn dual_scope() {
        let scope = derive_portal_scope(&attrs(&[
            "portal-scope:storefront",
            "portal-scope:workbench",
        ]));
        assert_eq!(
            scope,
            vec!["workbench".to_string(), "storefront".to_string()]
        );
    }

    #[test]
    fn no_portal_scope_attr_defaults_to_workbench() {
        // 规约默认策略（§3.3.2 策略 3）：无 portal-scope 属性 → workbench
        assert_eq!(
            derive_portal_scope(&attrs(&[])),
            vec!["workbench".to_string()]
        );
        assert_eq!(
            derive_portal_scope(&attrs(&["admin"])),
            vec!["workbench".to_string()]
        );
    }

    #[test]
    fn flat_business_role_names_do_not_map_to_portal_scope() {
        // 扁平业务角色名是业务域表达，不参与门户判定（customer/carrier/supplier/…）
        assert_eq!(
            derive_portal_scope(&attrs(&["customer"])),
            vec!["workbench".to_string()]
        );
        assert_eq!(
            derive_portal_scope(&attrs(&["carrier", "supplier"])),
            vec!["workbench".to_string()]
        );
    }

    #[test]
    fn unknown_or_empty_prefix_values_ignored() {
        assert_eq!(
            derive_portal_scope(&attrs(&["portal-scope:unknown", "portal-scope:"])),
            vec!["workbench".to_string()]
        );
    }

    #[test]
    fn storefront_only_context_shape() {
        // 删除进程级 env 干扰，确保缺省落地为 /
        std::env::remove_var("STOREFRONT_LANDING_PATH");
        let ctx = compute_portal_context(&attrs(&["portal-scope:storefront"]));
        assert_eq!(ctx.portal_default, "storefront");
        assert_eq!(ctx.layout_mode, "consumer");
        assert_eq!(ctx.landing_path, "/");
    }

    #[test]
    fn workbench_context_shape() {
        let ctx = compute_portal_context(&attrs(&["portal-scope:workbench"]));
        assert_eq!(ctx.portal_default, "workbench");
        assert_eq!(ctx.layout_mode, "operator");
        assert_eq!(ctx.landing_path, "/");
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 外部主体 API-KEY 签发面（取得入口 = OpenActivity 门户）
// ═══════════════════════════════════════════════════════════════════════════════

/// 归属前缀：`user:<uid>:`——自助创建的 client 用 client_name 前缀标记归属。
/// uid 为数字，不含 LIKE 通配符，无注入风险。
fn ownership_prefix(user_id: i64) -> String {
    format!("user:{}:", user_id)
}

/// 外部主体门禁（change `move-apikey-issuance-to-external-portal` D2）：
/// 仅 `user_type='external'` 且 `status='active'` 的账号可签发。
/// **白名单式判定**——`pending` / `pending_approval` / `identity_submitted` / `disabled` /
/// `rejected` 一律拒绝，新增状态 MUST NOT 静默放行。账号行缺失或查询失败 ⇒ fail-closed。
///
/// 响应：非 external → 403 `EXTERNAL_ONLY`；external 但未审批 → 403 `EXTERNAL_NOT_APPROVED`。
async fn require_external_active(pool: &sqlx::PgPool, user_id: i64) -> Result<(), HttpResponse> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT user_type::text, status::text FROM isahl_auth.auth_users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .unwrap_or_else(|e| {
        log::error!("require_external_active query failed: user={user_id} err={e}");
        None
    });

    match row {
        Some((user_type, status)) if user_type == "external" && status == "active" => Ok(()),
        Some((user_type, _)) if user_type == "external" => {
            Err(HttpResponse::Forbidden().json(serde_json::json!({
                "error": "EXTERNAL_NOT_APPROVED",
                "message": "external subject is not approved yet",
            })))
        }
        _ => Err(HttpResponse::Forbidden().json(serde_json::json!({
            "error": "EXTERNAL_ONLY",
            "message": "API key issuance is available to external subjects only",
        }))),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateExternalApiClientRequest {
    /// apikey | oauth2，缺省 apikey
    pub client_type: Option<String>,
    pub client_name: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ExternalApiClientResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub client_id: String,
    pub client_type: String,
    /// 剥离归属前缀后的显示名（DB 中为 `user:<uid>:<name>` 全量，审计可溯源）
    pub client_name: String,
    pub scopes: Vec<String>,
    #[serde(with = "common::serde_zuid")]
    pub fk_service_user: i64,
    pub enabled: bool,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// 派生授权状态（非 DB 列）：scopes 空 ⇒ `pending`（待管理员授予），非空 ⇒ `granted`
    #[sqlx(skip)]
    pub authorization_status: String,
}

#[derive(Debug, Serialize)]
pub struct ExternalApiClientCreatedResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub client_id: String,
    pub client_type: String,
    pub client_name: String,
    /// 仅在创建时返回一次（oauth2=client_secret / apikey=api_key 明文）
    pub secret: String,
    #[serde(with = "common::serde_zuid")]
    pub fk_service_user: i64,
    pub enabled: bool,
    /// 派生授权状态：见 ExternalApiClientResponse.authorization_status
    pub authorization_status: String,
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

/// GET /auth/external/api-clients
///
/// 当前外部主体自己的 api_client 列表（按归属前缀过滤，绝不返回他用户）。
pub async fn list_external_api_clients(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Err(resp) = require_external_active(pool.get_ref(), user_id).await {
        return resp;
    }
    let prefix = ownership_prefix(user_id);

    let clients: Vec<ExternalApiClientResponse> = sqlx::query_as(
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
        log::error!("list_external_api_clients query failed: {}", e);
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
            c.authorization_status = if c.scopes.is_empty() {
                "pending".to_string()
            } else {
                "granted".to_string()
            };
            c
        })
        .collect::<Vec<_>>();

    HttpResponse::Ok().json(serde_json::json!({ "clients": clients }))
}

/// POST /auth/external/api-clients
///
/// 创建本人 OpenAPI 调用方（仅外部主体）。client_type 缺省 apikey；同步创建服务用户
/// （NGAC 主体）+ 默认 free 订阅（同一事务）；归属前缀强制写入 client_name；
/// 明文 secret 仅此一次返回。
pub async fn create_external_api_client(
    req: HttpRequest,
    body: web::Json<CreateExternalApiClientRequest>,
    pool: web::Data<sqlx::PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Err(resp) = require_external_active(pool.get_ref(), user_id).await {
        return resp;
    }

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

    // 门户自助面 MUST NOT 自授通配（`*` / `*:{entity}`）——通配仅管理面经
    // NGAC `openapi_admin:0` 可授（scope-grammar-single-source：纵深防御封口）。
    if let Some(scopes) = body.scopes.as_ref() {
        let wildcards: Vec<String> = scopes
            .iter()
            .filter(|s| common::openapi_scope::is_wildcard_scope(s))
            .cloned()
            .collect();
        if !wildcards.is_empty() {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": "INVALID_SCOPE",
                "message": "通配 scope 不开放自助授予，请联系管理员经管理面授予",
                "details": wildcards,
            }));
        }
    }

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
        Err(api_clients_common::CreateClientError::InvalidScope(invalid)) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": "INVALID_SCOPE",
                "message": "scope 文法外值（合法形 {action}:{entity}，action ∈ read/create/update/delete；留空=待授权）",
                "details": invalid,
            }));
        }
        Err(e) => {
            log::error!("create_external_api_client error: {:?}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to create client",
            }));
        }
    };

    let requested_scopes = body.scopes.clone().unwrap_or_default();
    HttpResponse::Created().json(ExternalApiClientCreatedResponse {
        id: created.id,
        client_id,
        client_type,
        client_name: body.client_name.clone().unwrap_or_default(),
        secret,
        fk_service_user: created.fk_service_user,
        enabled: true,
        authorization_status: if requested_scopes.is_empty() {
            "pending".to_string()
        } else {
            "granted".to_string()
        },
    })
}

/// POST /auth/external/api-clients/{id}/rotate-secret
///
/// 轮换本人 client 的密钥（apikey 同步替换 client_id，oauth2 仅覆盖 secret_hash）。
/// 明文仅在此响应中返回一次；旧 secret 立即失效。
pub async fn rotate_external_api_client_secret(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<sqlx::PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Err(resp) = require_external_active(pool.get_ref(), user_id).await {
        return resp;
    }
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
            log::error!("rotate_external_api_client_secret hash error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to hash secret",
            }))
        }
        Err(api_clients_common::ClientOpError::Db(e)) => {
            log::error!("rotate_external_api_client_secret error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Rotate failed",
            }))
        }
    }
}

/// DELETE /auth/external/api-clients/{id}
///
/// 吊销（软删除）本人 client，并挂起其 active 订阅。
/// 服务用户保留（历史审计可解析），不再可签发令牌。
pub async fn delete_external_api_client(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<sqlx::PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Err(resp) = require_external_active(pool.get_ref(), user_id).await {
        return resp;
    }
    let client_row_id = path.into_inner();

    if let Err(resp) = own_client_check(pool.get_ref(), user_id, client_row_id).await {
        return resp;
    }

    match api_clients_common::soft_delete_client(pool.get_ref(), client_row_id).await {
        Ok(n) if n > 0 => HttpResponse::Ok().json(serde_json::json!({
            "deleted": true,
            "id": client_row_id.to_string(),
        })),
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Client not found",
        })),
        Err(e) => {
            log::error!("delete_external_api_client error: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Delete failed",
            }))
        }
    }
}

/// 外部主体签发面路由（相对路径注册，lib.rs 在 `/auth` 与 `/api/auth` scope 双挂载，
/// 见 design.md D1）。portal-context 仍由 `login::configure` 的 `/portal-context`
/// 路由对外，不在此重复注册。
///
/// 路径历史：原 `/self/api-clients`（任意已登录账号可用）→ `/external/api-clients`
/// （仅经审批的外部主体），见 change `move-apikey-issuance-to-external-portal` D2。
/// 旧路径 MUST NOT 保留为兼容别名（改名即让旧调用点 404，消灭隐性第二入口）。
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/external/api-clients",
        web::get().to(list_external_api_clients),
    )
    .route(
        "/external/api-clients",
        web::post().to(create_external_api_client),
    )
    .route(
        "/external/api-clients/{id}/rotate-secret",
        web::post().to(rotate_external_api_client_secret),
    )
    .route(
        "/external/api-clients/{id}",
        web::delete().to(delete_external_api_client),
    );
}
