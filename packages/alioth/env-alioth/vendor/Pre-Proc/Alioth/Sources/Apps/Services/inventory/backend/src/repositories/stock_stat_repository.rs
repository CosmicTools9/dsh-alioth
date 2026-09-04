//! StockStat Repository — 只读库存统计查询
//!
//! 物化储量读取：`rr_storage.qk_qty` 为标量引用（→ zc_id_scal-common.mark），
//! 读侧 JOIN 标量表取真值（O(1)）。统计值由 isahl 触发器增量维护。
//! 读侧对照融合：LATERAL 关联最近盘点明细，输出实盘截止值 `last_counted_qty`
//! 与盘盈盘亏 `variance`（实盘 − 物化，无盘点为 NULL）。

use common::error::AliothError as ApiError;
use sqlx::PgPool;

use crate::models::StockStat;

#[derive(Debug, Clone)]
pub struct StockStatRepository {
    pool: PgPool,
}

impl StockStatRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 按 (产品, 储元) 过滤查询物化库存统计（可空过滤条件）
    ///
    /// 时空伴随规范（物理层）：仅统计 `rr_storage.qk_period` 指向时间段覆盖 `now()` 的
    /// 生效关系（date_st/date_ed 为空 = 开放生效；qk_period 为空 = 无生效期约束）。
    pub async fn statistics(
        &self,
        production_id: Option<i64>,
        storage_id: Option<i64>,
    ) -> Result<Vec<StockStat>, ApiError> {
        let rows: Vec<StockStat> = sqlx::query_as::<_, StockStat>(
            r#"
            SELECT
                r.id,
                r.ref_left AS production_id,
                r.ref_right AS storage_id,
                COALESCE(sm.mark, 0) AS qty,
                r.sk_unit AS unit,
                cnt.counted_mark AS last_counted_qty,
                (cnt.counted_mark - COALESCE(sm.mark, 0)) AS variance,
                r.created_at,
                r.updated_at
            FROM isahl."zc_id_production_rr_storage" r
            LEFT JOIN isahl."zc_id_scal-common" sm ON sm.id = r.qk_qty
            LEFT JOIN isahl."zc_id_segm-date" pd ON pd.id = r.qk_period
            LEFT JOIN LATERAL (
                SELECT sc.mark AS counted_mark
                FROM isahl."zc_id_deta-counting" d
                LEFT JOIN isahl."zc_id_scal-common" sc ON sc.id = d.qk_qty
                LEFT JOIN isahl."zc_id_scal-date" sd ON sd.id = d.qk_date
                WHERE d.fk_production = r.ref_left AND d.fk_storage = r.ref_right
                  AND d.deleted_at IS NULL
                ORDER BY sd.date DESC NULLS LAST, d.id DESC
                LIMIT 1
            ) cnt ON TRUE
            WHERE r.deleted_at IS NULL
              AND ($1::bigint IS NULL OR r.ref_left = $1)
              AND ($2::bigint IS NULL OR r.ref_right = $2)
              AND (r.qk_period IS NULL OR pd.date_st IS NULL OR pd.date_ed IS NULL
                   OR now() BETWEEN pd.date_st AND pd.date_ed)
            ORDER BY r.ref_left, r.ref_right, r.id
            "#,
        )
        .bind(production_id)
        .bind(storage_id)
        .fetch_all(&self.pool)
        .await
        .map_err(ApiError::from)?;

        Ok(rows)
    }

    /// 按关系行 id 查单条物化库存（含最近盘点实值对照）
    pub async fn get_stat(&self, id: i64) -> Result<Option<StockStat>, ApiError> {
        let row = sqlx::query_as::<_, StockStat>(
            r#"
            SELECT
                r.id,
                r.ref_left AS production_id,
                r.ref_right AS storage_id,
                COALESCE(sm.mark, 0) AS qty,
                r.sk_unit AS unit,
                cnt.counted_mark AS last_counted_qty,
                (cnt.counted_mark - COALESCE(sm.mark, 0)) AS variance,
                r.created_at,
                r.updated_at
            FROM isahl."zc_id_production_rr_storage" r
            LEFT JOIN isahl."zc_id_scal-common" sm ON sm.id = r.qk_qty
            LEFT JOIN LATERAL (
                SELECT sc.mark AS counted_mark
                FROM isahl."zc_id_deta-counting" d
                LEFT JOIN isahl."zc_id_scal-common" sc ON sc.id = d.qk_qty
                LEFT JOIN isahl."zc_id_scal-date" sd ON sd.id = d.qk_date
                WHERE d.fk_production = r.ref_left AND d.fk_storage = r.ref_right
                  AND d.deleted_at IS NULL
                ORDER BY sd.date DESC NULLS LAST, d.id DESC
                LIMIT 1
            ) cnt ON TRUE
            WHERE r.id = $1 AND r.deleted_at IS NULL
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(ApiError::from)?;

        Ok(row)
    }
}
