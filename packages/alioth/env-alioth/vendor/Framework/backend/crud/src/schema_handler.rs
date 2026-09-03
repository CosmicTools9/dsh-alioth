//! actix-web 路由工厂使用 `SchemaRepository` 替代 `OntologyDispatcher`。
//!
//! 每个模块调用 `schema_routes(cfg, "/{module}")` 即可挂载完整 CRUD：
//!
//! ```rust,ignore
//! pub fn config(cfg: &mut web::ServiceConfig) {
//!     // ... 已有精确泛型路由 ...
//!     crud::schema_routes(cfg, "/channel");
//! }
//! ```

use actix_web::{web, HttpRequest, HttpResponse};
use common::{AliothError, ApiResponse};
use serde::Deserialize;
use serde_json::Value;
use sqlx::PgPool;

use crate::document_ingester::ingest_document;
use crate::schema_repository::SchemaRepository;

#[derive(Debug, Deserialize)]
pub struct LeafPath {
    pub table: String,
}

#[derive(Debug, Deserialize)]
pub struct LeafItemPath {
    pub table: String,
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct SchemaListQuery {
    #[serde(default = "default_page")]
    #[serde(with = "common::serde_zuid")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    #[serde(with = "common::serde_zuid")]
    pub page_size: i64,
    /// 是否包含 `_refs` 解析（默认 false 提升性能）
    #[serde(default)]
    pub with_refs: bool,
}

fn default_page() -> i64 {
    1
}

fn default_page_size() -> i64 {
    20
}

/// 挂载 schema 路由到指定 scope。
pub fn schema_routes(cfg: &mut web::ServiceConfig, base: &str) {
    cfg.service(
        web::scope(base)
            .route("/leaf/{table}", web::get().to(schema_list))
            .route("/leaf/{table}/{id}", web::get().to(schema_get))
            .route("/leaf/{table}", web::post().to(schema_create))
            .route("/leaf/{table}/{id}", web::put().to(schema_update))
            .route("/leaf/{table}/{id}", web::delete().to(schema_delete))
            // 文档级联创建：POST /{base}/ingest/{root_table}
            .route("/ingest/{table}", web::post().to(schema_ingest)),
    );
}

// ─── Handlers ──────────────────────────────────────────────────────

async fn schema_list(
    path: web::Path<String>,
    query: web::Query<SchemaListQuery>,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, AliothError> {
    let table = path.into_inner();
    let repo = SchemaRepository::new(pool.get_ref().clone());
    let data = if query.with_refs {
        repo.list_with_refs(&table, query.page, query.page_size)
            .await?
    } else {
        repo.list(&table, query.page, query.page_size).await?
    };
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "data": data,
            "table": table,
            "page": query.page,
            "page_size": query.page_size,
        }))),
    )
}

async fn schema_get(
    path: web::Path<LeafItemPath>,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, AliothError> {
    let LeafItemPath { table, id } = path.into_inner();
    let repo = SchemaRepository::new(pool.get_ref().clone());
    match repo.get(&table, id).await? {
        Some(v) => Ok(HttpResponse::Ok().json(ApiResponse::success(v))),
        None => Err(AliothError::NotFound("not_found".into())),
    }
}

async fn schema_create(
    path: web::Path<String>,
    body: web::Json<Value>,
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, AliothError> {
    let user_id = crate::handler::extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let table = path.into_inner();
    let repo = SchemaRepository::new(pool.get_ref().clone());
    let new_id = repo.create(&table, body.into_inner(), user_id).await?;
    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": new_id,
            "table": table,
        }))),
    )
}

async fn schema_update(
    path: web::Path<LeafItemPath>,
    body: web::Json<Value>,
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, AliothError> {
    let user_id = crate::handler::extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let LeafItemPath { table, id } = path.into_inner();
    let repo = SchemaRepository::new(pool.get_ref().clone());
    match repo.update(&table, id, body.into_inner(), user_id).await? {
        Some(v) => Ok(HttpResponse::Ok().json(ApiResponse::success(v))),
        None => Err(AliothError::NotFound("not_found".into())),
    }
}

async fn schema_delete(
    path: web::Path<LeafItemPath>,
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, AliothError> {
    let user_id = crate::handler::extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let LeafItemPath { table, id } = path.into_inner();
    let repo = SchemaRepository::new(pool.get_ref().clone());
    repo.delete(&table, id, user_id).await?;
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "id": id,
            "table": table,
            "deleted": true,
        }))),
    )
}

/// POST /{base}/ingest/{table} — 文档级联创建
async fn schema_ingest(
    path: web::Path<String>,
    body: web::Json<Value>,
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, AliothError> {
    let user_id = crate::handler::extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let table = path.into_inner();
    let new_id = ingest_document(pool.get_ref(), &table, body.into_inner(), user_id).await?;
    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": new_id,
            "table": table,
        }))),
    )
}
