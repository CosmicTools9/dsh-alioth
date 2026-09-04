//! Counting Repository — 盘点事件头（事件-盘点）CRUD
//!
//! 盘点 = 实值校准：事件头记录盘点（范例/实例经 `_t_`/`tpl_id`，盘哪些货经
//! `zc_id_event_rr_matter`），实盘数在明细（`zc_id_deta-counting`，CountingDetail）。
//! 校准（生成校准凭证）由明细创建时自动触发，本事件头不参与物化。
//!
//! 范例/实例（P1-4，SPEC-counting-event-template-instance-links）：
//! - `_t_` 为派生字段，由后端按规则注入：携带 `tpl_id` → '实例'，缺省 → '范例'
//! - 实例 `tpl_id` 指向范例盘点行；读取经 `_refs`（fk_index template 注册）解析范例 notice
//!
//! 盘点范围（P1-4，SPEC-counting-event-matter-m2n-persisted）：
//! 创建事件头时同事务经 `zc_id_event_rr_matter`（ref_left=事件 / ref_right=产品）批量写入；
//! 读取返回产品 `_refs` 对象数组。

use async_trait::async_trait;
use common::error::AliothError as ApiError;
use crud::repository::AliothRepository;
use crud::{GenericRepository, ListQuery, PaginatedResponse};
use sqlx::PgPool;
use trigger_registry::stock_materialization as sm;

use crate::models::{Counting, CreateCountingRequest, MatterRef, UpdateCountingRequest};

/// 抽象层级（`_t_`）取值：范例 / 实例
const KIND_TEMPLATE: &str = "范例";
const KIND_INSTANCE: &str = "实例";

#[derive(Debug, Clone)]
pub struct CountingRepository {
    generic: GenericRepository<Counting>,
}

impl CountingRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }

    /// 读盘点范围（`zc_id_event_rr_matter`，ref_left=事件）→ 产品 `_refs` 对象数组
    async fn matters_of(&self, counting_id: i64) -> Result<Vec<MatterRef>, ApiError> {
        let rows: Vec<(i64, Option<String>)> = sqlx::query_as(
            r#"SELECT m.ref_right, p.notice
               FROM isahl."zc_id_event_rr_matter" m
               LEFT JOIN isahl.zc_id_production p ON p.id = m.ref_right
               WHERE m.ref_left = $1 AND m.deleted_at IS NULL
               ORDER BY m.id"#,
        )
        .bind(counting_id)
        .fetch_all(self.generic.pool())
        .await
        .map_err(ApiError::from)?;

        Ok(rows
            .into_iter()
            .map(|(id, notice)| MatterRef { id, notice })
            .collect())
    }
}

impl From<PgPool> for CountingRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl AliothRepository<Counting, CreateCountingRequest, UpdateCountingRequest, ApiError>
    for CountingRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Counting>, ApiError> {
        self.generic.list(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<Counting>, ApiError> {
        let mut row = self.generic.get(id).await?;
        if let Some(r) = &mut row {
            r.matters = self.matters_of(r.id).await?;
        }
        Ok(row)
    }

    async fn create(&self, req: CreateCountingRequest, user_id: i64) -> Result<Counting, ApiError> {
        // qk_date 为标量引用：日期实际值 → scal-date
        let date_id = match req.counted_date {
            Some(v) => Some(
                sm::ensure_date_scalar(self.generic.pool(), &v)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };

        // `_t_` 派生（D1）：携带 tpl_id → 实例；缺省 → 范例（模板）
        let kind = if req.tpl_id.is_some() {
            KIND_INSTANCE
        } else {
            KIND_TEMPLATE
        };

        // 事件头 + 盘点范围（m2n）同事务写入（D2，复用 with_transaction 语义）
        let mut tx = self.generic.pool().begin().await.map_err(ApiError::from)?;

        let row = sqlx::query_as::<_, Counting>(
            r#"INSERT INTO isahl."zc_id_even-counting"
               (fk_place, qk_date, "_t_", tpl_id, created_by_id)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id, fk_place AS place_id,
                         qk_date AS counted_date,
                         "_t_", tpl_id,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(req.place_id)
        .bind(date_id)
        .bind(kind)
        .bind(req.tpl_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from)?;

        // 盘点范围批量写入（去重，幂等：空范围零行）
        let mut seen = std::collections::HashSet::new();
        let mut prod_ids = Vec::new();
        for pid in req.matters {
            if seen.insert(pid) {
                prod_ids.push(pid);
            }
        }
        for pid in &prod_ids {
            sqlx::query(
                r#"INSERT INTO isahl."zc_id_event_rr_matter"
                   (ref_left, ref_right, created_by_id)
                   VALUES ($1, $2, $3)"#,
            )
            .bind(row.id)
            .bind(pid)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::from)?;
        }

        tx.commit().await.map_err(ApiError::from)?;

        let mut created = row;
        created.matters = self.matters_of(created.id).await?;
        Ok(created)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateCountingRequest,
        user_id: i64,
    ) -> Result<Option<Counting>, ApiError> {
        let date_id = match req.counted_date {
            Some(v) => Some(
                sm::ensure_date_scalar(self.generic.pool(), &v)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };

        let row = sqlx::query_as::<_, Counting>(
            r#"UPDATE isahl."zc_id_even-counting" SET
                   fk_place = COALESCE($1, fk_place),
                   qk_date = COALESCE($2, qk_date),
                   tpl_id = COALESCE($3, tpl_id),
                   "_t_" = CASE WHEN COALESCE($3, tpl_id) IS NULL
                                THEN '范例' ELSE '实例' END,
                   updated_at = NOW(),
                   updated_by_id = $4
               WHERE id = $5 AND deleted_at IS NULL
               RETURNING id, fk_place AS place_id,
                         qk_date AS counted_date,
                         "_t_", tpl_id,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(req.place_id)
        .bind(date_id)
        .bind(req.tpl_id)
        .bind(user_id)
        .bind(id)
        .fetch_optional(self.generic.pool())
        .await
        .map_err(ApiError::from)?;

        let mut row = match row {
            Some(r) => r,
            None => return Ok(None),
        };
        row.matters = self.matters_of(row.id).await?;
        Ok(Some(row))
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        // 盘点事实已发生，软删即可（校准凭证不冲销）
        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_even-counting"
               SET deleted_at = NOW(), updated_by_id = $2
               WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(self.generic.pool())
        .await
        .map_err(ApiError::from)?;

        if rows.rows_affected() == 0 {
            return Err(ApiError::NotFound(format!("counting {} not found", id)));
        }
        Ok(())
    }
}
