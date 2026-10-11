//! System Config HTTP Handlers
//!
//! 提供 RESTful API 路由 handler 函数，供 Gateway / Meta 后端集成。
//! 本模块不绑定具体 Repository 实现，通过泛型 `SystemConfigService<R>` 注入。

use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use common::context::RequestContext;
use common::{AliothError, ApiResponse};
use serde::Deserialize;
use sqlx::PgPool;

use crate::models::{CreateSystemConfigRequest, UpdateSystemConfigRequest};
use crate::probe::ProbeRegistry;
use crate::repository::SystemConfigRepository;
use crate::schema;
use crate::service::{SystemConfigError, SystemConfigService};

impl From<SystemConfigError> for AliothError {
    fn from(err: SystemConfigError) -> Self {
        match err {
            SystemConfigError::NotFound(id) => {
                AliothError::NotFound(format!("Config {} not found", id))
            }
            SystemConfigError::Validation(msg) => AliothError::BadRequest(msg),
            SystemConfigError::Database(e) => {
                common::telemetry::error!("Database error: {}", e);
                AliothError::Database(e.to_string())
            }
            SystemConfigError::Crypto(e) => {
                common::telemetry::error!("Crypto error: {}", e);
                AliothError::Internal(format!("Crypto error: {}", e))
            }
        }
    }
}

// ============================================
// Query Types
// ============================================

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_limit")]
    #[serde(with = "common::serde_zuid")]
    pub limit: i64,
    #[serde(default = "default_offset")]
    #[serde(with = "common::serde_zuid")]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}
fn default_offset() -> i64 {
    0
}

/// 提取操作者 `(user_id, subject, email)`；`RequestContext` 缺失 ⇒ 401。
///
/// 写路径 fail-closed：规约 `config-change-audit` 要求「写操作 MUST 有操作者且落审计」，
/// 该不变量在入口强制（与 llm/test 端点同口径），MUST NOT 伪造操作者或静默跳过审计。
/// 审计主体标识走 `common::audit::resolve_subject` 唯一实现（SECURITY_SPEC §10.1/§11.2）。
fn actor_from(req: &HttpRequest) -> Result<(i64, String, String), AliothError> {
    let extensions = req.extensions();
    let ctx = extensions
        .get::<RequestContext>()
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".to_string()))?;
    Ok((
        ctx.user_id,
        common::audit::resolve_subject(&ctx.username, ctx.user_id),
        ctx.email.clone(),
    ))
}

// ============================================
// Handlers
// ============================================

/// GET /system-config/categories
/// 获取所有支持的配置分类及 Schema（无 DB 依赖）
pub async fn list_categories() -> Result<HttpResponse, AliothError> {
    Ok(HttpResponse::Ok().json(ApiResponse::success(
        crate::models::ConfigCategoryListResponse {
            categories: schema::get_all_categories(),
        },
    )))
}

/// GET /system-config/categories/{code}
/// 获取指定分类的 Schema（无 DB 依赖）
pub async fn get_category(path: web::Path<String>) -> Result<HttpResponse, AliothError> {
    match schema::get_category(&path.into_inner()) {
        Some(category) => Ok(HttpResponse::Ok().json(ApiResponse::success(category))),
        None => Err(AliothError::NotFound("Category not found".into())),
    }
}

/// GET /system-config
/// 列出配置（支持按分类筛选）
pub async fn list_configs<R: SystemConfigRepository>(
    service: web::Data<SystemConfigService<R>>,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, AliothError> {
    match service.list(query.limit, query.offset).await {
        Ok(configs) => Ok(HttpResponse::Ok().json(ApiResponse::success(configs))),
        Err(e) => Err(e.into()),
    }
}

/// POST /system-config
/// 创建配置（成功即落 `system_config.create` 审计 + `created_by_id` 归属）
pub async fn create_config<R: SystemConfigRepository>(
    service: web::Data<SystemConfigService<R>>,
    pool: web::Data<PgPool>,
    http_req: HttpRequest,
    req: web::Json<CreateSystemConfigRequest>,
) -> Result<HttpResponse, AliothError> {
    let actor = actor_from(&http_req)?;
    match service.create(req.into_inner(), Some(actor.0)).await {
        Ok(config) => {
            crate::operation::audit_write(
                &pool,
                actor.0,
                &actor.1,
                "system_config.create",
                &format!(
                    "system_config/{}/{}",
                    config._f_.as_deref().unwrap_or_default(),
                    config.id
                ),
                serde_json::json!({
                    "category": &config._f_,
                    "code": &config.code,
                    "provider": &config._t_,
                    "email": &actor.2,
                }),
            )
            .await;
            Ok(HttpResponse::Ok().json(ApiResponse::success(config)))
        }
        Err(e) => Err(e.into()),
    }
}

/// GET /system-config/{id}
/// 获取单个配置（敏感字段已隐藏）
pub async fn get_config<R: SystemConfigRepository>(
    service: web::Data<SystemConfigService<R>>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    match service.find_by_id(path.into_inner()).await {
        Ok(Some(config)) => Ok(HttpResponse::Ok().json(ApiResponse::success(config))),
        Ok(None) => Err(AliothError::NotFound("Config not found".into())),
        Err(e) => Err(e.into()),
    }
}

/// PUT /system-config/{id}
/// 更新配置（成功即落 `system_config.update` 审计 + `updated_by_id` 归属）
pub async fn update_config<R: SystemConfigRepository>(
    service: web::Data<SystemConfigService<R>>,
    pool: web::Data<PgPool>,
    http_req: HttpRequest,
    path: web::Path<i64>,
    req: web::Json<UpdateSystemConfigRequest>,
) -> Result<HttpResponse, AliothError> {
    let actor = actor_from(&http_req)?;
    let id = path.into_inner();
    match service.update(id, req.into_inner(), Some(actor.0)).await {
        Ok(Some(config)) => {
            crate::operation::audit_write(
                &pool,
                actor.0,
                &actor.1,
                "system_config.update",
                &format!(
                    "system_config/{}/{}",
                    config._f_.as_deref().unwrap_or_default(),
                    id
                ),
                serde_json::json!({
                    "category": &config._f_,
                    "code": &config.code,
                    "provider": &config._t_,
                    "email": &actor.2,
                }),
            )
            .await;
            Ok(HttpResponse::Ok().json(ApiResponse::success(config)))
        }
        Ok(None) => Err(AliothError::NotFound("Config not found".into())),
        Err(e) => Err(e.into()),
    }
}

/// DELETE /system-config/{id}
/// 删除配置（软删除；成功即落 `system_config.delete` 审计）
pub async fn delete_config<R: SystemConfigRepository>(
    service: web::Data<SystemConfigService<R>>,
    pool: web::Data<PgPool>,
    http_req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let actor = actor_from(&http_req)?;
    let id = path.into_inner();
    match service.delete(id).await {
        Ok(rows) if rows > 0 => {
            crate::operation::audit_write(
                &pool,
                actor.0,
                &actor.1,
                "system_config.delete",
                &format!("system_config/{id}"),
                serde_json::json!({ "id": id.to_string(), "email": &actor.2 }),
            )
            .await;
            Ok(HttpResponse::Ok()
                .json(ApiResponse::success(serde_json::json!({ "deleted": true }))))
        }
        Ok(_) => Err(AliothError::NotFound("Config not found".into())),
        Err(e) => Err(e.into()),
    }
}

/// 探测请求体（`{id}` 已存行 与 `{config}` 表单草稿二选一；同时给时 `config` 覆盖行字段）
#[derive(Debug, Deserialize)]
pub struct ProbeRequest {
    #[serde(default, with = "common::serde_zuid::opt")]
    pub id: Option<i64>,
    /// 表单草稿（键同 settings；敏感键按 schema 分流进 credentials，仅请求期明文）
    pub config: Option<serde_json::Value>,
}

/// POST /system-config/test/{category}
/// 连通性探测（统一 seam；scope 内单一路由，无宿主级注册顺序约束）。
///
/// - `{id}`：已存行经 `common::config_resolution::load_by_id`（凭证解密）读取，`config` 覆盖字段不落库；
/// - `{config}`：草稿按 `schema::get_sensitive_keys` 分流——敏感键移入 credentials，其余为 settings；
/// - 无探测器分类 ⇒ 400；请求上下文缺失 ⇒ 401（探测可触达已存凭证，MUST NOT 匿名可达）。
pub async fn probe_category(
    pool: web::Data<PgPool>,
    probes: web::Data<ProbeRegistry>,
    http_req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<ProbeRequest>,
) -> Result<HttpResponse, AliothError> {
    let _actor = actor_from(&http_req)?;
    let category = path.into_inner();
    let Some(probe) = probes.get(&category) else {
        return Err(AliothError::BadRequest(format!(
            "分类 {category} 不支持连通性测试"
        )));
    };
    let body = body.into_inner();

    let (provider, settings, credentials) = match (body.id, body.config.as_ref()) {
        (Some(id), overrides) => {
            let family = common::config_resolution::ConfigFamily::from_category(&category)
                .ok_or_else(|| AliothError::BadRequest(format!("分类 {category} 无解析映射")))?;
            let cfg = common::config_resolution::load_by_id(
                &pool,
                family,
                id,
                &crate::crypto::decrypt_prefixed,
            )
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?
            .ok_or_else(|| AliothError::NotFound(format!("配置 {id} 不存在")))?;
            let mut settings = cfg.settings;
            if let Some(src) = overrides.and_then(|o| o.as_object()) {
                if let Some(dst) = settings.as_object_mut() {
                    for (k, v) in src {
                        if !v.is_null() {
                            dst.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            let provider = cfg
                .provider
                .or_else(|| {
                    settings
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .unwrap_or_default();
            (provider, settings, cfg.credentials)
        }
        (None, Some(config)) => {
            let provider = config
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let sensitive = schema::get_sensitive_keys(&category, &provider);
            let mut settings = config.clone();
            let mut credentials = serde_json::Map::new();
            if let Some(obj) = settings.as_object_mut() {
                for key in &sensitive {
                    if let Some(v) = obj.remove(key) {
                        credentials.insert(key.clone(), v);
                    }
                }
            }
            (provider, settings, serde_json::Value::Object(credentials))
        }
        (None, None) => {
            return Err(AliothError::BadRequest(
                "缺少 id 或 config（新建配置时需填写表单或已存配置 id）".to_string(),
            ))
        }
    };

    let started = std::time::Instant::now();
    let mut outcome = probe.probe(&provider, &settings, &credentials).await;
    if outcome.latency_ms == 0 {
        outcome.latency_ms = started.elapsed().as_millis() as u64;
    }
    Ok(HttpResponse::Ok().json(ApiResponse::success(outcome)))
}

// ============================================
// Route Configuration
// ============================================

/// 注册系统配置路由。
///
/// 使用方传入已实现 `SystemConfigRepository` 的 Repository 实例、`PgPool`（变更审计写入 /
/// 探测取行）与 `ProbeRegistry`（各类别连通性探测实现，宿主注入）——后两者由本函数注册为
/// scope 内 app_data。本函数将 repo 包装为 `SystemConfigService` 并注册到 Actix 路由。
///
/// # 示例
/// ```rust,ignore
/// use system_config::{configure_routes, SystemConfigRepository};
///
/// // 假设 MyRepo 已实现 SystemConfigRepository、MyEmailProbe 已实现 ProviderProbe
/// let repo = MyRepo::new(pool.clone());
/// let probes = system_config::probe::ProbeRegistry::new().with("email", MyEmailProbe);
/// app.configure(|cfg| configure_routes(repo, pool, probes, cfg));
/// ```
pub fn configure_routes<R: SystemConfigRepository + 'static>(
    repo: R,
    pool: PgPool,
    probes: ProbeRegistry,
    cfg: &mut web::ServiceConfig,
) {
    let service = web::Data::new(SystemConfigService::new(repo));
    cfg.app_data(service.clone());
    cfg.service(
        web::scope("/system-config")
            .app_data(service)
            .app_data(web::Data::new(pool))
            .app_data(web::Data::new(probes))
            .route("/categories", web::get().to(list_categories))
            .route("/categories/{code}", web::get().to(get_category))
            .route("", web::get().to(list_configs::<R>))
            .route("", web::post().to(create_config::<R>))
            .route("/test/{category}", web::post().to(probe_category))
            .route("/{id}", web::get().to(get_config::<R>))
            .route("/{id}", web::put().to(update_config::<R>))
            .route("/{id}", web::delete().to(delete_config::<R>)),
    );
}
