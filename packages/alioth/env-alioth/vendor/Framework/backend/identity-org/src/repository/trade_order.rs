// TradeOrderRepository（split 自 repository.rs 单体，④ 候选）
//! 身份实体 Repository — 标准 CRUD 实现
//!
//! Identity 使用自定义 Repository，其余实体组合 GenericRepository，
//! 仅自定义 create/update 的 INSERT/UPDATE SQL。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::repository::AliothRepository;
use crud::GenericRepository;
use sqlx::{AssertSqlSafe, PgPool};

use crate::models::{CreateTradeOrderRequest, TradeOrder, UpdateTradeOrderRequest};

use super::ontology_binding;

// ═══════════════════════════════════════════════
// TradeOrder — "isahl"."zc_id_deta-trade_order"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct TradeOrderRepository {
    generic: GenericRepository<TradeOrder>,
    pool: PgPool,
}

impl TradeOrderRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for TradeOrderRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<TradeOrder, CreateTradeOrderRequest, UpdateTradeOrderRequest, ApiError>
    for TradeOrderRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<TradeOrder>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<TradeOrder>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateTradeOrderRequest,
        user_id: i64,
    ) -> Result<TradeOrder, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "TradeOrder").await?;
        sqlx::query_as::<_, TradeOrder>(
            r#"INSERT INTO "isahl"."zc_id_deta-trade_order" (code, notice, comments, fk_goods, fk_demand, fk_delivery, fk_deal, fk_biller, fk_counterparty, qk_price, qk_qty, qk_amount, sk_currency, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
               RETURNING id, code, notice, comments, fk_goods, fk_demand, fk_delivery, fk_deal, fk_biller, fk_counterparty, qk_price, qk_qty, qk_amount, sk_currency, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_goods).bind(req.fk_demand).bind(req.fk_delivery).bind(req.fk_deal)
        .bind(req.fk_biller).bind(req.fk_counterparty)
        .bind(req.qk_price).bind(req.qk_qty).bind(req.qk_amount)
        .bind(req.sk_currency)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateTradeOrderRequest,
        user_id: i64,
    ) -> Result<Option<TradeOrder>, ApiError> {
        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if req.code.is_some() {
            idx += 1;
            sets.push(format!("code = ${}", idx));
        }
        if req.notice.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
        if req.comments.is_some() {
            idx += 1;
            sets.push(format!("comments = ${}", idx));
        }
        if req.fk_goods.is_some() {
            idx += 1;
            sets.push(format!("fk_goods = ${}", idx));
        }
        if req.fk_demand.is_some() {
            idx += 1;
            sets.push(format!("fk_demand = ${}", idx));
        }
        if req.fk_delivery.is_some() {
            idx += 1;
            sets.push(format!("fk_delivery = ${}", idx));
        }
        if req.fk_deal.is_some() {
            idx += 1;
            sets.push(format!("fk_deal = ${}", idx));
        }
        if req.fk_biller.is_some() {
            idx += 1;
            sets.push(format!("fk_biller = ${}", idx));
        }
        if req.fk_counterparty.is_some() {
            idx += 1;
            sets.push(format!("fk_counterparty = ${}", idx));
        }
        if req.qk_price.is_some() {
            idx += 1;
            sets.push(format!("qk_price = ${}", idx));
        }
        if req.qk_qty.is_some() {
            idx += 1;
            sets.push(format!("qk_qty = ${}", idx));
        }
        if req.qk_amount.is_some() {
            idx += 1;
            sets.push(format!("qk_amount = ${}", idx));
        }
        if req.sk_currency.is_some() {
            idx += 1;
            sets.push(format!("sk_currency = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_deta-trade_order" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_goods, fk_demand, fk_delivery, fk_deal, fk_biller, fk_counterparty, qk_price, qk_qty, qk_amount, sk_currency, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, TradeOrder>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_goods {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_demand {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_delivery {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_deal {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_biller {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_counterparty {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_price {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_qty {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_amount {
            q = q.bind(v);
        }
        if let Some(v) = req.sk_currency {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
