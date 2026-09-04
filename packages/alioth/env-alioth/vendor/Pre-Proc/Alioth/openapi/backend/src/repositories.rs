//! OpenAPI 数据服务产品 Repositories — 标准 CRUD 实现
//!
//! 4 张 isahl 表（config / sales / purchase / made）各一个 repository，
//! 复用 `GenericRepository` 的 list/get/delete + 自定义 create/update
//! （COALESCE 模式，对齐 status 先例）。

use async_trait::async_trait;
use common::AliothError as ApiError;
use crud::repository::AliothRepository;
use crud::{GenericRepository, ListQuery, PaginatedResponse};
use sqlx::PgPool;

use crate::models::*;

// ── 1. 对接配置 ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OpenApiConfigRepository {
    generic: GenericRepository<OpenApiConfig>,
}

impl OpenApiConfigRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }
}

impl From<PgPool> for OpenApiConfigRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl
    AliothRepository<
        OpenApiConfig,
        CreateOpenApiConfigRequest,
        UpdateOpenApiConfigRequest,
        ApiError,
    > for OpenApiConfigRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<OpenApiConfig>, ApiError> {
        self.generic.list(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<OpenApiConfig>, ApiError> {
        self.generic.get(id).await
    }

    async fn create(
        &self,
        req: CreateOpenApiConfigRequest,
        user_id: i64,
    ) -> Result<OpenApiConfig, ApiError> {
        sqlx::query_as::<_, OpenApiConfig>(
            r#"INSERT INTO isahl."zc_id_prot-openapi_config"
               (notice, code, comments, settings, enc_fields, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id, notice AS name, code, comments, settings, enc_fields,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.settings)
        .bind(&req.enc_fields)
        .bind(user_id)
        .fetch_one(self.generic.pool())
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateOpenApiConfigRequest,
        user_id: i64,
    ) -> Result<Option<OpenApiConfig>, ApiError> {
        sqlx::query_as::<_, OpenApiConfig>(
            r#"UPDATE isahl."zc_id_prot-openapi_config"
               SET notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   settings = COALESCE($4, settings),
                   enc_fields = COALESCE($5, enc_fields),
                   updated_at = NOW(), updated_by_id = $6
               WHERE id = $7 AND deleted_at IS NULL
               RETURNING id, notice AS name, code, comments, settings, enc_fields,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.settings)
        .bind(&req.enc_fields)
        .bind(user_id)
        .bind(id)
        .fetch_optional(self.generic.pool())
        .await
        .map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ── 2. 销售侧产品 ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OpenApiSalesRepository {
    generic: GenericRepository<OpenApiSales>,
}

impl OpenApiSalesRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }
}

impl From<PgPool> for OpenApiSalesRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl AliothRepository<OpenApiSales, CreateOpenApiSalesRequest, UpdateOpenApiSalesRequest, ApiError>
    for OpenApiSalesRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<OpenApiSales>, ApiError> {
        self.generic.list(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<OpenApiSales>, ApiError> {
        self.generic.get(id).await
    }

    async fn create(
        &self,
        req: CreateOpenApiSalesRequest,
        user_id: i64,
    ) -> Result<OpenApiSales, ApiError> {
        sqlx::query_as::<_, OpenApiSales>(
            r#"INSERT INTO isahl."zc_id_prod-openapi-sales"
               (notice, code, comments, projection, tpl_id, p_number,
                "fk_subj-demand", "fk_subj-provider", qk_price, fk_process, sk_currency, qk_size,
                created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               RETURNING id, notice AS name, code, comments, projection, tpl_id, p_number,
                         "fk_subj-demand" AS fk_subj_demand, "fk_subj-provider" AS fk_subj_provider,
                         qk_price, fk_process, sk_currency, qk_size,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.projection)
        .bind(req.tpl_id)
        .bind(&req.p_number)
        .bind(req.fk_subj_demand)
        .bind(req.fk_subj_provider)
        .bind(req.qk_price)
        .bind(req.fk_process)
        .bind(req.sk_currency)
        .bind(req.qk_size)
        .bind(user_id)
        .fetch_one(self.generic.pool())
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateOpenApiSalesRequest,
        user_id: i64,
    ) -> Result<Option<OpenApiSales>, ApiError> {
        sqlx::query_as::<_, OpenApiSales>(
            r#"UPDATE isahl."zc_id_prod-openapi-sales"
               SET notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   projection = COALESCE($4, projection),
                   tpl_id = COALESCE($5, tpl_id),
                   p_number = COALESCE($6, p_number),
                   "fk_subj-demand" = COALESCE($7, "fk_subj-demand"),
                   "fk_subj-provider" = COALESCE($8, "fk_subj-provider"),
                   qk_price = COALESCE($9, qk_price),
                   fk_process = COALESCE($10, fk_process),
                   sk_currency = COALESCE($11, sk_currency),
                   qk_size = COALESCE($12, qk_size),
                   updated_at = NOW(), updated_by_id = $13
               WHERE id = $14 AND deleted_at IS NULL
               RETURNING id, notice AS name, code, comments, projection, tpl_id, p_number,
                         "fk_subj-demand" AS fk_subj_demand, "fk_subj-provider" AS fk_subj_provider,
                         qk_price, fk_process, sk_currency, qk_size,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.projection)
        .bind(req.tpl_id)
        .bind(&req.p_number)
        .bind(req.fk_subj_demand)
        .bind(req.fk_subj_provider)
        .bind(req.qk_price)
        .bind(req.fk_process)
        .bind(req.sk_currency)
        .bind(req.qk_size)
        .bind(user_id)
        .bind(id)
        .fetch_optional(self.generic.pool())
        .await
        .map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ── 3. 采购侧产品 ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OpenApiPurchaseRepository {
    generic: GenericRepository<OpenApiPurchase>,
}

impl OpenApiPurchaseRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }
}

impl From<PgPool> for OpenApiPurchaseRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl
    AliothRepository<
        OpenApiPurchase,
        CreateOpenApiPurchaseRequest,
        UpdateOpenApiPurchaseRequest,
        ApiError,
    > for OpenApiPurchaseRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<OpenApiPurchase>, ApiError> {
        self.generic.list(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<OpenApiPurchase>, ApiError> {
        self.generic.get(id).await
    }

    async fn create(
        &self,
        req: CreateOpenApiPurchaseRequest,
        user_id: i64,
    ) -> Result<OpenApiPurchase, ApiError> {
        sqlx::query_as::<_, OpenApiPurchase>(
            r#"INSERT INTO isahl."zc_id_prod-openapi-purchase"
               (notice, code, comments, projection, tpl_id, p_number,
                "fk_subj-demand", "fk_subj-provider", qk_price, fk_process, sk_currency, qk_size,
                created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               RETURNING id, notice AS name, code, comments, projection, tpl_id, p_number,
                         "fk_subj-demand" AS fk_subj_demand, "fk_subj-provider" AS fk_subj_provider,
                         qk_price, fk_process, sk_currency, qk_size,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.projection)
        .bind(req.tpl_id)
        .bind(&req.p_number)
        .bind(req.fk_subj_demand)
        .bind(req.fk_subj_provider)
        .bind(req.qk_price)
        .bind(req.fk_process)
        .bind(req.sk_currency)
        .bind(req.qk_size)
        .bind(user_id)
        .fetch_one(self.generic.pool())
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateOpenApiPurchaseRequest,
        user_id: i64,
    ) -> Result<Option<OpenApiPurchase>, ApiError> {
        sqlx::query_as::<_, OpenApiPurchase>(
            r#"UPDATE isahl."zc_id_prod-openapi-purchase"
               SET notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   projection = COALESCE($4, projection),
                   tpl_id = COALESCE($5, tpl_id),
                   p_number = COALESCE($6, p_number),
                   "fk_subj-demand" = COALESCE($7, "fk_subj-demand"),
                   "fk_subj-provider" = COALESCE($8, "fk_subj-provider"),
                   qk_price = COALESCE($9, qk_price),
                   fk_process = COALESCE($10, fk_process),
                   sk_currency = COALESCE($11, sk_currency),
                   qk_size = COALESCE($12, qk_size),
                   updated_at = NOW(), updated_by_id = $13
               WHERE id = $14 AND deleted_at IS NULL
               RETURNING id, notice AS name, code, comments, projection, tpl_id, p_number,
                         "fk_subj-demand" AS fk_subj_demand, "fk_subj-provider" AS fk_subj_provider,
                         qk_price, fk_process, sk_currency, qk_size,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.projection)
        .bind(req.tpl_id)
        .bind(&req.p_number)
        .bind(req.fk_subj_demand)
        .bind(req.fk_subj_provider)
        .bind(req.qk_price)
        .bind(req.fk_process)
        .bind(req.sk_currency)
        .bind(req.qk_size)
        .bind(user_id)
        .bind(id)
        .fetch_optional(self.generic.pool())
        .await
        .map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ── 4. 制造侧产品 ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OpenApiMadeRepository {
    generic: GenericRepository<OpenApiMade>,
}

impl OpenApiMadeRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }
}

impl From<PgPool> for OpenApiMadeRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl AliothRepository<OpenApiMade, CreateOpenApiMadeRequest, UpdateOpenApiMadeRequest, ApiError>
    for OpenApiMadeRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<OpenApiMade>, ApiError> {
        self.generic.list(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<OpenApiMade>, ApiError> {
        self.generic.get(id).await
    }

    async fn create(
        &self,
        req: CreateOpenApiMadeRequest,
        user_id: i64,
    ) -> Result<OpenApiMade, ApiError> {
        sqlx::query_as::<_, OpenApiMade>(
            r#"INSERT INTO isahl."zc_id_prod-openapi-made"
               (notice, code, comments, projection, tpl_id, p_number,
                "fk_subj-demand", "fk_subj-provider", qk_price, fk_process, sk_currency, qk_size,
                created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               RETURNING id, notice AS name, code, comments, projection, tpl_id, p_number,
                         "fk_subj-demand" AS fk_subj_demand, "fk_subj-provider" AS fk_subj_provider,
                         qk_price, fk_process, sk_currency, qk_size,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.projection)
        .bind(req.tpl_id)
        .bind(&req.p_number)
        .bind(req.fk_subj_demand)
        .bind(req.fk_subj_provider)
        .bind(req.qk_price)
        .bind(req.fk_process)
        .bind(req.sk_currency)
        .bind(req.qk_size)
        .bind(user_id)
        .fetch_one(self.generic.pool())
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateOpenApiMadeRequest,
        user_id: i64,
    ) -> Result<Option<OpenApiMade>, ApiError> {
        sqlx::query_as::<_, OpenApiMade>(
            r#"UPDATE isahl."zc_id_prod-openapi-made"
               SET notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   projection = COALESCE($4, projection),
                   tpl_id = COALESCE($5, tpl_id),
                   p_number = COALESCE($6, p_number),
                   "fk_subj-demand" = COALESCE($7, "fk_subj-demand"),
                   "fk_subj-provider" = COALESCE($8, "fk_subj-provider"),
                   qk_price = COALESCE($9, qk_price),
                   fk_process = COALESCE($10, fk_process),
                   sk_currency = COALESCE($11, sk_currency),
                   qk_size = COALESCE($12, qk_size),
                   updated_at = NOW(), updated_by_id = $13
               WHERE id = $14 AND deleted_at IS NULL
               RETURNING id, notice AS name, code, comments, projection, tpl_id, p_number,
                         "fk_subj-demand" AS fk_subj_demand, "fk_subj-provider" AS fk_subj_provider,
                         qk_price, fk_process, sk_currency, qk_size,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.projection)
        .bind(req.tpl_id)
        .bind(&req.p_number)
        .bind(req.fk_subj_demand)
        .bind(req.fk_subj_provider)
        .bind(req.qk_price)
        .bind(req.fk_process)
        .bind(req.sk_currency)
        .bind(req.qk_size)
        .bind(user_id)
        .bind(id)
        .fetch_optional(self.generic.pool())
        .await
        .map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
