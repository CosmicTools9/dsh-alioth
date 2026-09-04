//! 库存统计 HTTP Handler + 路由注册
//!
//! - `GET /service/inventory/statistics` — 物化库存统计查询（按 产品/储元 过滤）
//! - `GET /service/inventory/statistics/{id}` — 单条统计
//! - `crud_routes` 注册 Voucher / Counting / StorageNest 标准 CRUD

use actix_web::web;
use common::error::AliothError as ApiError;
use crud::crud_routes;
use sqlx::PgPool;

use crate::models::{
    Counting, CountingDetail, CreateCountingDetailRequest, CreateCountingRequest,
    CreateStorageNestRequest, CreateVoucherRequest, StockStat, StorageNest,
    UpdateCountingDetailRequest, UpdateCountingRequest, UpdateStorageNestRequest,
    UpdateVoucherRequest, Voucher,
};
use crate::repositories::counting_detail_repository::CountingDetailRepository;
use crate::repositories::counting_repository::CountingRepository;
use crate::repositories::storage_nest_repository::StorageNestRepository;
use crate::repositories::voucher_repository::VoucherRepository;
use crate::services::{StockStatQuery, StockStatService};

/// GET /statistics?production_id=&storage_id=
async fn list_statistics(
    pool: web::Data<PgPool>,
    query: web::Query<StockStatQueryParam>,
) -> Result<web::Json<Vec<StockStat>>, ApiError> {
    let svc = StockStatService::new(pool.get_ref().clone());
    let q = StockStatQuery {
        production_id: query.production_id,
        storage_id: query.storage_id,
    };
    Ok(web::Json(svc.statistics(&q).await?))
}

/// GET /statistics/{id}
async fn get_statistic(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<web::Json<StockStat>, ApiError> {
    let svc = StockStatService::new(pool.get_ref().clone());
    let id = path.into_inner();
    svc.get_stat(id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("stock_stat {} not found", id)))
        .map(web::Json)
}

/// 查询参数（Option<i64> 可直接反序列化）
#[derive(Debug, serde::Deserialize)]
pub struct StockStatQueryParam {
    pub production_id: Option<i64>,
    pub storage_id: Option<i64>,
}

/// 注册库存统计相关全部路由
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/statistics")
            .route("", web::get().to(list_statistics))
            .route("/{id}", web::get().to(get_statistic)),
    );
    cfg.configure(crud_routes::<
        Voucher,
        CreateVoucherRequest,
        UpdateVoucherRequest,
        VoucherRepository,
        ApiError,
    >("/vouchers"));
    cfg.configure(crud_routes::<
        Counting,
        CreateCountingRequest,
        UpdateCountingRequest,
        CountingRepository,
        ApiError,
    >("/countings"));
    cfg.configure(crud_routes::<
        CountingDetail,
        CreateCountingDetailRequest,
        UpdateCountingDetailRequest,
        CountingDetailRepository,
        ApiError,
    >("/counting-details"));
    cfg.configure(crud_routes::<
        StorageNest,
        CreateStorageNestRequest,
        UpdateStorageNestRequest,
        StorageNestRepository,
        ApiError,
    >("/storage-nests"));
}
