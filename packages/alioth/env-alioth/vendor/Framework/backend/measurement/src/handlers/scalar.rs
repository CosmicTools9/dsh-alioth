//! 标量价格 Handler — 统一 CRUD（biz repository + RLS/NGAC + 领域事件发布）
//!
//! 创建标量值后发布 `measurement.scalar.created` 领域事件（非关键路径，总线缺失/失败忽略）。

use std::sync::Arc;

use crate::biz_models::{CreateScalarPriceRequest, ScalarPrice, UpdateScalarPriceRequest};
use crate::biz_repositories::ScalarPriceRepository;
use actix_web::{web, HttpRequest, HttpResponse};
use common::data::{ApiResponse, ListQuery};
use common::error::AliothError;
use common::event_bus::{DomainEvent, DomainEventBus};
use crud::entity::AliothDbEntity;
use crud::handler::{
    extract_user_id, parse_authorized_columns, parse_visible_ids, register_created_resource_ngac,
    resolve_dk_ctx,
};
use crud::AliothRepository;
use sqlx::PgPool;

async fn list_scalars(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, AliothError> {
    let repo = ScalarPriceRepository::from(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    let page = repo
        .list_with_rls(
            &query,
            visible_ids.as_deref(),
            authorized_columns.as_deref(),
        )
        .await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(page)))
}

async fn get_scalar(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let repo = ScalarPriceRepository::from(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    let item = repo
        .get_with_rls(
            path.into_inner(),
            visible_ids.as_deref(),
            authorized_columns.as_deref(),
        )
        .await?
        .ok_or_else(|| AliothError::NotFound("scalar_price".into()))?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(item)))
}

/// 创建标量值并发布领域事件（measurement.scalar.created）
async fn create_scalar(
    pool: web::Data<PgPool>,
    event_bus: web::Data<Option<Arc<dyn DomainEventBus>>>,
    req: HttpRequest,
    body: web::Json<CreateScalarPriceRequest>,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let repo = ScalarPriceRepository::from(pool.get_ref().clone());
    let dk_ctx = resolve_dk_ctx::<ScalarPrice>(pool.get_ref(), &req).await;
    let created = repo
        .create_with_rls(body.into_inner(), user_id, dk_ctx.as_ref())
        .await?;
    // NGAC 资源注册（与 crud_routes 行为等价，NGAC_SPEC §2.2）
    register_created_resource_ngac::<ScalarPrice>(pool.get_ref(), created.id, user_id).await;

    // 发布标量创建事件（非关键路径，总线缺失/错误忽略）
    if let Some(bus) = event_bus.as_ref().as_ref() {
        let event = DomainEvent::new(
            "measurement.scalar.created",
            "measurement",
            created.id,
            serde_json::json!({
                "id": created.id,
                "name": created.name,
                "value": created.value,
                "unit": created.unit,
            }),
        );
        if let Ok(event) = event {
            if let Err(e) = bus.publish("measurement.scalar.created", &event).await {
                eprintln!("Failed to publish scalar.created event: {:?}", e);
            }
        }
    }

    Ok(HttpResponse::Created().json(ApiResponse::success(created)))
}

async fn update_scalar(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    req: HttpRequest,
    body: web::Json<UpdateScalarPriceRequest>,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let id = path.into_inner();
    common::permissions::require_resource_access(
        pool.get_ref(),
        user_id,
        ScalarPrice::ENTITY_NAME,
        id,
        "update",
    )
    .await?;
    let repo = ScalarPriceRepository::from(pool.get_ref().clone());
    // 行级可见性预检（NGAC_SPEC visible_ids）：不可见行 -> NotFound
    if let Some(visible_ids) = parse_visible_ids(&req) {
        let existing = repo.get_with_rls(id, Some(&visible_ids), None).await?;
        if existing.is_none() {
            return Err(AliothError::NotFound("scalar_price".into()));
        }
    }
    let dk_ctx = resolve_dk_ctx::<ScalarPrice>(pool.get_ref(), &req).await;
    let updated = repo
        .update_with_rls(id, body.into_inner(), user_id, dk_ctx.as_ref())
        .await?;
    match updated {
        Some(s) => Ok(HttpResponse::Ok().json(ApiResponse::success(s))),
        None => Err(AliothError::NotFound("scalar_price".into())),
    }
}

async fn delete_scalar(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    req: HttpRequest,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let id = path.into_inner();
    common::permissions::require_resource_access(
        pool.get_ref(),
        user_id,
        ScalarPrice::ENTITY_NAME,
        id,
        "delete",
    )
    .await?;
    let _ = sqlx::query(
        "UPDATE isahl_auth.ngac_object_attribute SET deleted_at = NOW(), deleted_by_id = $1 \
         WHERE resource_type = $2 AND fk_resource = $3 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(ScalarPrice::ENTITY_NAME)
    .bind(id)
    .execute(pool.get_ref())
    .await;
    let repo = ScalarPriceRepository::from(pool.get_ref().clone());
    // 行级可见性预检（NGAC_SPEC visible_ids）：不可见行 -> NotFound
    if let Some(visible_ids) = parse_visible_ids(&req) {
        let existing = repo.get_with_rls(id, Some(&visible_ids), None).await?;
        if existing.is_none() {
            return Err(AliothError::NotFound("scalar_price".into()));
        }
    }
    let dk_ctx = resolve_dk_ctx::<ScalarPrice>(pool.get_ref(), &req).await;
    repo.delete_with_rls(id, user_id, dk_ctx.as_ref()).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::<()>::success(())))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/scalars")
            .route("", web::get().to(list_scalars))
            .route("", web::post().to(create_scalar))
            .route("/{id}", web::get().to(get_scalar))
            .route("/{id}", web::put().to(update_scalar))
            .route("/{id}", web::delete().to(delete_scalar)),
    );
}
