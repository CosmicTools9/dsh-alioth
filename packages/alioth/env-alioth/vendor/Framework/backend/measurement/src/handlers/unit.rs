//! 计量单位 Handler — 统一 CRUD（biz repository + multiplier 组装 + RLS/NGAC + 补色）
//!
//! 安全模式：list/get 走 RLS 读（visible_ids + authorized_columns），
//! create/update/delete 走 extract_user_id + NGAC resource access + dk_ctx + with_rls 写。
//! units list/get/detail 响应统一含 multiplier（相对同量纲 base 单位的换算率，
//! base 无换算率回退 1）与 base 字段——全局契约，ns 前端忽略未消费字段。

use crate::biz_models::{
    CreateMeasurementUnitRequest, MeasurementUnit, UpdateMeasurementUnitRequest,
};
use crate::biz_repositories::MeasurementUnitRepository;
use crate::service::MeasurementService;
use actix_web::{web, HttpRequest, HttpResponse};
use common::data::{ApiResponse, ListQuery};
use common::error::AliothError;
use crud::entity::AliothDbEntity;
use crud::handler::{
    extract_user_id, parse_authorized_columns, parse_visible_ids, register_created_resource_ngac,
    resolve_dk_ctx,
};
use crud::AliothRepository;
use sqlx::PgPool;

/// 量纲→主题色。与前端 DIM_COLORS 保持一致（创建时不传 t_color_ 时后端默认着色）。
pub fn dimension_color(dim_key: &str) -> &'static str {
    match dim_key {
        "length" => "#5856d6",
        "mass" => "#0071e3",
        "time" => "#34c759",
        "current" => "#ff9500",
        "temperature" => "#ff3b30",
        "intensity" => "#ff6482",
        "force" => "#00b8a9",
        "area" => "#5e5ce6",
        "volume" => "#30b0c7",
        "pressure" => "#ff2d55",
        "energy" => "#bf5af2",
        "power" => "#ff9f0a",
        "speed" => "#64d2ff",
        "frequency" => "#ffd60a",
        "data" => "#a78bfa",
        "angle" => "#fb923c",
        "density" => "#ff6b6b",
        "substance_amount" => "#af52de",
        "luminance" => "#ff748c",
        "magnetic_flux" => "#00c9db",
        "magnetic_field_strength" => "#7c3aed",
        "stress" => "#e11d48",
        "display" => "#6366f1",
        "pricing" => "#f59e0b",
        "price" => "#10b981",
        "distance" => "#3b82f6",
        "duration" => "#8b5cf6",
        "weight" => "#84cc16",
        "currency" => "#06b6d4",
        "container" => "#ec4899",
        "common" => "#9ca3af",
        "working" => "#f97316",
        "voltage" => "#ef4444",
        "radiation" => "#14b8a6",
        _ => "#9ca3af",
    }
}

async fn list_units(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, AliothError> {
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    let svc = MeasurementService::new(pool.get_ref().clone());
    let resp = svc
        .list_units(
            &query,
            visible_ids.as_deref(),
            authorized_columns.as_deref(),
        )
        .await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(resp)))
}

async fn get_unit(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let repo = MeasurementUnitRepository::from(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    let unit = repo
        .get_with_rls(
            path.into_inner(),
            visible_ids.as_deref(),
            authorized_columns.as_deref(),
        )
        .await?
        .ok_or_else(|| AliothError::NotFound("measurement_unit".into()))?;
    let svc = MeasurementService::new(pool.get_ref().clone());
    Ok(HttpResponse::Ok().json(ApiResponse::success(svc.to_list_item(unit).await?)))
}

async fn get_detail(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let repo = MeasurementUnitRepository::from(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    let unit = repo
        .get_with_rls(
            path.into_inner(),
            visible_ids.as_deref(),
            authorized_columns.as_deref(),
        )
        .await?
        .ok_or_else(|| AliothError::NotFound("measurement_unit".into()))?;
    let svc = MeasurementService::new(pool.get_ref().clone());
    Ok(HttpResponse::Ok().json(ApiResponse::success(svc.to_unit_detail(unit).await?)))
}

async fn create_unit(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    mut body: web::Json<CreateMeasurementUnitRequest>,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    // 展示色补填（前端不传 t_color_ 时后端默认着色）
    if body.t_color_.is_none() {
        let dim = body.dimension_key.as_deref().unwrap_or("");
        body.t_color_ = Some(dimension_color(dim).to_string());
    }
    let repo = MeasurementUnitRepository::from(pool.get_ref().clone());
    let dk_ctx = resolve_dk_ctx::<MeasurementUnit>(pool.get_ref(), &req).await;
    let created = repo
        .create_with_rls(body.into_inner(), user_id, dk_ctx.as_ref())
        .await?;
    // NGAC 资源注册（与 crud_routes 行为等价，NGAC_SPEC §2.2）
    register_created_resource_ngac::<MeasurementUnit>(pool.get_ref(), created.id, user_id).await;
    let svc = MeasurementService::new(pool.get_ref().clone());
    Ok(HttpResponse::Created().json(ApiResponse::success(svc.to_list_item(created).await?)))
}

async fn update_unit(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    req: HttpRequest,
    body: web::Json<UpdateMeasurementUnitRequest>,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let id = path.into_inner();
    common::permissions::require_resource_access(
        pool.get_ref(),
        user_id,
        MeasurementUnit::ENTITY_NAME,
        id,
        "update",
    )
    .await?;
    let repo = MeasurementUnitRepository::from(pool.get_ref().clone());
    // 行级可见性预检（NGAC_SPEC visible_ids）：不可见行 -> NotFound
    if let Some(visible_ids) = parse_visible_ids(&req) {
        let existing = repo.get_with_rls(id, Some(&visible_ids), None).await?;
        if existing.is_none() {
            return Err(AliothError::NotFound("measurement_unit".into()));
        }
    }
    let dk_ctx = resolve_dk_ctx::<MeasurementUnit>(pool.get_ref(), &req).await;
    let updated = repo
        .update_with_rls(id, body.into_inner(), user_id, dk_ctx.as_ref())
        .await?;
    match updated {
        Some(u) => {
            let svc = MeasurementService::new(pool.get_ref().clone());
            Ok(HttpResponse::Ok().json(ApiResponse::success(svc.to_list_item(u).await?)))
        }
        None => Err(AliothError::NotFound("measurement_unit".into())),
    }
}

async fn delete_unit(
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
        MeasurementUnit::ENTITY_NAME,
        id,
        "delete",
    )
    .await?;
    let _ = sqlx::query(
        "UPDATE isahl_auth.ngac_object_attribute SET deleted_at = NOW(), deleted_by_id = $1 \
         WHERE resource_type = $2 AND fk_resource = $3 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(MeasurementUnit::ENTITY_NAME)
    .bind(id)
    .execute(pool.get_ref())
    .await;
    let repo = MeasurementUnitRepository::from(pool.get_ref().clone());
    // 行级可见性预检（NGAC_SPEC visible_ids）：不可见行 -> NotFound
    if let Some(visible_ids) = parse_visible_ids(&req) {
        let existing = repo.get_with_rls(id, Some(&visible_ids), None).await?;
        if existing.is_none() {
            return Err(AliothError::NotFound("measurement_unit".into()));
        }
    }
    let dk_ctx = resolve_dk_ctx::<MeasurementUnit>(pool.get_ref(), &req).await;
    repo.delete_with_rls(id, user_id, dk_ctx.as_ref()).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::<()>::success(())))
}

/// 查询数据库中的量纲叶表列表作为可用维度
async fn get_dimensions(pool: web::Data<PgPool>) -> Result<HttpResponse, AliothError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"SELECT table_name::text FROM information_schema.tables
           WHERE table_schema = 'isahl'
             AND table_name LIKE 'zc_id_unit-%'
           ORDER BY table_name"#,
    )
    .fetch_all(pool.get_ref())
    .await
    .map_err(|e| AliothError::Database(e.to_string()))?;

    let dimensions: Vec<String> = rows
        .into_iter()
        .map(|(t,)| t.strip_prefix("zc_id_unit-").unwrap_or(&t).to_string())
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(dimensions)))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/units")
            .route("", web::get().to(list_units))
            .route("", web::post().to(create_unit))
            .route("/dimensions", web::get().to(get_dimensions))
            .route("/{id}", web::get().to(get_unit))
            .route("/{id}", web::put().to(update_unit))
            .route("/{id}", web::delete().to(delete_unit))
            .route("/{id}/detail", web::get().to(get_detail)),
    );
}
