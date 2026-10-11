//! 状态 Handler — 值域族 typed 维护面（status 共享内核 + 前端契约投影 + RLS/NGAC + 留痕）
//!
//! 端点（壳持 `/service/status` scope，本模块持 `/statuses` 子 scope）：
//! - `GET /statuses`         — 列表（RLS 读径：X-Visible-Ids / x-authorized-columns）
//! - `POST /statuses`        — 新增（NGAC `create` + 创建行锚点注册 + 留痕）
//! - `GET /statuses/{id}`    — 详情
//! - `PUT /statuses/{id}`    — 更新（NGAC `update` + 前后值留痕）
//! - `DELETE /statuses/{id}` — 软删（NGAC `delete` + 被删行摘要留痕）
//!
//! Alioth 前端契约字段：name（notice AS name 别名）/flag/comments。
//! 实现上举自 ns 局部 handler（add-value-domain-office :: value-domain-family-coverage）；
//! 写径鉴权/留痕口径与 dict crate 同构（office-audit-trace）。

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use common::audit::{record_audit_event_with_metadata, Decision};
use common::context::{extract_user_email, extract_username};
use common::data::{ApiResponse, ListQuery, PaginatedResponse};
use common::error::AliothError;
use common::permissions::require_resource_access;
use crud::entity::AliothDbEntity;
use crud::handler::{
    extract_user_id, parse_authorized_columns, parse_visible_ids, register_created_resource_ngac,
    resolve_dk_ctx,
};
use crud::AliothRepository;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

use crate::models::{CreateStatusRequest, Status, UpdateStatusRequest};
use crate::repository::StatusRepository;

/// NGAC 资源类型（= `Status::ENTITY_NAME`：鉴权检查与创建行锚点注册同源）。
const NGAC_RESOURCE: &str = <Status as AliothDbEntity>::ENTITY_NAME;

/// 值域物理表名（留痕 metadata / object_path 用裸表名，与 dict 留痕同口径）。
const STATUS_TABLE: &str = "zc_id_stus-project";

/// Alioth 状态响应形状（name = notice 别名）
#[derive(Debug, Clone, Serialize)]
pub struct StatusResp {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: Option<String>,
    pub flag: Option<String>,
    pub comments: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Alioth 状态创建请求（前端契约）
#[derive(Debug, Clone, Deserialize)]
pub struct CreateStatusReq {
    pub name: Option<String>,
    pub flag: Option<String>,
    pub comments: Option<String>,
}

/// Alioth 状态更新请求（前端契约）
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateStatusReq {
    pub name: Option<String>,
    pub flag: Option<String>,
    pub comments: Option<String>,
}

fn to_resp(s: &Status) -> StatusResp {
    StatusResp {
        id: s.id,
        name: s.notice.clone(),
        flag: s.flag.clone(),
        comments: s.comments.clone(),
        created_at: s.created_at,
        updated_at: s.updated_at,
        deleted_at: s.deleted_at,
    }
}

/// 留痕值摘要（StatusResp 的 JSON 镜像；zuid 以字符串入 metadata，与列表出口同口径）。
fn resp_value(s: &Status) -> Value {
    serde_json::to_value(to_resp(s)).unwrap_or(Value::Null)
}

fn to_create(req: CreateStatusReq) -> CreateStatusRequest {
    CreateStatusRequest {
        notice: req.name,
        code: None,
        flag: req.flag,
        enable: None,
        comments: req.comments,
    }
}

fn to_update(req: UpdateStatusReq) -> UpdateStatusRequest {
    UpdateStatusRequest {
        notice: req.name,
        code: None,
        flag: req.flag,
        enable: None,
        comments: req.comments,
    }
}

pub async fn list_statuses(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, AliothError> {
    let repo = StatusRepository::from(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    let page = repo
        .list_with_rls(
            &query,
            visible_ids.as_deref(),
            authorized_columns.as_deref(),
        )
        .await?;
    let items: Vec<StatusResp> = page.items.iter().map(to_resp).collect();
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(PaginatedResponse {
            page: page.page,
            page_size: page.page_size,
            total: page.total,
            items,
        })),
    )
}

pub async fn get_status(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let repo = StatusRepository::from(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    let s = repo
        .get_with_rls(
            path.into_inner(),
            visible_ids.as_deref(),
            authorized_columns.as_deref(),
        )
        .await?
        .ok_or_else(|| AliothError::NotFound("status".into()))?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(to_resp(&s))))
}

pub async fn create_status(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateStatusReq>,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    // 写径 NGAC（与 dict crate 同构）：资源级 create 检查 → 写 → 行级锚点注册
    require_resource_access(pool.get_ref(), user_id, NGAC_RESOURCE, 0, "create").await?;
    let repo = StatusRepository::from(pool.get_ref().clone());
    let dk_ctx = resolve_dk_ctx::<Status>(pool.get_ref(), &req).await;
    let created = repo
        .create_with_rls(to_create(body.into_inner()), user_id, dk_ctx.as_ref())
        .await?;
    // NGAC 资源注册（与 crud_routes 行为等价，NGAC_SPEC §2.2）
    register_created_resource_ngac::<Status>(pool.get_ref(), created.id, user_id).await;
    // 留痕（office-audit-trace）：create 记新值摘要（仅成功写入后落）
    let after = resp_value(&created);
    audit_status_write(
        pool.get_ref(),
        user_id,
        &req,
        created.id,
        "status.create",
        None,
        Some(&after),
    )
    .await;
    Ok(HttpResponse::Created().json(ApiResponse::success(to_resp(&created))))
}

pub async fn update_status(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    req: HttpRequest,
    body: web::Json<UpdateStatusReq>,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, NGAC_RESOURCE, id, "update").await?;
    let repo = StatusRepository::from(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    // 前值镜像（留痕 before + RLS 存在性判定）：行不可见/不存在 ⇒ 404，不落成功事件
    let Some(before) = repo.get_with_rls(id, visible_ids.as_deref(), None).await? else {
        return Err(AliothError::NotFound("status".into()));
    };
    let dk_ctx = resolve_dk_ctx::<Status>(pool.get_ref(), &req).await;
    let updated = repo
        .update_with_rls(id, to_update(body.into_inner()), user_id, dk_ctx.as_ref())
        .await?;
    match updated {
        Some(s) => {
            // 留痕：update 记前后值摘要
            let before_v = resp_value(&before);
            let after_v = resp_value(&s);
            audit_status_write(
                pool.get_ref(),
                user_id,
                &req,
                id,
                "status.update",
                Some(&before_v),
                Some(&after_v),
            )
            .await;
            Ok(HttpResponse::Ok().json(ApiResponse::success(to_resp(&s))))
        }
        None => Err(AliothError::NotFound("status".into())),
    }
}

pub async fn delete_status(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    req: HttpRequest,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, NGAC_RESOURCE, id, "delete").await?;
    let repo = StatusRepository::from(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    // 被删行摘要（留痕 before）——行不存在时 delete 本身亦 404，审计不落
    let Some(before) = repo.get_with_rls(id, visible_ids.as_deref(), None).await? else {
        return Err(AliothError::NotFound("status".into()));
    };
    let dk_ctx = resolve_dk_ctx::<Status>(pool.get_ref(), &req).await;
    repo.delete_with_rls(id, user_id, dk_ctx.as_ref()).await?;
    let before_v = resp_value(&before);
    audit_status_write(
        pool.get_ref(),
        user_id,
        &req,
        id,
        "status.delete",
        Some(&before_v),
        None,
    )
    .await;
    Ok(HttpResponse::Ok().json(ApiResponse::<()>::success(())))
}

/// 值域写入留痕（`value-domain-office :: office-audit-trace`）：
/// 经 `common::audit::record_audit_event_with_metadata` 落 `isahl_audit.audit_events`，
/// 事件含 actor（user_id/username/email）、表名、行 id、前后值摘要。
///
/// 仅在写入**成功后**调用（被拒/未命中路径在调用点之前 return ⇒ MUST NOT 落成功事件）；
/// 非阻断语义（与 record_audit_event_* 一致）：留痕失败仅 warn，不回滚业务写入。
async fn audit_status_write(
    pool: &PgPool,
    user_id: i64,
    req: &HttpRequest,
    row_id: i64,
    operation: &str,
    before: Option<&Value>,
    after: Option<&Value>,
) {
    let username = extract_username(req).unwrap_or_default();
    let subject = common::audit::resolve_subject(&username, user_id);
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "actor".to_string(),
        serde_json::json!({
            "user_id": user_id.to_string(),
            "username": username,
            "email": extract_user_email(req),
        }),
    );
    metadata.insert("table".to_string(), serde_json::json!(STATUS_TABLE));
    // zuid（> 2^53）以字符串入 metadata，避免 JSON number 精度丢失（与列表出口同口径）
    metadata.insert("id".to_string(), serde_json::json!(row_id.to_string()));
    if let Some(b) = before {
        metadata.insert("before".to_string(), b.clone());
    }
    if let Some(a) = after {
        metadata.insert("after".to_string(), a.clone());
    }
    if let Err(e) = record_audit_event_with_metadata(
        pool,
        user_id,
        &subject,
        &format!("status:{}:{}", STATUS_TABLE, row_id),
        operation,
        &Decision::Permit,
        Value::Object(metadata),
    )
    .await
    {
        common::telemetry::warn!(
            "status 写入留痕失败（{} {} {}）：{}",
            operation,
            STATUS_TABLE,
            row_id,
            e
        );
    }
}

/// 注册 status typed 维护面（scope 语义 = `/statuses`；壳以 `/service/status` 包裹）。
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/statuses")
            .route("", web::get().to(list_statuses))
            .route("", web::post().to(create_status))
            .route("/{id}", web::get().to(get_status))
            .route("/{id}", web::put().to(update_status))
            .route("/{id}", web::delete().to(delete_status)),
    );
}
