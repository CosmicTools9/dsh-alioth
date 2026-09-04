//! 库存统计 Service — 业务逻辑层
//!
//! 核心：`StockStatService.statistics()` 读物化储量（触发器维护的标量 mark）。
//! 写入口（Voucher/Counting/StorageNest）为标准 CRUD，物化由 DB 触发器完成。

use common::data::{ListQuery, PaginatedResponse};
use common::error::AliothError;
use crud::repository::AliothRepository;
use sqlx::PgPool;

use crate::models::{
    Counting, CountingDetail, CreateCountingDetailRequest, CreateCountingRequest,
    CreateStorageNestRequest, CreateVoucherRequest, StockStat, StorageNest,
    UpdateCountingDetailRequest, UpdateCountingRequest, UpdateStorageNestRequest,
    UpdateVoucherRequest, Voucher,
};
use crate::repositories::counting_detail_repository::CountingDetailRepository;
use crate::repositories::counting_repository::CountingRepository;
use crate::repositories::stock_stat_repository::StockStatRepository;
use crate::repositories::storage_nest_repository::StorageNestRepository;
use crate::repositories::voucher_repository::VoucherRepository;

/// 库存统计查询参数
#[derive(Debug, Clone, Default)]
pub struct StockStatQuery {
    pub production_id: Option<i64>,
    pub storage_id: Option<i64>,
}

/// 库存统计 Service
#[derive(Clone)]
pub struct StockStatService {
    stat_repo: StockStatRepository,
}

impl StockStatService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            stat_repo: StockStatRepository::new(pool),
        }
    }

    /// 按 (产品, 储元) 读物化库存统计（含嵌套汇总口径——物化值已含 rollup）
    pub async fn statistics(&self, query: &StockStatQuery) -> Result<Vec<StockStat>, AliothError> {
        self.stat_repo
            .statistics(query.production_id, query.storage_id)
            .await
    }

    pub async fn get_stat(&self, id: i64) -> Result<Option<StockStat>, AliothError> {
        self.stat_repo.get_stat(id).await
    }
}

/// Voucher（货 stock in/out 储元）CRUD Service
#[derive(Clone)]
pub struct VoucherService {
    repo: VoucherRepository,
}

impl VoucherService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repo: VoucherRepository::new(pool),
        }
    }

    pub async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Voucher>, AliothError> {
        self.repo.list(query).await
    }

    pub async fn get(&self, id: i64) -> Result<Option<Voucher>, AliothError> {
        self.repo.get(id).await
    }

    pub async fn create(
        &self,
        req: CreateVoucherRequest,
        user_id: i64,
    ) -> Result<Voucher, AliothError> {
        self.repo.create(req, user_id).await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateVoucherRequest,
        user_id: i64,
    ) -> Result<Option<Voucher>, AliothError> {
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.repo.delete(id, user_id).await
    }
}

/// Counting（盘点）CRUD Service
#[derive(Clone)]
pub struct CountingService {
    repo: CountingRepository,
}

impl CountingService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repo: CountingRepository::new(pool),
        }
    }

    pub async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<Counting>, AliothError> {
        self.repo.list(query).await
    }

    pub async fn get(&self, id: i64) -> Result<Option<Counting>, AliothError> {
        self.repo.get(id).await
    }

    pub async fn create(
        &self,
        req: CreateCountingRequest,
        user_id: i64,
    ) -> Result<Counting, AliothError> {
        self.repo.create(req, user_id).await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateCountingRequest,
        user_id: i64,
    ) -> Result<Option<Counting>, AliothError> {
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.repo.delete(id, user_id).await
    }
}

/// CountingDetail（盘点明细，实值校准 + 自动校准）CRUD Service
#[derive(Clone)]
pub struct CountingDetailService {
    repo: CountingDetailRepository,
}

impl CountingDetailService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repo: CountingDetailRepository::new(pool),
        }
    }

    pub async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<CountingDetail>, AliothError> {
        self.repo.list(query).await
    }

    pub async fn get(&self, id: i64) -> Result<Option<CountingDetail>, AliothError> {
        self.repo.get(id).await
    }

    pub async fn create(
        &self,
        req: CreateCountingDetailRequest,
        user_id: i64,
    ) -> Result<CountingDetail, AliothError> {
        self.repo.create(req, user_id).await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateCountingDetailRequest,
        user_id: i64,
    ) -> Result<Option<CountingDetail>, AliothError> {
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.repo.delete(id, user_id).await
    }
}

/// StorageNest（储元⇲储元 时空嵌套）CRUD Service
#[derive(Clone)]
pub struct StorageNestService {
    repo: StorageNestRepository,
}

impl StorageNestService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repo: StorageNestRepository::new(pool),
        }
    }

    pub async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<StorageNest>, AliothError> {
        self.repo.list(query).await
    }

    pub async fn get(&self, id: i64) -> Result<Option<StorageNest>, AliothError> {
        self.repo.get(id).await
    }

    pub async fn create(
        &self,
        req: CreateStorageNestRequest,
        user_id: i64,
    ) -> Result<StorageNest, AliothError> {
        self.repo.create(req, user_id).await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateStorageNestRequest,
        user_id: i64,
    ) -> Result<Option<StorageNest>, AliothError> {
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.repo.delete(id, user_id).await
    }
}
