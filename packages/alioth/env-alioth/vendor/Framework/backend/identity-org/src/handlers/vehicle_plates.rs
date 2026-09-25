//! 车辆号牌 Handler —— 号牌经「实体↔身份」桥承载（change `align-vehicle-plate-identity`）。
//!
//! 模型口径（用户 2026-09-23）：车辆 `notice` = 描述信息、`code` = 序列号（车架号/VIN）；
//! 号牌号经 `zc_id_entity_rr_identity` 关联（可多牌、可换牌、历史留痕），读写件 =
//! [`crate::plates`]（唯一实现）。
//!
//! 端点（挂 `/service/isahl-db`）：
//! - `GET    /vehicles/{id}/plates` —— 号牌列表（含历史行，`active` 标记；活动在前）
//! - `POST   /vehicles/{id}/plates` —— 新增号牌（入口归一 + GA 36 形态校验 +
//!   同号牌**全局**生效期重叠 → 409；不重叠 = 换牌/历史段，复用同一身份行）
//! - `DELETE /vehicles/{id}/plates/{relId}` —— 解除 / 换牌（软删桥行，历史保留）
//!
//! 鉴权：NGAC 行级（`vehicles` 资源 = `zc_id_stor-ctn-vehicle`，与车辆 CRUD 同源
//! `crud::ngac_resource_name::<Vehicle>()` = `vehicles`）。

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::plates::{self, PlateInput};

/// NGAC 资源名（与 `crud::ngac_resource_name::<Vehicle>()` 同值）
const VEHICLE_RESOURCE: &str = "vehicles";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVehiclePlateRequest {
    /// 号牌号（入口归一：去空白/分隔圆点 + 大写；形态须符合 GA 36-2018 民用号牌）
    pub plate: String,
    /// 号牌描述（缺省 `车牌-{号牌}`）
    #[serde(default)]
    pub description: Option<String>,
    /// 生效期起（空 = 无下界 `-∞`；全空段 = 覆盖全部时间）
    #[serde(default)]
    pub valid_from: Option<DateTime<Utc>>,
    /// 生效期止（空 = 无上界 `+∞`）
    #[serde(default)]
    pub valid_to: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VehiclePlateRow {
    /// 桥行 id（`zc_id_entity_rr_identity.id`；解除/换牌入参）
    #[serde(with = "common::serde_zuid")]
    pub rel_id: i64,
    /// 身份行 id
    #[serde(with = "common::serde_zuid")]
    pub identity_id: i64,
    /// 号牌号（`zc_id_identity.identity`）
    pub plate: String,
    /// 号牌描述（`zc_id_identity.notice`）
    pub description: Option<String>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
    /// 活动态（桥行/身份行未软删且未过有效期）；`false` = 换牌/解除后的历史行
    pub active: bool,
}

/// GET /service/isahl-db/vehicles/{id}/plates —— 车辆号牌列表（含历史）
pub async fn list_plates(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let vehicle_id = path.into_inner();
    require_resource_access(
        pool.get_ref(),
        user_id,
        VEHICLE_RESOURCE,
        vehicle_id,
        "read",
    )
    .await?;

    let mut conn = pool.acquire().await.map_err(ApiError::from_sqlx)?;
    plates::ensure_vehicle_exists(&mut conn, vehicle_id).await?;
    let rows = plates::list_vehicle_plates(&mut conn, vehicle_id).await?;
    let items: Vec<VehiclePlateRow> = rows
        .into_iter()
        .map(|r| VehiclePlateRow {
            rel_id: r.rel_id,
            identity_id: r.identity_id,
            plate: r.plate,
            description: r.description,
            valid_from: r.valid_from,
            valid_to: r.valid_to,
            active: r.active,
        })
        .collect();
    Ok(HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({ "items": items }))))
}

/// POST /service/isahl-db/vehicles/{id}/plates —— 新增号牌（换牌 = 本端点 + 解除旧牌）
pub async fn create_plate(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<CreateVehiclePlateRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let vehicle_id = path.into_inner();
    require_resource_access(
        pool.get_ref(),
        user_id,
        VEHICLE_RESOURCE,
        vehicle_id,
        "update",
    )
    .await?;

    let body = body.into_inner();
    let plate = common::plate::normalize_plate(&body.plate);
    if plate.is_empty() {
        return Err(ApiError::BadRequest("号牌号不能为空".into()));
    }
    if !common::plate::plate_format_ok(&plate) {
        return Err(ApiError::BadRequest(
            "号牌不符合中国民用号牌规则（如 京A12345、蒙BD22345）".into(),
        ));
    }
    if let (Some(from), Some(to)) = (body.valid_from, body.valid_to) {
        if from > to {
            return Err(ApiError::BadRequest("有效期起始不得晚于终止".into()));
        }
    }

    let mut conn = pool.acquire().await.map_err(ApiError::from_sqlx)?;
    plates::ensure_vehicle_exists(&mut conn, vehicle_id).await?;
    let (rel_id, identity_id) = plates::create_vehicle_plate(
        &mut conn,
        vehicle_id,
        user_id,
        &PlateInput {
            plate,
            description: body.description,
            valid_from: body.valid_from,
            valid_to: body.valid_to,
        },
    )
    .await?;
    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "relId": rel_id.to_string(),
            "identityId": identity_id.to_string(),
        }))),
    )
}

/// DELETE /service/isahl-db/vehicles/{id}/plates/{relId} —— 解除号牌（软删，历史保留）
pub async fn delete_plate(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (vehicle_id, rel_id) = path.into_inner();
    require_resource_access(
        pool.get_ref(),
        user_id,
        VEHICLE_RESOURCE,
        vehicle_id,
        "update",
    )
    .await?;

    let mut conn = pool.acquire().await.map_err(ApiError::from_sqlx)?;
    plates::retire_vehicle_plate(&mut conn, vehicle_id, rel_id, user_id).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(
        serde_json::json!({ "relId": rel_id.to_string() }),
    )))
}

/// 路由注册（由 isahl-db 服务壳 `register_service_routes` 调用）
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/vehicles/{id}/plates")
            .route(web::get().to(list_plates))
            .route(web::post().to(create_plate)),
    )
    .service(web::resource("/vehicles/{id}/plates/{relId}").route(web::delete().to(delete_plate)));
}

#[cfg(test)]
mod route_registration_tests {
    //! 路由可达性回归（**不触库**）：号牌子资源 MUST 先于 `/vehicles` CRUD scope 注册——
    //! actix `web::scope("/vehicles")` 对同前缀请求命中即截断，晚注册的子资源恒 404（实测）。
    //! 判据：无鉴权访问应得 **401**（资源已注册、进到 handler 的鉴权），若被截断则为 404。

    use actix_web::{test, web, App};

    #[actix_web::test]
    async fn plate_routes_survive_vehicle_crud_scope() {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(
                    sqlx::PgPool::connect_lazy("postgres://u@127.0.0.1:1/none").expect("lazy pool"),
                ))
                .configure(crate::handlers::crud::register_business_domain),
        )
        .await;

        for (method, uri) in [
            ("GET", "/vehicles/1/plates"),
            ("POST", "/vehicles/1/plates"),
            ("DELETE", "/vehicles/1/plates/2"),
        ] {
            let req = match method {
                "GET" => test::TestRequest::get().uri(uri).to_request(),
                "POST" => test::TestRequest::post().uri(uri).to_request(),
                _ => test::TestRequest::delete().uri(uri).to_request(),
            };
            let status = test::call_service(&app, req).await.status();
            assert_ne!(
                status,
                actix_web::http::StatusCode::NOT_FOUND,
                "{method} {uri} 未注册或被 /vehicles scope 截断（检查注册顺序）"
            );
        }
    }
}
