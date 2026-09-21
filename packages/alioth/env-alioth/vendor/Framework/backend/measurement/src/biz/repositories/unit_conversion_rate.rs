//! 单位换算率 Repository（biz 共享内核）——委托 GenericRepository + 叶表路由 INSERT
//!
//! 同量纲单位间换算率，经 `zc_id_rate-*` 叶表存储。生产逻辑从 Alioth measurement
//! 提取（A′ 案）；id 依赖列默认 gen_next_uid(291)，MUST NOT 使用 gen_next_zuid()。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository};
use sqlx::PgPool;

use crate::biz::models::{
    CreateUnitConversionRateRequest, UnitConversionRate, UpdateUnitConversionRateRequest,
};
use crate::biz::repositories::{rate_leaf_sql_for_dimension, RATE_LEAF_PARENT};

#[derive(Clone)]
pub struct UnitConversionRateRepository {
    generic: GenericRepository<UnitConversionRate>,
}

impl From<PgPool> for UnitConversionRateRepository {
    fn from(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }
}

#[async_trait]
impl
    AliothRepository<
        UnitConversionRate,
        CreateUnitConversionRateRequest,
        UpdateUnitConversionRateRequest,
        ApiError,
    > for UnitConversionRateRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<UnitConversionRate>, ApiError> {
        self.generic.list(query).await
    }
    async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<PaginatedResponse<UnitConversionRate>, ApiError> {
        self.generic
            .list_with_rls(query, visible_ids, authorized_columns)
            .await
    }
    async fn get(&self, id: i64) -> Result<Option<UnitConversionRate>, ApiError> {
        self.generic.get(id).await
    }
    async fn get_with_rls(
        &self,
        id: i64,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<Option<UnitConversionRate>, ApiError> {
        self.generic
            .get_with_rls(id, visible_ids, authorized_columns)
            .await
    }
    async fn create(
        &self,
        req: CreateUnitConversionRateRequest,
        user_id: i64,
    ) -> Result<UnitConversionRate, ApiError> {
        let p = self.generic.pool();
        let sql = match req.dimension_key.as_deref() {
            Some(dim_key) => rate_leaf_sql_for_dimension(dim_key).insert_sql,
            None => RATE_LEAF_PARENT.insert_sql,
        };
        sqlx::query_as::<_, UnitConversionRate>(sql)
            .bind(&req.name)
            .bind(req.left)
            .bind(req.right)
            .bind(req.multiply)
            .bind(req.division)
            .bind(req.precision_)
            .bind(req.intrinsic)
            .bind(user_id)
            .fetch_one(p)
            .await
            .map_err(ApiError::from)
    }
    async fn update(
        &self,
        id: i64,
        req: UpdateUnitConversionRateRequest,
        user_id: i64,
    ) -> Result<Option<UnitConversionRate>, ApiError> {
        let p = self.generic.pool();
        sqlx::query_as::<_, UnitConversionRate>(
            r#"UPDATE isahl.zc_id_rate AS e
               SET notice = COALESCE($1, notice), ck_left = COALESCE($2, ck_left),
                   ck_right = COALESCE($3, ck_right), multiply = COALESCE($4, multiply),
                   division = COALESCE($5, division), precision_ = COALESCE($6, precision_),
                   intrinsic = COALESCE($7, intrinsic), updated_by_id = $8, updated_at = NOW()
               WHERE id = $9 AND deleted_at IS NULL
               RETURNING id, notice AS name, ck_left AS left, ck_right AS right,
                         multiply, division, precision_, intrinsic,
                         CASE WHEN e.tableoid = 'isahl.zc_id_rate'::regclass THEN NULL
                              ELSE replace((SELECT relname FROM pg_class WHERE oid = e.tableoid), 'zc_id_rate-', '') END AS dimension,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(req.left)
        .bind(req.right)
        .bind(req.multiply)
        .bind(req.division)
        .bind(req.precision_)
        .bind(req.intrinsic)
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
