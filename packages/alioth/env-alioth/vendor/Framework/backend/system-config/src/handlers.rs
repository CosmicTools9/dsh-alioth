//! System Config HTTP Handlers
//!
//! 提供 RESTful API 路由 handler 函数，供 Gateway / Meta 后端集成。
//! 本模块不绑定具体 Repository 实现，通过泛型 `SystemConfigService<R>` 注入。

use actix_web::{web, HttpResponse};
use common::{AliothError, ApiResponse};
use serde::Deserialize;

use crate::models::{CreateSystemConfigRequest, UpdateSystemConfigRequest};
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
/// 创建配置
pub async fn create_config<R: SystemConfigRepository>(
    service: web::Data<SystemConfigService<R>>,
    req: web::Json<CreateSystemConfigRequest>,
) -> Result<HttpResponse, AliothError> {
    match service.create(req.into_inner()).await {
        Ok(config) => Ok(HttpResponse::Ok().json(ApiResponse::success(config))),
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
/// 更新配置
pub async fn update_config<R: SystemConfigRepository>(
    service: web::Data<SystemConfigService<R>>,
    path: web::Path<i64>,
    req: web::Json<UpdateSystemConfigRequest>,
) -> Result<HttpResponse, AliothError> {
    match service.update(path.into_inner(), req.into_inner()).await {
        Ok(Some(config)) => Ok(HttpResponse::Ok().json(ApiResponse::success(config))),
        Ok(None) => Err(AliothError::NotFound("Config not found".into())),
        Err(e) => Err(e.into()),
    }
}

/// DELETE /system-config/{id}
/// 删除配置（软删除）
pub async fn delete_config<R: SystemConfigRepository>(
    service: web::Data<SystemConfigService<R>>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    match service.delete(path.into_inner()).await {
        Ok(rows) if rows > 0 => {
            Ok(HttpResponse::Ok()
                .json(ApiResponse::success(serde_json::json!({ "deleted": true }))))
        }
        Ok(_) => Err(AliothError::NotFound("Config not found".into())),
        Err(e) => Err(e.into()),
    }
}

// ============================================
// Route Configuration
// ============================================

/// 注册系统配置路由。
///
/// 使用方传入已实现 `SystemConfigRepository` 的 Repository 实例，
/// 本函数将其包装为 `SystemConfigService` 并注册到 Actix 路由。
///
/// # 示例
/// ```rust,ignore
/// use system_config::{configure_routes, SystemConfigRepository};
///
/// // 假设 MyRepo 已实现 SystemConfigRepository
/// let repo = MyRepo::new(pool);
/// app.configure(|cfg| configure_routes(repo, cfg));
/// ```
pub fn configure_routes<R: SystemConfigRepository + 'static>(
    repo: R,
    cfg: &mut web::ServiceConfig,
) {
    let service = web::Data::new(SystemConfigService::new(repo));
    cfg.app_data(service.clone());
    cfg.service(
        web::scope("/system-config")
            .app_data(service)
            .route("/categories", web::get().to(list_categories))
            .route("/categories/{code}", web::get().to(get_category))
            .route("", web::get().to(list_configs::<R>))
            .route("", web::post().to(create_config::<R>))
            .route("/{id}", web::get().to(get_config::<R>))
            .route("/{id}", web::put().to(update_config::<R>))
            .route("/{id}", web::delete().to(delete_config::<R>)),
    );
}
