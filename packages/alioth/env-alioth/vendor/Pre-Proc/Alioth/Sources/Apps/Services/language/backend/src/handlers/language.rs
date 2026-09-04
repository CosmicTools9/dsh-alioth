//! 语言 Handler — 通过 code LIKE 'lang:%' 查询 isahl.zc_id_prot-env_config
//! 元数据（locale/enabled/coverage）存储在 settings JSONB 列，
//! SELECT 时以 settings AS notice 返回，兼容前端 toFrontend() 解析。
//! 数据访问在 repositories/language_query.rs（F7 分层）。
use actix_web::{web, HttpRequest, HttpResponse};
use common::error::AliothError as ApiError;
use common::ApiResponse;
use crud::parse_visible_ids;
use sqlx::PgPool;

use crate::repositories::language_query::{LanguageRepository, PREFIX};

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/languages")
            .route(web::get().to(list))
            .route(web::post().to(create)),
    )
    .service(
        web::resource("/languages/{id}")
            .route(web::get().to(get))
            .route(web::patch().to(update))
            .route(web::delete().to(del)),
    );
}

/// 从请求体中提取 metadata JSON（locale/enabled/coverage）
fn extract_meta(b: &serde_json::Value) -> serde_json::Value {
    let mut meta = serde_json::json!({});
    if let Some(locale) = b.get("locale").and_then(|v| v.as_str()) {
        meta.as_object_mut().unwrap().insert(
            "locale".into(),
            serde_json::Value::String(locale.to_string()),
        );
    }
    if let Some(v) = b.get("enabled") {
        meta.as_object_mut()
            .unwrap()
            .insert("enabled".into(), v.clone());
    }
    if let Some(v) = b.get("coverage") {
        meta.as_object_mut()
            .unwrap()
            .insert("coverage".into(), v.clone());
    }
    meta
}

async fn list(pool: web::Data<PgPool>, req: HttpRequest) -> Result<HttpResponse, ApiError> {
    let visible = parse_visible_ids(&req);
    let items = LanguageRepository::new(pool.get_ref().clone())
        .list(visible.as_deref())
        .await?;
    Ok(HttpResponse::Ok().json(serde_json::json!({"items": items, "total": items.len()})))
}

async fn get(pool: web::Data<PgPool>, path: web::Path<i64>) -> Result<HttpResponse, ApiError> {
    let item = LanguageRepository::new(pool.get_ref().clone())
        .get(path.into_inner())
        .await?;
    match item {
        Some(e) => Ok(HttpResponse::Ok().json(e)),
        None => Ok(HttpResponse::NotFound().json(serde_json::json!({"error": "not_found"}))),
    }
}

async fn create(
    pool: web::Data<PgPool>,
    body: web::Json<serde_json::Value>,
    req: HttpRequest,
) -> Result<HttpResponse, ApiError> {
    let user_id = common::context::require_auth(&req)?;
    let b = body.into_inner();
    let locale = b.get("locale").and_then(|v| v.as_str()).unwrap_or("");
    let code = format!("{}{}", PREFIX, locale);
    let meta = extract_meta(&b);

    let item = LanguageRepository::new(pool.get_ref().clone())
        .create(
            b.get("name").and_then(|v| v.as_str()).unwrap_or(""),
            &code,
            &meta,
            user_id,
        )
        .await?;

    Ok(HttpResponse::Created().json(ApiResponse::success(item)))
}

async fn update(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<serde_json::Value>,
    req: HttpRequest,
) -> Result<HttpResponse, ApiError> {
    let user_id = common::context::require_auth(&req)?;
    let b = body.into_inner();
    let id = path.into_inner();
    let meta = extract_meta(&b);

    let item = LanguageRepository::new(pool.get_ref().clone())
        .update(id, b.get("name").and_then(|v| v.as_str()), &meta, user_id)
        .await?;
    match item {
        Some(e) => Ok(HttpResponse::Ok().json(ApiResponse::success(e))),
        None => Ok(HttpResponse::NotFound().json(serde_json::json!({"error": "not_found"}))),
    }
}

async fn del(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    req: HttpRequest,
) -> Result<HttpResponse, ApiError> {
    let user_id = common::context::require_auth(&req)?;
    let deleted = LanguageRepository::new(pool.get_ref().clone())
        .delete(path.into_inner(), user_id)
        .await?;
    if !deleted {
        return Ok(HttpResponse::NotFound().json(serde_json::json!({"error": "not_found"})));
    }
    Ok(HttpResponse::NoContent().finish())
}
