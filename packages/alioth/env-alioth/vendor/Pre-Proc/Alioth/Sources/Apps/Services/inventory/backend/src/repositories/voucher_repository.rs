//! Voucher Repository — 货 stock in/out 储元 CRUD（显式物化版）
//!
//! 物化由 Framework 库函数显式调用（ADR D-018 方案 2）：
//! `stock_materialization::apply_voucher`（时空伴随：voucher 净变 + 余额链回填 + 嵌套 rollup）。
//!
//! ⚠️ 本文件为「并行进程停止后落地」的待应用副本——并行进程覆盖
//! `inventory` 目录时使用此版本恢复。

use async_trait::async_trait;
use common::error::AliothError as ApiError;
use crud::repository::AliothRepository;
use crud::{GenericRepository, ListQuery, PaginatedResponse};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use trigger_registry::stock_materialization as sm;

use crate::models::{CreateVoucherRequest, UpdateVoucherRequest, Voucher};

/// 建单 INSERT 模板——`{title_col}` = 库内当前「物/交易对象」列
/// （[`sm::voucher_title_column`]：新模型 `fk_payload` / 旧模型 `fk_production`），
/// 运行期替换后经 `AssertSqlSafe` 执行；`AS production_id` 输出别名不变（对外 DTO 契约）。
const VOUCHER_INSERT_SQL: &str = r#"INSERT INTO isahl."zc_id_stat-whs-voucher"
               ({title_col}, "fk_subj-storage", "fk_obj-storage", qk_qty, qk_income, qk_outgo, created_by_id,
                dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING id, {title_col} AS production_id, "fk_subj-storage" AS from_storage_id,
                         "fk_obj-storage" AS to_storage_id, qk_qty AS qty, qk_income AS income,
                         qk_outgo AS outgo, qk_pre_balance AS pre_balance, qk_balance AS balance, created_at, updated_at, deleted_at"#;

/// 更新 UPDATE 模板（同款「物」列占位）：两处 `{title_col}` = SET 目标 + RETURNING 投影。
const VOUCHER_UPDATE_SQL: &str = r#"UPDATE isahl."zc_id_stat-whs-voucher" SET
                   {title_col} = COALESCE($1, {title_col}),
                   "fk_subj-storage" = COALESCE($2, "fk_subj-storage"),
                   "fk_obj-storage" = COALESCE($3, "fk_obj-storage"),
                   qk_qty = COALESCE($4, qk_qty),
                   qk_income = COALESCE($5, qk_income),
                   qk_outgo = COALESCE($6, qk_outgo),
                   updated_at = NOW(),
                   updated_by_id = $7
               WHERE id = $8 AND deleted_at IS NULL
               RETURNING id, {title_col} AS production_id, "fk_subj-storage" AS from_storage_id,
                         "fk_obj-storage" AS to_storage_id, qk_qty AS qty, qk_income AS income,
                         qk_outgo AS outgo, qk_pre_balance AS pre_balance, qk_balance AS balance, created_at, updated_at, deleted_at"#;

/// 物化记录构造：`title_col` = 库内当前「物」列（探测值），键用物理列名——
/// 物化侧 `get_title_id` 同时识别 `fk_payload` / `fk_production` 两键。
fn voucher_map(
    title_col: &str,
    id: Option<i64>,
    production_id: Option<i64>,
    from_storage_id: Option<i64>,
    to_storage_id: Option<i64>,
    qty: Option<i64>,
    income: Option<i64>,
    outgo: Option<i64>,
) -> HashMap<String, Value> {
    let mut m = HashMap::new();
    if let Some(v) = id {
        m.insert("id".to_string(), Value::from(v));
    }
    if let Some(v) = production_id {
        m.insert(title_col.to_string(), Value::from(v));
    }
    if let Some(v) = from_storage_id {
        m.insert("fk_subj-storage".to_string(), Value::from(v));
    }
    if let Some(v) = to_storage_id {
        m.insert("fk_obj-storage".to_string(), Value::from(v));
    }
    if let Some(v) = qty {
        m.insert("qk_qty".to_string(), Value::from(v));
    }
    if let Some(v) = income {
        m.insert("qk_income".to_string(), Value::from(v));
    }
    if let Some(v) = outgo {
        m.insert("qk_outgo".to_string(), Value::from(v));
    }
    m
}

#[derive(Debug, Clone)]
pub struct VoucherRepository {
    generic: GenericRepository<Voucher>,
}

impl VoucherRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }
}

impl From<PgPool> for VoucherRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl AliothRepository<Voucher, CreateVoucherRequest, UpdateVoucherRequest, ApiError>
    for VoucherRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Voucher>, ApiError> {
        self.generic.list(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<Voucher>, ApiError> {
        self.generic.get(id).await
    }

    async fn create(&self, req: CreateVoucherRequest, user_id: i64) -> Result<Voucher, ApiError> {
        // qk_* 为标量引用：qty/income/outgo 用户数值 → find_or_create 标量行 → 存标量 ID
        let qty_id = match req.qty {
            Some(v) => Some(
                sm::ensure_scalar(self.generic.pool(), v as f64)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };
        let income_id = match req.income {
            Some(v) => Some(
                sm::ensure_scalar(self.generic.pool(), v as f64)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };
        let outgo_id = match req.outgo {
            Some(v) => Some(
                sm::ensure_scalar(self.generic.pool(), v as f64)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };

        // 「物/交易对象」列随模型演进（新 fk_payload / 旧 fk_production）——运行期探测，
        // 读写同列：SQL 以 {title_col} 占位替换后经 AssertSqlSafe 执行
        let title_col = sm::voucher_title_column(self.generic.pool())
            .await
            .map_err(ApiError::Database)?;

        // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(self.generic.pool(), ("JC", "GID", "↓_LA"))
                .await
                .map_err(ApiError::from)?;
        let insert_sql = VOUCHER_INSERT_SQL.replace("{title_col}", title_col);
        let row = sqlx::query_as::<_, Voucher>(sqlx::AssertSqlSafe(insert_sql))
            .bind(req.production_id)
            .bind(req.from_storage_id)
            .bind(req.to_storage_id)
            .bind(qty_id)
            .bind(income_id)
            .bind(outgo_id)
            .bind(user_id)
            .bind(dk_scene)
            .bind(dk_factor)
            .bind(dk_function)
            .fetch_one(self.generic.pool())
            .await
            .map_err(ApiError::from)?;

        // 物化：时空伴随净变 + 余额链回填（显式调用 Framework 库函数）
        let new_map = voucher_map(
            title_col,
            Some(row.id),
            row.production_id,
            row.from_storage_id,
            row.to_storage_id,
            row.qty,
            row.income,
            row.outgo,
        );
        sm::apply_voucher(self.generic.pool(), None, Some(&new_map))
            .await
            .map_err(ApiError::Database)?;

        Ok(row)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateVoucherRequest,
        user_id: i64,
    ) -> Result<Option<Voucher>, ApiError> {
        let old = match self.generic.get(id).await? {
            Some(e) => e,
            None => return Ok(None),
        };
        // qk_* 为标量引用：qty/income/outgo 数值 → 标量 ID
        let qty_id = match req.qty {
            Some(v) => Some(
                sm::ensure_scalar(self.generic.pool(), v as f64)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };
        let income_id = match req.income {
            Some(v) => Some(
                sm::ensure_scalar(self.generic.pool(), v as f64)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };
        let outgo_id = match req.outgo {
            Some(v) => Some(
                sm::ensure_scalar(self.generic.pool(), v as f64)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };
        // 「物/交易对象」列运行期探测（同 create：读写同列，两态可执行）
        let title_col = sm::voucher_title_column(self.generic.pool())
            .await
            .map_err(ApiError::Database)?;
        let old_map = voucher_map(
            title_col,
            Some(old.id),
            old.production_id,
            old.from_storage_id,
            old.to_storage_id,
            old.qty,
            old.income,
            old.outgo,
        );

        let update_sql = VOUCHER_UPDATE_SQL.replace("{title_col}", title_col);
        let row = sqlx::query_as::<_, Voucher>(sqlx::AssertSqlSafe(update_sql))
            .bind(req.production_id)
            .bind(req.from_storage_id)
            .bind(req.to_storage_id)
            .bind(qty_id)
            .bind(income_id)
            .bind(outgo_id)
            .bind(user_id)
            .bind(id)
            .fetch_optional(self.generic.pool())
            .await
            .map_err(ApiError::from)?;

        if let Some(row) = &row {
            let new_map = voucher_map(
                title_col,
                Some(row.id),
                row.production_id,
                row.from_storage_id,
                row.to_storage_id,
                row.qty,
                row.income,
                row.outgo,
            );
            // 物化：冲销旧 + 应用新
            sm::apply_voucher(self.generic.pool(), Some(&old_map), Some(&new_map))
                .await
                .map_err(ApiError::Database)?;
        }

        Ok(row)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        let old = self
            .generic
            .get(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("voucher {} not found", id)))?;
        // 物化记录键 = 库内当前「物」列（与 create/update 同源探测）
        let title_col = sm::voucher_title_column(self.generic.pool())
            .await
            .map_err(ApiError::Database)?;
        let old_map = voucher_map(
            title_col,
            Some(old.id),
            old.production_id,
            old.from_storage_id,
            old.to_storage_id,
            old.qty,
            old.income,
            old.outgo,
        );

        sqlx::query(
            r#"UPDATE isahl."zc_id_stat-whs-voucher"
               SET deleted_at = NOW(), updated_by_id = $2
               WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(self.generic.pool())
        .await
        .map_err(ApiError::from)?;

        // 物化：冲销
        sm::apply_voucher(self.generic.pool(), Some(&old_map), None)
            .await
            .map_err(ApiError::Database)?;

        Ok(())
    }
}
