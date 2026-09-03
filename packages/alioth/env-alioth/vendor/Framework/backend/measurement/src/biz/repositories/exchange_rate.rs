//! 汇率 Repository（biz 共享内核）——委托 GenericRepository + 定制 INSERT
//!
//! zc_id_rate-exchange 叶表存储货币汇率配对。生产逻辑从 WZ measurement 提取（A′ 案）；
//! id 依赖列默认 gen_next_uid(384)，MUST NOT 使用 gen_next_zuid()。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository};
use sqlx::PgPool;

use crate::biz::models::{CreateExchangeRateRequest, ExchangeRate, UpdateExchangeRateRequest};

#[derive(Clone)]
pub struct ExchangeRateRepository {
    generic: GenericRepository<ExchangeRate>,
}

impl From<PgPool> for ExchangeRateRepository {
    fn from(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }
}

#[async_trait]
impl AliothRepository<ExchangeRate, CreateExchangeRateRequest, UpdateExchangeRateRequest, ApiError>
    for ExchangeRateRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<ExchangeRate>, ApiError> {
        self.generic.list(query).await
    }
    async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<PaginatedResponse<ExchangeRate>, ApiError> {
        self.generic
            .list_with_rls(query, visible_ids, authorized_columns)
            .await
    }
    async fn get(&self, id: i64) -> Result<Option<ExchangeRate>, ApiError> {
        self.generic.get(id).await
    }
    async fn get_with_rls(
        &self,
        id: i64,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<Option<ExchangeRate>, ApiError> {
        self.generic
            .get_with_rls(id, visible_ids, authorized_columns)
            .await
    }
    async fn create(
        &self,
        req: CreateExchangeRateRequest,
        user_id: i64,
    ) -> Result<ExchangeRate, ApiError> {
        let p = self.generic.pool();
        sqlx::query_as::<_, ExchangeRate>(
            r#"INSERT INTO isahl."zc_id_rate-exchange" (notice, ck_left, ck_right, multiply, division, code, date, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               RETURNING id, notice AS name, ck_left AS left_currency, ck_right AS right_currency, multiply AS rate, division AS ask_price, code AS source, date, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(req.left_currency)
        .bind(req.right_currency)
        .bind(req.rate)
        .bind(req.ask_price)
        .bind(&req.source)
        .bind(req.date)
        .bind(user_id)
        .fetch_one(p)
        .await
        .map_err(ApiError::from)
    }
    async fn update(
        &self,
        id: i64,
        req: UpdateExchangeRateRequest,
        user_id: i64,
    ) -> Result<Option<ExchangeRate>, ApiError> {
        let p = self.generic.pool();
        sqlx::query_as::<_, ExchangeRate>(
            r#"UPDATE isahl."zc_id_rate-exchange"
               SET notice = COALESCE($1, notice), ck_left = COALESCE($2, ck_left),
                   ck_right = COALESCE($3, ck_right), multiply = COALESCE($4, multiply),
                   division = COALESCE($5, division), code = COALESCE($6, code),
                   date = COALESCE($7, date),
                   updated_by_id = $8, updated_at = NOW()
               WHERE id = $9 AND deleted_at IS NULL
               RETURNING id, notice AS name, ck_left AS left_currency, ck_right AS right_currency, multiply AS rate, division AS ask_price, code AS source, date, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(req.left_currency)
        .bind(req.right_currency)
        .bind(req.rate)
        .bind(req.ask_price)
        .bind(&req.source)
        .bind(req.date)
        .bind(user_id)
        .bind(id)
        .fetch_optional(p)
        .await
        .map_err(ApiError::from)
    }
    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
