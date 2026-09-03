//! Reusable actix-web handlers for the ontology dispatcher.
//!
//! Each module that wants to expose CRUD over its lifecycle leaves can call
//! `ontology_routes::<MyBindings>(cfg, "/my_module")` to mount:
//!   GET    /my_module/leaf/{table}
//!   GET    /my_module/leaf/{table}/{id}
//!   POST   /my_module/leaf/{table}
//!
//! The binding is looked up by table_name via the per-module
//! `get_ontology_binding()` function supplied as a type parameter.
//!
//! # Example
//!
//! ```rust,ignore
//! pub fn config(cfg: &mut web::ServiceConfig) {
//!     ontology_routes::<crate::ontology_bindings::Bindings>(
//!         cfg, "/modules/channel", &crate::ontology_bindings::get_ontology_binding
//!     );
//! }
//! ```

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::extract_user_id;
use common::{AliothError, ApiResponse};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};

use crate::schema_repository::{AliothLeaf, SchemaRepository};

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

#[derive(Debug, Serialize)]
pub struct LeafListResponse {
    pub data: Vec<serde_json::Value>,
    pub table: String,
}

/// Mount the standard ontology routes under `base`.
///
/// `binding_lookup` is a function that takes a table name and returns the
/// (scene, factor, function) triple.
pub fn ontology_routes<F>(cfg: &mut web::ServiceConfig, base: &str, binding_lookup: F)
where
    F: Fn(&str) -> (Option<i64>, Option<i64>, Option<i64>) + Send + Sync + 'static + Clone,
{
    let lookup = web::Data::new(binding_lookup);
    cfg.app_data(lookup)
        .app_data(web::Data::new(PaginationParams::default()))
        .service(
            web::scope(base)
                .route("/leaf/{table}", web::get().to(list_leaf))
                .route("/leaf/{table}/{id}", web::get().to(get_leaf))
                .route("/leaf/{table}", web::post().to(create_leaf::<F>))
                .route("/leaf/{table}/{id}", web::delete().to(delete_leaf)),
        );
}

/// 只读引用路由变体：`ontology_routes` 的全部路由 + `GET /reference/{table}` /
/// `GET /reference/{table}/{id}`。
///
/// 枚举下拉场景（AVIC-CAASEC airworthiness 表单）需要读取**非叶**受管表
/// （如 `zc_id_process` 4 行数据、`zc_id_subjects`）作为可选值源——leaf endpoint
/// 的 `is_leaf_table` 契约正确拒绝这些表，枚举读取不应被 leaf-only 契约卡死。
///
/// 安全约束（防 URL 参数变任意表读取）：
/// - 授权白名单 `allowlist`（`Arc<HashSet<String>>`）由调用 service 注入——
///   明确列出允许读取的枚举源表（fk_index 未覆盖的独立枚举叶表）；
/// - 授权判定：fk_index 覆盖 或 allowlist 命中，二者皆无则拒绝；
/// - repository 仅做表存在性校验，安全策略不散落；
/// - **只读**：不挂 POST/PUT/DELETE，写路径保持 leaf-only。
///
/// ⚠️ 原 `ontology_routes` 不挂 reference（避免现有调用方无意暴露）；需
/// reference 的服务显式调用本函数并提供 allowlist。
pub fn ontology_routes_with_reference<F>(
    cfg: &mut web::ServiceConfig,
    base: &str,
    binding_lookup: F,
    reference_allowlist: std::sync::Arc<std::collections::HashSet<String>>,
) where
    F: Fn(&str) -> (Option<i64>, Option<i64>, Option<i64>) + Send + Sync + 'static + Clone,
{
    let lookup = web::Data::new(binding_lookup);
    cfg.app_data(lookup)
        .app_data(web::Data::new(PaginationParams::default()))
        .app_data(web::Data::new(reference_allowlist))
        .service(
            web::scope(base)
                .route("/leaf/{table}", web::get().to(list_leaf))
                .route("/leaf/{table}/{id}", web::get().to(get_leaf))
                .route("/leaf/{table}", web::post().to(create_leaf::<F>))
                .route("/leaf/{table}/{id}", web::delete().to(delete_leaf))
                .route("/reference/{table}", web::get().to(list_reference))
                .route("/reference/{table}/{id}", web::get().to(get_reference)),
        );
}

/// 兼容占位：reference 路由由 `ontology_routes_with_reference` 挂载，本函数
/// 不额外注册任何路由（避免同 scope 重复 shadow）。
pub fn reference_routes<F>(cfg: &mut web::ServiceConfig, _base: &str, _binding_lookup: F)
where
    F: Fn(&str) -> (Option<i64>, Option<i64>, Option<i64>) + Send + Sync + 'static + Clone,
{
    let _ = cfg;
}

/// 校验 table 的**静态**合法性（快速预检）：仅字母/数字/下划线，防 SQL 注入。
fn is_allowed_reference_table(table: &str) -> bool {
    !table.is_empty()
        && table
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// reference 授权判定：fk_index 覆盖（被引用/引用了其他表）或注入 allowlist 命中。
/// 授权在 handler 层完成，repository 仅做表存在性校验。
fn is_authorized_reference(
    table: &str,
    allowlist: &std::sync::Arc<std::collections::HashSet<String>>,
) -> bool {
    let fk_covered = !crate::fk_index::lookup_reverse_fk(table).is_empty()
        || !crate::fk_index::lookup_forward_fk(table).is_empty();
    fk_covered || allowlist.contains(table)
}

async fn list_reference(
    path: web::Path<String>,
    query: web::Query<PaginationParams>,
    pool: web::Data<PgPool>,
    allowlist: web::Data<std::sync::Arc<std::collections::HashSet<String>>>,
) -> Result<HttpResponse, AliothError> {
    let table = path.into_inner();
    if !is_allowed_reference_table(&table) || !is_authorized_reference(&table, &allowlist) {
        return Err(AliothError::BadRequest(format!(
            "{} 不是受管引用目标（fk_index 未覆盖且不在 allowlist）",
            table
        )));
    }
    let dispatcher = SchemaRepository::new(pool.get_ref().clone());
    let data = dispatcher
        .list_reference(&table, query.page, query.page_size)
        .await?;
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(LeafListResponse {
            data,
            table: table.clone(),
        })),
    )
}

async fn get_reference(
    path: web::Path<LeafItemPath>,
    pool: web::Data<PgPool>,
    allowlist: web::Data<std::sync::Arc<std::collections::HashSet<String>>>,
) -> Result<HttpResponse, AliothError> {
    let LeafItemPath { table, id } = path.into_inner();
    if !is_allowed_reference_table(&table) || !is_authorized_reference(&table, &allowlist) {
        return Err(AliothError::BadRequest(format!(
            "{} 不是受管引用目标（fk_index 未覆盖且不在 allowlist）",
            table
        )));
    }
    let dispatcher = SchemaRepository::new(pool.get_ref().clone());
    let row = dispatcher.get_reference(&table, id).await?;
    match row {
        Some(v) => Ok(HttpResponse::Ok().json(ApiResponse::success(v))),
        None => Err(AliothError::NotFound("not_found".into())),
    }
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct PaginationParams {
    #[serde(default = "default_page")]
    #[serde(with = "common::serde_zuid")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    #[serde(with = "common::serde_zuid")]
    pub page_size: i64,
}

fn default_page() -> i64 {
    1
}

fn default_page_size() -> i64 {
    20
}

async fn list_leaf(
    path: web::Path<String>,
    query: web::Query<PaginationParams>,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, AliothError> {
    let dispatcher = SchemaRepository::new(pool.get_ref().clone());
    let data = dispatcher
        .list_leaf(&path, query.page, query.page_size)
        .await?;
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(LeafListResponse {
            data,
            table: path.into_inner(),
        })),
    )
}

async fn get_leaf(
    path: web::Path<LeafItemPath>,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, AliothError> {
    let LeafItemPath { table, id } = path.into_inner();
    let dispatcher = SchemaRepository::new(pool.get_ref().clone());
    let row = dispatcher.get_leaf(&table, id).await?;
    match row {
        Some(v) => Ok(HttpResponse::Ok().json(ApiResponse::success(v))),
        None => Err(AliothError::NotFound("not_found".into())),
    }
}

async fn create_leaf<F>(
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
    req: HttpRequest,
    pool: web::Data<PgPool>,
    lookup: web::Data<F>,
) -> Result<HttpResponse, AliothError>
where
    F: Fn(&str) -> (Option<i64>, Option<i64>, Option<i64>) + Send + Sync + 'static,
{
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let table = path.into_inner();
    if table.starts_with("vw_") {
        return Err(AliothError::BadRequest(format!(
            "{} 是只读投影视图，禁止创建（写路径走基础表）",
            table
        )));
    }
    let dispatcher = SchemaRepository::new(pool.get_ref().clone());
    let leaf = body.into_inner();
    let leaf_obj = parse_leaf(leaf)?;
    let binding = lookup.get_ref()(&table);
    let new_id = dispatcher
        .create_in_leaf(&table, binding, leaf_obj, user_id)
        .await?;
    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": new_id,
            "table": table,
        }))),
    )
}

async fn delete_leaf(
    path: web::Path<LeafItemPath>,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, AliothError> {
    let LeafItemPath { table, id } = path.into_inner();
    let dispatcher = SchemaRepository::new(pool.get_ref().clone());
    if table.starts_with("vw_") {
        return Err(AliothError::BadRequest(format!(
            "{} 是只读投影视图，禁止删除（写路径走基础表）",
            table
        )));
    }
    if !dispatcher.is_leaf_table(&table).await? {
        return Err(AliothError::BadRequest(format!("{} is not a leaf", table)));
    }
    let sql = format!(
        r#"UPDATE isahl."{}" SET deleted_at = NOW(), deleted_by_id = 1 WHERE id = $1 AND deleted_at IS NULL"#,
        table
    );
    let rows = sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(id)
        .execute(pool.get_ref())
        .await
        .map_err(|e| AliothError::Database(e.to_string()))?;
    if rows.rows_affected() == 0 {
        return Err(AliothError::NotFound("not_found_or_already_deleted".into()));
    }
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "id": id,
            "table": table,
            "deleted": true
        }))),
    )
}

fn parse_leaf(v: serde_json::Value) -> Result<AliothLeaf, AliothError> {
    match v {
        serde_json::Value::Object(map) => Ok(AliothLeaf { fields: map }),
        _ => Err(AliothError::BadRequest(
            "request body must be a JSON object".into(),
        )),
    }
}
