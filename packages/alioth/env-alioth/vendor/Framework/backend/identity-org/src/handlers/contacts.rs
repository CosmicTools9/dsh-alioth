//! 通讯录薄包装 Handler — 复用 Framework/contacts ContactsService
//!
//! 端点（挂载在 /service/isahl-db）：
//! - `GET /contacts`       — 联系人列表（分页）
//! - `POST /contacts`      — 创建联系人
//! - `GET /contacts/{id}`  — 获取单个联系人
//! - `PUT /contacts/{id}`  — 更新联系人
//! - `DELETE /contacts/{id}` — 删除联系人（软删除）
//!
//! DTO：`{id, name, code, comments, infos:[{kind,value,is_default}]}`
//! L2 语义命名：notice→name（由 Framework contacts 提供）
//!
//! 复用策略（REUSE_FIRST）：create/update/delete 委托 `ContactsService` 公开方法；
//! list 因需 q 关键字过滤（ContactsService 无 keyword 支持，crud raw_filter 不支持
//! 参数绑定）改由 handler 侧 sqlx QueryBuilder 参数化查询，行映射复用 get 同款
//! `SELECT_FIELDS` + `build_refs_select_suffix`（不重写聚合解析）；
//! code/comments 随请求体直传 Framework 模型（Create/UpdateContactRequest），同事务落列，
//! 响应始终从真实行映射。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use crud::entity::AliothDbEntity;
use crud::reference::build_refs_select_suffix;
use framework_contacts::{
    models::{
        ContactInfoInput, ContactInfoValue, ContactsEntity, CreateContactRequest,
        UpdateContactRequest,
    },
    ContactsService,
};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};

/// 前端 DTO：通讯录联系人（camelCase）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactDto {
    #[serde(with = "common::serde_zuid")]
    id: i64,
    name: String,
    code: String,
    comments: String,
    infos: Vec<ContactInfoValue>,
}

/// 单条查询行（id/name/code/comments + refs 后缀）
#[derive(sqlx::FromRow)]
struct ContactRow {
    id: i64,
    #[sqlx(rename = "notice")]
    name: Option<String>,
    #[sqlx(default)]
    code: Option<String>,
    #[sqlx(default)]
    comments: Option<String>,
    #[sqlx(default)]
    _refs: Option<serde_json::Value>,
}

/// 从真实行映射 ContactDto（code/comments 取行值，不再硬编码空串）
fn contact_row_to_dto(row: ContactRow) -> ContactDto {
    let infos = parse_contact_infos_from_refs(&row._refs);
    ContactDto {
        id: row.id,
        name: row.name.unwrap_or_default(),
        code: row.code.unwrap_or_default(),
        comments: row.comments.unwrap_or_default(),
        infos,
    }
}

/// 按 id 读取真实行并映射 DTO（create/update/get 共用）
async fn fetch_contact_dto(pool: &PgPool, id: i64) -> Result<Option<ContactDto>, ApiError> {
    let refs_suffix = build_refs_select_suffix::<ContactsEntity>();
    let sql = format!(
        "SELECT e.id, e.notice AS notice, e.code AS code, e.comments AS comments {} FROM {} AS e WHERE e.id = $1 AND e.deleted_at IS NULL",
        refs_suffix,
        ContactsEntity::table_name()
    );

    let row: Option<ContactRow> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::from_sqlx)?;

    Ok(row.map(contact_row_to_dto))
}

/// ContactsService 返回 String 错误 → 统一映射为 Database 错误
fn map_service_err(e: String) -> ApiError {
    ApiError::Database(e)
}

/// 分页查询参数（snake_case 契约：page/page_size/q，对齐 subjects.rs SubjectListQuery）
#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    #[serde(default = "default_page")]
    page: i64,
    #[serde(default = "default_page_size")]
    page_size: i64,
    /// 搜索词（匹配 notice / code）
    q: Option<String>,
}

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    20
}

/// 向查询构建器追加关键字过滤条件（参数化绑定，防注入；与 subjects.rs 同范式）
fn push_keyword_filter(builder: &mut sqlx::QueryBuilder<sqlx::Postgres>, q: &Option<String>) {
    let kw = q.as_deref().map(str::trim).unwrap_or("");
    if kw.is_empty() {
        return;
    }
    let pat = format!("%{}%", kw);
    builder.push(" AND (e.notice ILIKE ");
    builder.push_bind(pat.clone());
    builder.push(" OR e.code ILIKE ");
    builder.push_bind(pat);
    builder.push(")");
}

/// GET /contacts — 联系人列表（分页 + q 关键字过滤）
pub async fn list_contacts(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    query: web::Query<PaginationQuery>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "contacts", 0, "list").await?;

    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;

    // 行级查询：SELECT_FIELDS 同语义（id/notice/code/comments + _refs 后缀），
    // q 过滤走参数化绑定（ContactsService 无 keyword 支持，raw_filter 不支持绑定参数）。
    let refs_suffix = build_refs_select_suffix::<ContactsEntity>();
    let mut builder: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(format!(
        "SELECT e.id, e.notice AS notice, e.code AS code, e.comments AS comments {} FROM {} AS e WHERE e.deleted_at IS NULL AND e.notice IS NOT NULL AND e.notice != ''",
        refs_suffix,
        ContactsEntity::table_name()
    ));
    push_keyword_filter(&mut builder, &query.q);
    builder.push(" ORDER BY e.id LIMIT ");
    builder.push_bind(page_size);
    builder.push(" OFFSET ");
    builder.push_bind(offset);
    let rows: Vec<ContactRow> = builder
        .build_query_as()
        .fetch_all(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?;

    // 同条件计数（total 与过滤结果一致）
    let mut counter: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(format!(
        "SELECT COUNT(*) FROM {} AS e WHERE e.deleted_at IS NULL AND e.notice IS NOT NULL AND e.notice != ''",
        ContactsEntity::table_name()
    ));
    push_keyword_filter(&mut counter, &query.q);
    let (total,): (i64,) = counter
        .build_query_as()
        .fetch_one(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?;

    let items: Vec<ContactDto> = rows.into_iter().map(contact_row_to_dto).collect();

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "items": items,
            "total": total,
            "page": page,
            "page_size": page_size,
        }))),
    )
}

/// POST /contacts — 创建联系人
pub async fn create_contact(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<CreateContactInput>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "contacts", 0, "create").await?;

    let req_inner = CreateContactRequest {
        name: body.name.clone(),
        code: body.code.clone(),
        comments: body.comments.clone(),
        infos: body.infos.clone(),
    };

    let contact = ContactsService::create_contact(pool.get_ref(), req_inner)
        .await
        .map_err(map_service_err)?;

    let dto = fetch_contact_dto(pool.get_ref(), contact.id)
        .await?
        .ok_or_else(|| ApiError::Internal("created contact not found".into()))?;

    Ok(HttpResponse::Created().json(ApiResponse::success(dto)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateContactInput {
    name: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    comments: Option<String>,
    #[serde(default)]
    infos: Vec<ContactInfoInput>,
}

/// GET /contacts/{id}
pub async fn get_contact(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "contacts", id, "read").await?;

    match fetch_contact_dto(pool.get_ref(), id).await? {
        Some(dto) => Ok(HttpResponse::Ok().json(ApiResponse::success(dto))),
        None => Err(ApiError::NotFound("Contact not found".into())),
    }
}

/// 从 `_refs` JSONB 解析联系方式数组（与 Framework service.rs entity_to_info 同逻辑）
fn parse_contact_infos_from_refs(_refs: &Option<serde_json::Value>) -> Vec<ContactInfoValue> {
    let mut infos = Vec::new();
    let Some(refs) = _refs else {
        return infos;
    };
    let kind_keys = ["email", "phone", "im", "isahl", "postal", "zipcode"];
    for kind in kind_keys {
        if let Some(arr) = refs.get(kind).and_then(|v| v.as_array()) {
            for item in arr {
                let value = item.get("notice").and_then(|v| v.as_str()).unwrap_or("");
                let is_default = item
                    .get("is_default")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                infos.push(ContactInfoValue {
                    kind: kind.to_string(),
                    value: value.to_string(),
                    is_default,
                });
            }
        }
    }
    infos
}

/// PUT /contacts/{id}
pub async fn update_contact(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<UpdateContactInput>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "contacts", id, "update").await?;

    let req_inner = UpdateContactRequest {
        name: body.name.clone(),
        code: body.code.clone(),
        comments: body.comments.clone(),
        infos: body.infos.clone(),
    };

    let contact = ContactsService::update_contact(pool.get_ref(), id, req_inner, user_id)
        .await
        .map_err(map_service_err)?;

    if contact.is_none() {
        return Err(ApiError::NotFound("Contact not found".into()));
    }

    let dto = fetch_contact_dto(pool.get_ref(), id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Contact not found".into()))?;

    Ok(HttpResponse::Ok().json(ApiResponse::success(dto)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateContactInput {
    name: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    comments: Option<String>,
    #[serde(default)]
    infos: Vec<ContactInfoInput>,
}

/// DELETE /contacts/{id}
pub async fn delete_contact(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "contacts", id, "delete").await?;

    let deleted = ContactsService::delete_contact(pool.get_ref(), id, user_id)
        .await
        .map_err(map_service_err)?;

    if !deleted {
        return Err(ApiError::NotFound("Contact not found".into()));
    }
    Ok(HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({ "deleted": true }))))
}

/// 注册通讯录路由
pub fn register(cfg: &mut actix_web::web::ServiceConfig) {
    cfg.service(
        web::resource("/contacts")
            .route(web::get().to(list_contacts))
            .route(web::post().to(create_contact)),
    )
    .service(
        web::resource("/contacts/{id}")
            .route(web::get().to(get_contact))
            .route(web::put().to(update_contact))
            .route(web::delete().to(delete_contact)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contact_row_to_dto_maps_real_code_comments() {
        // 纯逻辑：验证行映射（code/comments 取行值，不再硬编码空串）
        let row = ContactRow {
            id: 1,
            name: Some("张三".into()),
            code: Some("CT-001".into()),
            comments: Some("核心客户".into()),
            _refs: Some(serde_json::json!({
                "email": [{ "notice": "zhang@example.com", "is_default": true }],
            })),
        };
        let dto = contact_row_to_dto(row);
        assert_eq!(dto.id, 1);
        assert_eq!(dto.name, "张三");
        assert_eq!(dto.code, "CT-001");
        assert_eq!(dto.comments, "核心客户");
        assert_eq!(dto.infos.len(), 1);
        assert_eq!(dto.infos[0].kind, "email");
        assert_eq!(dto.infos[0].value, "zhang@example.com");
        assert!(dto.infos[0].is_default);
    }

    #[test]
    fn test_contact_row_to_dto_without_infos_and_nullable() {
        let row = ContactRow {
            id: 2,
            name: Some("李四".into()),
            code: None,
            comments: None,
            _refs: None,
        };
        let dto = contact_row_to_dto(row);
        assert_eq!(dto.id, 2);
        assert_eq!(dto.name, "李四");
        assert_eq!(dto.code, "");
        assert_eq!(dto.comments, "");
        assert!(dto.infos.is_empty());
    }

    #[test]
    fn test_parse_contact_infos_from_refs() {
        let refs = serde_json::json!({
            "email": [{ "notice": "a@b.com", "is_default": true }],
            "phone": [{ "notice": "13800000000", "is_default": false }],
        });
        let infos = parse_contact_infos_from_refs(&Some(refs));
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].kind, "email");
        assert_eq!(infos[0].value, "a@b.com");
        assert!(infos[0].is_default);
        assert_eq!(infos[1].kind, "phone");
        assert!(!infos[1].is_default);
    }

    #[test]
    fn test_parse_contact_infos_empty() {
        let infos = parse_contact_infos_from_refs(&None);
        assert!(infos.is_empty());
    }
}
