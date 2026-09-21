//! CountingDetail Repository — 盘点明细（`zc_id_deta-counting`）CRUD（自动校准版）
//!
//! 盘点 = 实值校准：明细记录实盘截止值（`qk_qty`）。**明细创建时自动校准库存**——
//! 账实差异（实盘 − 物化）经生成**新校准凭证**（`zc_id_stat-sto-voucher`）进入既有
//! voucher 链路，自动触发物化更新（`apply_voucher`：净变 + 余额链 + rollup）：
//! - 盘盈（实盘 > 物化）→ 校准凭证 `qk_income` = 差异，入库位 = 明细储元
//! - 盘亏（实盘 < 物化）→ 校准凭证 `qk_outgo` = |差异|，出库位 = 明细储元
//! - 溯源：**缺桥，已报缺**（§8.5.10「缺桥必须报缺，MUST NOT 借用他桥」）——模型无
//!   「校准凭证 ↔ 盘点明细」桥：通用 `zc_id_statement_rr_reason` 本体零直插，其叶
//!   `zc_id_order_rr_contract` 声明语义 = 关联-订单↔合约（写凭证/明细即越界借桥）。
//!   申请单 = `Pre-Proc/WZ/model-center-requests.md` §R4（「事实↔前因」专叶）。专叶落地前
//!   明细↔凭证**无合法溯源落点**（写径不落行、读径取不到），不得借用他桥。
//!
//! 差异为 0 不生成凭证；UPDATE 明细不重复校准（校准事实已发生）。
//!
//! 关系语义（用户定稿）：
//! - `fk_list` → 所属盘点事件（`zc_id_even-counting`）
//! - `fk_production` → 储位上的存货
//! - `fk_voucher` → 触发盘点的交易凭证（`zc_id_stat-sto-voucher`：储位完成该笔交易后盘点）
//! - `fk_storage` → 储元（与事件头 fk_storage 可同可异；异则经 `zc_id_storage_rr_stock-in` 嵌套）

use async_trait::async_trait;
use common::error::AliothError as ApiError;
use crud::repository::AliothRepository;
use crud::{GenericRepository, ListQuery, PaginatedResponse};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use trigger_registry::stock_materialization as sm;

use crate::models::{CountingDetail, CreateCountingDetailRequest, UpdateCountingDetailRequest};

/// 校准凭证 INSERT 模板（目标表 = 凭证族 `zc_id_stat-whs-voucher`）——
/// `{title_col}` = 库内当前「物/交易对象」列（[`sm::voucher_title_column`]：
/// 新模型 `fk_payload` / 旧模型 `fk_production`），运行期替换后经 `AssertSqlSafe` 执行。
const CALIB_VOUCHER_INSERT_SQL: &str = r#"INSERT INTO isahl."zc_id_stat-whs-voucher"
                       ({title_col}, "fk_subj-storage", "fk_obj-storage",
                        qk_income, qk_outgo, notice, created_by_id,
                        dk_scene, dk_factor, dk_function)
                       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                       RETURNING id"#;

#[derive(Debug, Clone)]
pub struct CountingDetailRepository {
    generic: GenericRepository<CountingDetail>,
}

impl CountingDetailRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }
}

impl From<PgPool> for CountingDetailRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl
    AliothRepository<
        CountingDetail,
        CreateCountingDetailRequest,
        UpdateCountingDetailRequest,
        ApiError,
    > for CountingDetailRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<CountingDetail>, ApiError> {
        self.generic.list(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<CountingDetail>, ApiError> {
        self.generic.get(id).await
    }

    async fn create(
        &self,
        req: CreateCountingDetailRequest,
        user_id: i64,
    ) -> Result<CountingDetail, ApiError> {
        // qk_* 为标量引用：qty/w_qty/v_qty 数值 → 对应标量表；counted_date 日期 → scal-date
        let qty_id = match req.qty {
            Some(v) => Some(
                sm::ensure_scalar(self.generic.pool(), v as f64)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };
        let w_qty_id = match req.w_qty {
            Some(v) => Some(
                sm::ensure_scalar(self.generic.pool(), v as f64)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };
        let v_qty_id = match req.v_qty {
            Some(v) => Some(
                sm::ensure_scalar(self.generic.pool(), v as f64)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };
        let date_id = match req.counted_date {
            Some(v) => Some(
                sm::ensure_date_scalar(self.generic.pool(), &v)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };

        // 坐标三元组（§6.12 声明即必须）：deta-counting 族坐标（JC/FTA/↓_NC），
        // 值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(self.generic.pool(), ("JC", "FTA", "↓_NC"))
                .await
                .map_err(ApiError::from)?;
        let row = sqlx::query_as::<_, CountingDetail>(
            r#"INSERT INTO isahl."zc_id_deta-counting"
               (fk_list, fk_production, fk_storage, fk_voucher,
                qk_qty, qk_w_qty, qk_v_qty, qk_date, fk_biller, created_by_id,
                dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               RETURNING id, fk_list AS counting_id, fk_production AS production_id,
                         fk_storage AS storage_id, fk_voucher AS voucher_id,
                         qk_qty AS qty, qk_w_qty AS w_qty, qk_v_qty AS v_qty,
                         qk_date AS counted_date, fk_biller AS biller_id,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(req.counting_id)
        .bind(req.production_id)
        .bind(req.storage_id)
        .bind(req.voucher_id)
        .bind(qty_id)
        .bind(w_qty_id)
        .bind(v_qty_id)
        .bind(date_id)
        .bind(req.biller_id)
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(self.generic.pool())
        .await
        .map_err(ApiError::from)?;

        // 自动校准：实盘 − 物化 ≠ 0 → 生成校准凭证（走 voucher 链路自动物化）
        if let (Some(prod), Some(storage), Some(qty)) = (row.production_id, row.storage_id, row.qty)
        {
            let actual = sm::scalar_mark(self.generic.pool(), qty)
                .await
                .map_err(ApiError::Database)?;
            let cur = sm::stock_mark(self.generic.pool(), prod, storage)
                .await
                .map_err(ApiError::Database)?;
            let delta = actual - cur;
            if delta != 0.0 {
                let amount_id = sm::ensure_scalar(self.generic.pool(), delta.abs())
                    .await
                    .map_err(ApiError::Database)?;
                // 盘盈 → income + 入库位；盘亏 → outgo + 出库位（储元 = 明细储元）
                let (income_id, outgo_id, subj_storage, obj_storage) = if delta > 0.0 {
                    (Some(amount_id), None, None, Some(storage))
                } else {
                    (None, Some(amount_id), Some(storage), None)
                };

                // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
                let (dk_scene, dk_factor, dk_function) =
                    ontology_binding::resolve(self.generic.pool(), ("JC", "GID", "↓_LA"))
                        .await
                        .map_err(ApiError::from)?;
                // 校准凭证「物/交易对象」列运行期探测（凭证族：新 fk_payload / 旧 fk_production）
                let title_col = sm::voucher_title_column(self.generic.pool())
                    .await
                    .map_err(ApiError::Database)?;
                let insert_sql = CALIB_VOUCHER_INSERT_SQL.replace("{title_col}", title_col);
                let voucher_id: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(insert_sql))
                    .bind(prod)
                    .bind(subj_storage)
                    .bind(obj_storage)
                    .bind(income_id)
                    .bind(outgo_id)
                    .bind("盘点校准")
                    .bind(user_id)
                    .bind(dk_scene)
                    .bind(dk_factor)
                    .bind(dk_function)
                    .fetch_one(self.generic.pool())
                    .await
                    .map_err(ApiError::from)?;

                // 物化：走 apply_voucher（净变 + 余额链回填 + rollup）
                let mut new_map = HashMap::new();
                new_map.insert("id".to_string(), Value::from(voucher_id));
                new_map.insert(title_col.to_string(), Value::from(prod));
                if let Some(s) = subj_storage {
                    new_map.insert("fk_subj-storage".to_string(), Value::from(s));
                }
                if let Some(o) = obj_storage {
                    new_map.insert("fk_obj-storage".to_string(), Value::from(o));
                }
                if let Some(i) = income_id {
                    new_map.insert("qk_income".to_string(), Value::from(i));
                }
                if let Some(o) = outgo_id {
                    new_map.insert("qk_outgo".to_string(), Value::from(o));
                }
                sm::apply_voucher(self.generic.pool(), None, Some(&new_map))
                    .await
                    .map_err(ApiError::Database)?;

                // 溯源：缺桥，已报缺（§8.5.10）——模型无「校准凭证 ↔ 盘点明细」桥，
                // `zc_id_statement_rr_reason` 本体零直插、其叶 `zc_id_order_rr_contract`
                // 声明语义 = 关联-订单↔合约（写凭证/明细即越界借桥）。故此处**不落任何桥行**；
                // 申请单 = `Pre-Proc/WZ/model-center-requests.md` §R4。专叶落地前这条溯源不可用。
            }
        }

        Ok(row)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateCountingDetailRequest,
        user_id: i64,
    ) -> Result<Option<CountingDetail>, ApiError> {
        let qty_id = match req.qty {
            Some(v) => Some(
                sm::ensure_scalar(self.generic.pool(), v as f64)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };
        let w_qty_id = match req.w_qty {
            Some(v) => Some(
                sm::ensure_scalar(self.generic.pool(), v as f64)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };
        let v_qty_id = match req.v_qty {
            Some(v) => Some(
                sm::ensure_scalar(self.generic.pool(), v as f64)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };
        let date_id = match req.counted_date {
            Some(v) => Some(
                sm::ensure_date_scalar(self.generic.pool(), &v)
                    .await
                    .map_err(ApiError::Database)?,
            ),
            None => None,
        };

        let row = sqlx::query_as::<_, CountingDetail>(
            r#"UPDATE isahl."zc_id_deta-counting" SET
                   fk_list = COALESCE($1, fk_list),
                   fk_production = COALESCE($2, fk_production),
                   fk_storage = COALESCE($3, fk_storage),
                   fk_voucher = COALESCE($4, fk_voucher),
                   qk_qty = COALESCE($5, qk_qty),
                   qk_w_qty = COALESCE($6, qk_w_qty),
                   qk_v_qty = COALESCE($7, qk_v_qty),
                   qk_date = COALESCE($8, qk_date),
                   fk_biller = COALESCE($9, fk_biller),
                   updated_at = NOW(),
                   updated_by_id = $10
               WHERE id = $11 AND deleted_at IS NULL
               RETURNING id, fk_list AS counting_id, fk_production AS production_id,
                         fk_storage AS storage_id, fk_voucher AS voucher_id,
                         qk_qty AS qty, qk_w_qty AS w_qty, qk_v_qty AS v_qty,
                         qk_date AS counted_date, fk_biller AS biller_id,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(req.counting_id)
        .bind(req.production_id)
        .bind(req.storage_id)
        .bind(req.voucher_id)
        .bind(qty_id)
        .bind(w_qty_id)
        .bind(v_qty_id)
        .bind(date_id)
        .bind(req.biller_id)
        .bind(user_id)
        .bind(id)
        .fetch_optional(self.generic.pool())
        .await
        .map_err(ApiError::from)?;

        Ok(row)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        // 盘点事实已发生，软删即可（校准凭证不冲销）
        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_deta-counting"
               SET deleted_at = NOW(), updated_by_id = $2
               WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(self.generic.pool())
        .await
        .map_err(ApiError::from)?;

        if rows.rows_affected() == 0 {
            return Err(ApiError::NotFound(format!(
                "counting_detail {} not found",
                id
            )));
        }
        Ok(())
    }
}
