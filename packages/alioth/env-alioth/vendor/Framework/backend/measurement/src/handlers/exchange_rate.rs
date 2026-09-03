//! 汇率 Handler — 统一 CRUD（biz repository + 字符串货币代码解析 + RLS/NGAC）
//!
//! 输入统一支持 left/right（i64 引用）或 left_currency/right_currency（字符串货币代码，
//! 解析 zc_id_unit-currency 叶表）；响应统一 biz 实体直出（超集，ns 前端忽略未消费字段）。

use crate::biz_models::{CreateExchangeRateRequest, ExchangeRate, UpdateExchangeRateRequest};
use crate::biz_repositories::ExchangeRateRepository;
use actix_web::{web, HttpRequest, HttpResponse};
use common::data::{ApiResponse, ListQuery};
use common::error::AliothError;
use crud::entity::AliothDbEntity;
use crud::handler::{
    extract_user_id, parse_authorized_columns, parse_visible_ids, register_created_resource_ngac,
    resolve_dk_ctx,
};
use crud::AliothRepository;
use serde::Deserialize;
use sqlx::PgPool;

/// 汇率请求形状（前端契约：left/right i64 或 left_currency/right_currency 字符串代码）
#[derive(Debug, Deserialize)]
pub struct ExchangeRateCreateRequest {
    pub name: Option<String>,
    pub left: Option<i64>,
    pub right: Option<i64>,
    pub left_currency: Option<String>,
    pub right_currency: Option<String>,
    pub bid_price: Option<rust_decimal::Decimal>,
    pub ask_price: Option<rust_decimal::Decimal>,
    pub rate: Option<rust_decimal::Decimal>,
    pub source: Option<String>,
    pub updated: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct ExchangeRateUpdateRequest {
    pub name: Option<String>,
    pub left: Option<i64>,
    pub right: Option<i64>,
    pub left_currency: Option<String>,
    pub right_currency: Option<String>,
    pub bid_price: Option<rust_decimal::Decimal>,
    pub ask_price: Option<rust_decimal::Decimal>,
    pub rate: Option<rust_decimal::Decimal>,
    pub source: Option<String>,
    pub updated: Option<chrono::DateTime<chrono::Utc>>,
}

/// 货币代码（字符串）→ 货币单位 id（zc_id_unit-currency 叶表）
async fn resolve_currency_id(pool: &PgPool, code: &str) -> Result<i64, AliothError> {
    let id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_unit-currency" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(code)
    .fetch_optional(pool)
    .await
    .map_err(AliothError::from)?;
    id.ok_or_else(|| AliothError::NotFound(format!("currency code {} not found", code)))
}

async fn list_rates(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, AliothError> {
    let repo = ExchangeRateRepository::from(pool.get_ref().clone());
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

async fn get_rate(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let repo = ExchangeRateRepository::from(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    let rate = repo
        .get_with_rls(
            path.into_inner(),
            visible_ids.as_deref(),
            authorized_columns.as_deref(),
        )
        .await?
        .ok_or_else(|| AliothError::NotFound("exchange_rate".into()))?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(rate)))
}

fn to_biz_create(
    body: ExchangeRateCreateRequest,
    left_id: Option<i64>,
    right_id: Option<i64>,
) -> CreateExchangeRateRequest {
    CreateExchangeRateRequest {
        name: body.name.unwrap_or_default(),
        left_currency: left_id.or(body.left),
        right_currency: right_id.or(body.right),
        rate: body.rate.or(body.bid_price),
        ask_price: body.ask_price,
        source: body.source,
        date: body.updated,
    }
}

async fn create_rate(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<ExchangeRateCreateRequest>,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let body = body.into_inner();
    let left_id = match body.left_currency.as_deref() {
        Some(code) => Some(resolve_currency_id(pool.get_ref(), code).await?),
        None => None,
    };
    let right_id = match body.right_currency.as_deref() {
        Some(code) => Some(resolve_currency_id(pool.get_ref(), code).await?),
        None => None,
    };
    let repo = ExchangeRateRepository::from(pool.get_ref().clone());
    let dk_ctx = resolve_dk_ctx::<ExchangeRate>(pool.get_ref(), &req).await;
    let created = repo
        .create_with_rls(
            to_biz_create(body, left_id, right_id),
            user_id,
            dk_ctx.as_ref(),
        )
        .await?;
    // NGAC 资源注册（与 crud_routes 行为等价，NGAC_SPEC §2.2）
    register_created_resource_ngac::<ExchangeRate>(pool.get_ref(), created.id, user_id).await;
    Ok(HttpResponse::Created().json(ApiResponse::success(created)))
}

async fn update_rate(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    req: HttpRequest,
    body: web::Json<ExchangeRateUpdateRequest>,
) -> Result<HttpResponse, AliothError> {
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;
    let id = path.into_inner();
    common::permissions::require_resource_access(
        pool.get_ref(),
        user_id,
        ExchangeRate::ENTITY_NAME,
        id,
        "update",
    )
    .await?;
    let repo = ExchangeRateRepository::from(pool.get_ref().clone());
    // 行级可见性预检（NGAC_SPEC visible_ids）：不可见行 -> NotFound
    if let Some(visible_ids) = parse_visible_ids(&req) {
        let existing = repo.get_with_rls(id, Some(&visible_ids), None).await?;
        if existing.is_none() {
            return Err(AliothError::NotFound("exchange_rate".into()));
        }
    }
    let body = body.into_inner();
    let left_id = match body.left_currency.as_deref() {
        Some(code) => Some(resolve_currency_id(pool.get_ref(), code).await?),
        None => None,
    };
    let right_id = match body.right_currency.as_deref() {
        Some(code) => Some(resolve_currency_id(pool.get_ref(), code).await?),
        None => None,
    };
    let biz_req = UpdateExchangeRateRequest {
        name: body.name,
        left_currency: left_id.or(body.left),
        right_currency: right_id.or(body.right),
        rate: body.rate.or(body.bid_price),
        ask_price: body.ask_price,
        source: body.source,
        date: body.updated,
    };
    let dk_ctx = resolve_dk_ctx::<ExchangeRate>(pool.get_ref(), &req).await;
    let updated = repo
        .update_with_rls(id, biz_req, user_id, dk_ctx.as_ref())
        .await?;
    match updated {
        Some(r) => Ok(HttpResponse::Ok().json(ApiResponse::success(r))),
        None => Err(AliothError::NotFound("exchange_rate".into())),
    }
}

async fn delete_rate(
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
        ExchangeRate::ENTITY_NAME,
        id,
        "delete",
    )
    .await?;
    let _ = sqlx::query(
        "UPDATE isahl_auth.ngac_object_attribute SET deleted_at = NOW(), deleted_by_id = $1 \
         WHERE resource_type = $2 AND fk_resource = $3 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(ExchangeRate::ENTITY_NAME)
    .bind(id)
    .execute(pool.get_ref())
    .await;
    let repo = ExchangeRateRepository::from(pool.get_ref().clone());
    // 行级可见性预检（NGAC_SPEC visible_ids）：不可见行 -> NotFound
    if let Some(visible_ids) = parse_visible_ids(&req) {
        let existing = repo.get_with_rls(id, Some(&visible_ids), None).await?;
        if existing.is_none() {
            return Err(AliothError::NotFound("exchange_rate".into()));
        }
    }
    let dk_ctx = resolve_dk_ctx::<ExchangeRate>(pool.get_ref(), &req).await;
    repo.delete_with_rls(id, user_id, dk_ctx.as_ref()).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::<()>::success(())))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/exchange-rates")
            .route("", web::get().to(list_rates))
            .route("", web::post().to(create_rate))
            .route("/{id}", web::get().to(get_rate))
            .route("/{id}", web::put().to(update_rate))
            .route("/{id}", web::delete().to(delete_rate)),
    );
}
