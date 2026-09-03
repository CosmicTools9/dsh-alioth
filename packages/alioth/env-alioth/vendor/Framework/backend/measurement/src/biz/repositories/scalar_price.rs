//! 标量值 Repository（biz 共享内核）——委托 GenericRepository + 定制 INSERT
//!
//! zc_id_scal-price 是 zc_id_scale 的子表，存储标量价格/数值实体。
//! 生产逻辑从 WZ measurement 提取（A′ 案）；id 依赖列默认 gen_next_uid(415)，
//! MUST NOT 使用 gen_next_zuid()。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository};
use sqlx::PgPool;

use crate::biz::models::{CreateScalarPriceRequest, ScalarPrice, UpdateScalarPriceRequest};

#[derive(Clone)]
pub struct ScalarPriceRepository {
    generic: GenericRepository<ScalarPrice>,
}

impl From<PgPool> for ScalarPriceRepository {
    fn from(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }
}

#[async_trait]
impl AliothRepository<ScalarPrice, CreateScalarPriceRequest, UpdateScalarPriceRequest, ApiError>
    for ScalarPriceRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<ScalarPrice>, ApiError> {
        self.generic.list(query).await
    }
    async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<PaginatedResponse<ScalarPrice>, ApiError> {
        self.generic
            .list_with_rls(query, visible_ids, authorized_columns)
            .await
    }
    async fn get(&self, id: i64) -> Result<Option<ScalarPrice>, ApiError> {
        self.generic.get(id).await
    }
    async fn get_with_rls(
        &self,
        id: i64,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<Option<ScalarPrice>, ApiError> {
        self.generic
            .get_with_rls(id, visible_ids, authorized_columns)
            .await
    }
    async fn create(
        &self,
        req: CreateScalarPriceRequest,
        user_id: i64,
    ) -> Result<ScalarPrice, ApiError> {
        let p = self.generic.pool();
        sqlx::query_as::<_, ScalarPrice>(
            r#"INSERT INTO isahl."zc_id_scal-price" (notice, code, comments, mark, sk_unit, precision_, retain_signal, t_color_, ref_count, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING id, notice AS name, code, comments, mark AS value,
                         sk_unit AS unit, precision_, retain_signal, t_color_, ref_count,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(req.value)
        .bind(req.unit)
        .bind(req.precision_)
        .bind(req.retain_signal)
        .bind(&req.t_color_)
        .bind(req.ref_count)
        .bind(user_id)
        .fetch_one(p)
        .await
        .map_err(ApiError::from)
    }
    async fn update(
        &self,
        id: i64,
        req: UpdateScalarPriceRequest,
        user_id: i64,
    ) -> Result<Option<ScalarPrice>, ApiError> {
        let p = self.generic.pool();
        sqlx::query_as::<_, ScalarPrice>(
            r#"UPDATE isahl."zc_id_scal-price"
               SET notice = COALESCE($1, notice), code = COALESCE($2, code),
                   comments = COALESCE($3, comments), mark = COALESCE($4, mark),
                   sk_unit = COALESCE($5, sk_unit), precision_ = COALESCE($6, precision_),
                   retain_signal = COALESCE($7, retain_signal), t_color_ = COALESCE($8, t_color_),
                   ref_count = COALESCE($9, ref_count), updated_by_id = $10, updated_at = NOW()
               WHERE id = $11 AND deleted_at IS NULL
               RETURNING id, notice AS name, code, comments, mark AS value,
                         sk_unit AS unit, precision_, retain_signal, t_color_, ref_count,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(req.value)
        .bind(req.unit)
        .bind(req.precision_)
        .bind(req.retain_signal)
        .bind(&req.t_color_)
        .bind(req.ref_count)
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
