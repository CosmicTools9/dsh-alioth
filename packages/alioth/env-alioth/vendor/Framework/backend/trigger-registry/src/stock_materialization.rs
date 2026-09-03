//! Stock Materialization — 库存时空伴随物化库
//!
//! 库存基础逻辑（通用后端数据逻辑）承载于 Framework（ADR D-018）。
//! 本模块提供库函数，由业务 Service 在写路径上显式调用（方案：显式调用，
//! 不依赖触发器注册；框架内部亦可复用）。
//!
//! ## 时空伴随规范（严格）
//!
//! 库存是时空伴随的统计值：
//! 1. **关系生效期**：`zc_id_production_rr_storage.qk_period` → `zc_id_segm-date`
//!    标识「货 ⇲ 箱/位」关系的生效时间；**关系提取（统计/查询）必须过滤时间段**：
//!    仅统计 `now()` 落在 `[date_st, date_ed]` 内的生效关系（date_ed 为空 = 开放）
//! 2. **货 stock in/out**：`zc_id_stat-sto-voucher` 是货与箱/位的伴随事实（净变）
//! 3. **储元嵌套**：`zc_id_storage_rr_stock-in` 行存在即「置入」，嵌套同样带时段
//! 4. 物化载体 = `rr_storage.qk_qty` 指向 `zc_id_scal-common.mark`（标量引用模型）
//! 5. **多计量形式**（2026-08-20 用户定稿）：同一库存可同时具备 计数/重量/体积/金额
//!    四计量——`qk_qty`/`qk_w_qty`/`qk_v_qty`/`qk_amount` 独立标量（金额落 `zc_id_scal-amount`，
//!    其余落 `zc_id_scal-common`），互不换算；凭证表达主计量，附加计量由维度原语
//!    （`apply_stock_delta_dim_tx` 等）同事务物化；`mv_inventory` 物化四列（qty/w_qty/v_qty/amount），
//!    标量解析统一 JOIN 父表 `zc_id_scale`（子表查询对父表直插行不可见——实测坑）
//!
//! 统计口径（当前快照）：
//! ```text
//! 库存(产品 P, 储元 S) = Σ_{v ∈ voucher(P,S)} (income − outgo)   -- 伴随事实
//!                      + Σ_{盘点校正(P,S)}                         -- 盘点校正
//!                      + Σ_{子储元 T : 当前嵌套于 S} 库存(P, T)    -- 嵌套 rollup
//! 仅统计 rr_storage.qk_period 覆盖 now() 的生效关系行
//! ```

use rust_decimal::prelude::FromPrimitive;
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

// ============================================
// isahl.mv_inventory 物化视图（用户定稿命名 + dev 基准）
// ============================================

/// `isahl.mv_inventory` 物化视图 DDL（内嵌幂等——单源防漂移，用户 2026-08-07 定稿）。
///
/// 定义按 `stock_stat` statistics 查询物化（无过滤参数版）：
/// - 基表 `zc_id_production_rr_storage`（ref_left→production_id / ref_right→storage_id）
/// - `qk_qty` 标量 JOIN `zc_id_scal-common.mark` 取真值（物化列 qty = 实际库存）
/// - `qk_p_capacity` 标量 JOIN `zc_id_scal-common.mark` 取真值（物化列 capacity = 品容）
/// - `sk_unit` 直出 unit
/// - LATERAL 最近盘点（`zc_id_deta-counting` 实盘截止值 last_counted_qty / variance）
/// - 过滤 deleted_at + qk_period 时空伴随（`zc_id_segm-date` 生效期覆盖 now()）
/// - 物化为快照（REFRESH 时点求值）；读侧 StockStat 仍实时 JOIN（视图为可选读优化载体）
///
/// 冻结边界（ENVIRONMENT_SPEC §11）：isahl schema 冻结规则为「CREATE VIEW 允许 /
/// ALTER TABLE 禁止」（实证：`scripts/db/vw_capacity_available.sql:9` 先例），
/// 本 DDL 仅创建物化视图与索引，合规。
pub const MV_INVENTORY_DDL: &[&str] = &[
    r#"CREATE MATERIALIZED VIEW IF NOT EXISTS isahl.mv_inventory AS
        SELECT
            r.id,
            r.ref_left AS production_id,
            r.ref_right AS storage_id,
            COALESCE(sm.mark, 0) AS qty,
            COALESCE(wm.mark, 0) AS w_qty,
            COALESCE(vm.mark, 0) AS v_qty,
            COALESCE(am.mark, 0) AS amount,
            COALESCE(cap.mark, 0) AS capacity,
            r.sk_unit AS unit,
            cnt.counted_mark AS last_counted_qty,
            (cnt.counted_mark - COALESCE(sm.mark, 0)) AS variance,
            r.created_at,
            r.updated_at
        FROM isahl."zc_id_production_rr_storage" r
        -- 标量解析统一 JOIN 父表 zc_id_scale（覆盖 scal-common/scal-amount/直插行；
        -- 子表查询对父表直插行不可见——fix-wz-capacity-unified-inventory 实测坑）
        LEFT JOIN isahl."zc_id_scale" sm ON sm.id = r.qk_qty
        LEFT JOIN isahl."zc_id_scale" wm ON wm.id = r.qk_w_qty
        LEFT JOIN isahl."zc_id_scale" vm ON vm.id = r.qk_v_qty
        LEFT JOIN isahl."zc_id_scale" am ON am.id = r.qk_amount
        LEFT JOIN isahl."zc_id_scale" cap ON cap.id = r.qk_p_capacity
        LEFT JOIN isahl."zc_id_segm-date" pd ON pd.id = r.qk_period
        LEFT JOIN LATERAL (
            SELECT sc.mark AS counted_mark
            FROM isahl."zc_id_deta-counting" d
            LEFT JOIN isahl."zc_id_scale" sc ON sc.id = d.qk_qty
            LEFT JOIN isahl."zc_id_scal-date" sd ON sd.id = d.qk_date
            WHERE d.fk_production = r.ref_left AND d.fk_storage = r.ref_right
              AND d.deleted_at IS NULL
            ORDER BY sd.date DESC NULLS LAST, d.id DESC
            LIMIT 1
        ) cnt ON TRUE
        WHERE r.deleted_at IS NULL
          AND (r.qk_period IS NULL OR pd.date_st IS NULL OR pd.date_ed IS NULL
               OR now() BETWEEN pd.date_st AND pd.date_ed)"#,
    r#"CREATE UNIQUE INDEX IF NOT EXISTS idx_mv_inventory_id
       ON isahl.mv_inventory (id)"#,
    r#"CREATE INDEX IF NOT EXISTS idx_mv_inventory_prod_storage
       ON isahl.mv_inventory (production_id, storage_id)"#,
];

// ============================================
// 辅助：祖先链（存在型嵌套，深度防护）
// ============================================

/// 储元当前祖先链（行存在=置入；depth<50 防环）
pub async fn storage_ancestors(pool: &PgPool, child: i64) -> Result<Vec<i64>, String> {
    let rows: Vec<(i64,)> = sqlx::query_as(
        r#"
        WITH RECURSIVE chain AS (
            SELECT n.ref_left AS parent, 1 AS depth
            FROM isahl."zc_id_storage_rr_stock-in" n
            WHERE n.ref_right = $1 AND n.deleted_at IS NULL
            UNION ALL
            SELECT n.ref_left, c.depth + 1
            FROM isahl."zc_id_storage_rr_stock-in" n
            JOIN chain c ON c.parent = n.ref_right AND c.depth < 50
            WHERE n.deleted_at IS NULL
        )
        SELECT parent FROM chain
        "#,
    )
    .bind(child)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("ancestors: {}", e))?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

// ============================================
// 辅助：当前嵌套判定（行存在=置入）
// ============================================

/// (parent, child) 当前是否嵌套（存在未删除行）
pub async fn is_currently_nested(pool: &PgPool, parent: i64, child: i64) -> Result<bool, String> {
    let exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(SELECT 1 FROM isahl."zc_id_storage_rr_stock-in"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL)"#,
    )
    .bind(parent)
    .bind(child)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("is_currently_nested: {}", e))?;
    Ok(exists)
}

/// (parent, child) 当前是否嵌套（排除指定行 id）
pub async fn is_currently_nested_excl(
    pool: &PgPool,
    parent: i64,
    child: i64,
    excl_id: i64,
) -> Result<bool, String> {
    let exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(SELECT 1 FROM isahl."zc_id_storage_rr_stock-in"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL AND id <> $3)"#,
    )
    .bind(parent)
    .bind(child)
    .bind(excl_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("is_currently_nested_excl: {}", e))?;
    Ok(exists)
}

// ============================================
// 辅助：标量行确保 + 增量
// ============================================

/// 确保 (产品, 储元) 的 rr_storage 行 + 标量行存在，返回标量行 id
pub async fn ensure_stock_row(pool: &PgPool, prod: i64, storage: i64) -> Result<i64, String> {
    let rel: Option<(i64, Option<i64>)> = sqlx::query_as(
        r#"SELECT id, qk_qty FROM isahl."zc_id_production_rr_storage"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL
           FOR UPDATE"#,
    )
    .bind(prod)
    .bind(storage)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("ensure rel: {}", e))?;

    let (rel_id, scalar_id) = match rel {
        Some(r) => r,
        None => {
            let id: i64 = sqlx::query_scalar(
                r#"INSERT INTO isahl."zc_id_production_rr_storage" (ref_left, ref_right)
                   VALUES ($1, $2) RETURNING id"#,
            )
            .bind(prod)
            .bind(storage)
            .fetch_one(pool)
            .await
            .map_err(|e| format!("insert rel: {}", e))?;
            (id, None)
        }
    };

    if let Some(sid) = scalar_id {
        return Ok(sid);
    }

    let sid: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (mark) VALUES (0) RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("insert scalar: {}", e))?;

    sqlx::query(r#"UPDATE isahl."zc_id_production_rr_storage" SET qk_qty = $1 WHERE id = $2"#)
        .bind(sid)
        .bind(rel_id)
        .execute(pool)
        .await
        .map_err(|e| format!("backfill scalar: {}", e))?;

    Ok(sid)
}

/// 对 (产品, 储元) 累加 delta 并沿当前祖先链传播（物化 mark）
pub async fn apply_stock_delta(
    pool: &PgPool,
    prod: i64,
    storage: i64,
    delta: f64,
) -> Result<(), String> {
    if delta == 0.0 {
        return Ok(());
    }
    let mut targets = vec![storage];
    targets.extend(storage_ancestors(pool, storage).await?);

    for s in targets {
        let sid = ensure_stock_row(pool, prod, s).await?;
        sqlx::query(
            r#"UPDATE isahl."zc_id_scal-common" SET mark = COALESCE(mark, 0) + $1 WHERE id = $2"#,
        )
        .bind(rust_decimal::Decimal::from_f64(delta).unwrap_or_default())
        .bind(sid)
        .execute(pool)
        .await
        .map_err(|e| format!("apply delta: {}", e))?;
    }
    Ok(())
}

/// 当前物化 mark：(产品, 储元)
pub async fn stock_mark(pool: &PgPool, prod: i64, storage: i64) -> Result<f64, String> {
    let mark: Option<f64> = sqlx::query_scalar(
        r#"SELECT CAST(COALESCE(sm.mark, 0) AS float8)
           FROM isahl."zc_id_production_rr_storage" r
           LEFT JOIN isahl."zc_id_scal-common" sm ON sm.id = r.qk_qty
           WHERE r.ref_left = $1 AND r.ref_right = $2 AND r.deleted_at IS NULL"#,
    )
    .bind(prod)
    .bind(storage)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("stock_mark: {}", e))?;
    Ok(mark.unwrap_or(0.0))
}

/// 子储元下全部有库存的产品及总量（嵌套 rollup 用）
pub async fn products_with_stock(pool: &PgPool, child: i64) -> Result<Vec<(i64, f64)>, String> {
    let rows: Vec<(i64, f64)> = sqlx::query_as(
        r#"SELECT r.ref_left, CAST(COALESCE(sm.mark, 0) AS float8)
           FROM isahl."zc_id_production_rr_storage" r
           JOIN isahl."zc_id_scal-common" sm ON sm.id = r.qk_qty
           WHERE r.ref_right = $1 AND r.deleted_at IS NULL
             AND COALESCE(sm.mark, 0) <> 0"#,
    )
    .bind(child)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("products_with_stock: {}", e))?;
    Ok(rows)
}

// ============================================
// 标量引用读写（qk_* 均为标量引用：bigint 存标量 ID，非数值）
// ============================================

/// 读标量行真值（mark）
///
/// 经父表 `zc_id_scale` 解析（继承覆盖 scal-common/scal-weight/scal-amount 等全部
/// 子表）——凭证 qk_income/qk_outgo 允许引用类型化标量行；子表查询对兄弟子表行
/// 不可见（fix-wz-capacity-unified-inventory 实测坑，同 MV_INVENTORY_DDL 注记）。
pub async fn scalar_mark(pool: &PgPool, scalar_id: i64) -> Result<f64, String> {
    let mark: Option<f64> = sqlx::query_scalar(
        r#"SELECT CAST(COALESCE(mark, 0) AS float8) FROM isahl."zc_id_scale" WHERE id = $1"#,
    )
    .bind(scalar_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("scalar_mark: {}", e))?;
    Ok(mark.unwrap_or(0.0))
}

/// 确保数值对应的标量行存在（find_or_create by mark），返回标量 ID
pub async fn ensure_scalar(pool: &PgPool, value: f64) -> Result<i64, String> {
    let existing: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_scal-common" WHERE mark = $1 ORDER BY id LIMIT 1"#,
    )
    .bind(value)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("ensure_scalar find: {}", e))?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (mark) VALUES ($1) RETURNING id"#,
    )
    .bind(value)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("ensure_scalar insert: {}", e))?;
    Ok(id)
}

/// 确保日期对应的日期标量行存在（find_or_create by date），返回标量 ID
/// （qk_date → zc_id_scal-date.date）
pub async fn ensure_date_scalar(pool: &PgPool, value: &str) -> Result<i64, String> {
    let existing: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_scal-date" WHERE date = $1 ORDER BY id LIMIT 1"#,
    )
    .bind(value)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("ensure_date_scalar find: {}", e))?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-date" (date) VALUES ($1) RETURNING id"#,
    )
    .bind(value)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("ensure_date_scalar insert: {}", e))?;
    Ok(id)
}

// ============================================
// 业务入口（Service 写路径显式调用）
// ============================================

/// Voucher 净变：old → 冲销，new → 应用（fk_production/fk_obj-storage/fk_subj-storage
/// 及 qk_income/qk_outgo/qk_qty 以物理列名传入 HashMap；缺省记录传 None）
pub async fn apply_voucher(
    pool: &PgPool,
    old: Option<&HashMap<String, Value>>,
    new: Option<&HashMap<String, Value>>,
) -> Result<(), String> {
    if let Some(old) = old {
        if let Some(prod) = get_id(old, "fk_production") {
            // qk_income/qk_outgo/qk_qty 为标量引用（bigint 存标量 ID）——读真值
            let income = match get_id(old, "qk_income").or_else(|| get_id(old, "qk_qty")) {
                Some(sid) => scalar_mark(pool, sid).await?,
                None => 0.0,
            };
            let outgo = match get_id(old, "qk_outgo").or_else(|| get_id(old, "qk_qty")) {
                Some(sid) => scalar_mark(pool, sid).await?,
                None => 0.0,
            };
            if let Some(obj) = get_id(old, "fk_obj-storage") {
                apply_stock_delta(pool, prod, obj, -income).await?;
            }
            if let Some(subj) = get_id(old, "fk_subj-storage") {
                apply_stock_delta(pool, prod, subj, outgo).await?;
            }
        }
    }
    if let Some(new) = new {
        if let Some(prod) = get_id(new, "fk_production") {
            let income = match get_id(new, "qk_income").or_else(|| get_id(new, "qk_qty")) {
                Some(sid) => scalar_mark(pool, sid).await?,
                None => 0.0,
            };
            let outgo = match get_id(new, "qk_outgo").or_else(|| get_id(new, "qk_qty")) {
                Some(sid) => scalar_mark(pool, sid).await?,
                None => 0.0,
            };
            if let Some(obj) = get_id(new, "fk_obj-storage") {
                apply_stock_delta(pool, prod, obj, income).await?;
            }
            if let Some(subj) = get_id(new, "fk_subj-storage") {
                apply_stock_delta(pool, prod, subj, -outgo).await?;
            }
            // 余额链回填（凭证通用逻辑：qk_pre_balance=期初余额（事实发生前），
            // qk_balance=期末余额（事实发生后）；主储元 = 入库位（obj）优先，否则出库位（subj）。
            // qk_* 为标量引用——创建标量行（mark=余额值）并回填其 ID。
            // 期初余额语义（账本）：链起点（该 prod+storage 无前置凭证）期初=0；
            // 非首笔 before = after − income + outgo（反推=前笔期末）。
            if let Some(id) = get_id(new, "id") {
                let main = get_id(new, "fk_obj-storage").or_else(|| get_id(new, "fk_subj-storage"));
                if let Some(storage) = main {
                    let after = stock_mark(pool, prod, storage).await?;
                    // fix-wz-capacity-calculation：余额回填表感知（sto/com/tsp/slf 四族）
                    let mut conn = pool
                        .acquire()
                        .await
                        .map_err(|e| format!("balance backfill acquire: {e}"))?;
                    backfill_voucher_balance_tx(
                        &mut *conn, prod, storage, id, income, outgo, after,
                    )
                    .await?;
                }
            }
        }
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════
// 事务内变体（&mut PgConnection）——业务 Service 在写事务内调用，
// 与凭证/关系行写入同一事务（原子：回滚即全部回滚）。
// 语义与 &PgPool 版完全一致（同一 SQL 体）。
// ═══════════════════════════════════════════════════════
// ═══════════════════════════════════════════════════════
// 多计量维度（用户定稿：库存 = (时间段, 储元, 商品/服务) → 数量，
// 同一库存可同时具备 计数/重量/体积/金额 多个计量形式）
// ═══════════════════════════════════════════════════════

/// 库存计量维度：rr_storage 目标列 + 标量族静态映射（编译期白名单，无动态 SQL 注入面）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StockDim {
    /// 计数（qk_qty → zc_id_scal-common）
    Qty,
    /// 重量（qk_w_qty → zc_id_scal-common）
    Weight,
    /// 体积（qk_v_qty → zc_id_scal-common）
    Volume,
    /// 金额（qk_amount → zc_id_scal-amount）
    Amount,
}

impl StockDim {
    /// rr_storage 目标列名（静态白名单）
    pub fn column(self) -> &'static str {
        match self {
            StockDim::Qty => "qk_qty",
            StockDim::Weight => "qk_w_qty",
            StockDim::Volume => "qk_v_qty",
            StockDim::Amount => "qk_amount",
        }
    }
}

/// 维度 → 标量族（金额落 scal-amount，其余落 scal-common；两者均为 zc_id_scale 子表）
fn scalar_table(dim: StockDim) -> &'static str {
    match dim {
        StockDim::Amount => "isahl.\"zc_id_scal-amount\"",
        _ => "isahl.\"zc_id_scal-common\"",
    }
}

/// 事务版：储元当前祖先链（行存在=置入；depth<50 防环）
pub async fn storage_ancestors_tx(
    conn: &mut sqlx::PgConnection,
    child: i64,
) -> Result<Vec<i64>, String> {
    let rows: Vec<(i64,)> = sqlx::query_as(
        r#"
        WITH RECURSIVE chain AS (
            SELECT n.ref_left AS parent, 1 AS depth
            FROM isahl."zc_id_storage_rr_stock-in" n
            WHERE n.ref_right = $1 AND n.deleted_at IS NULL
            UNION ALL
            SELECT n.ref_left, c.depth + 1
            FROM isahl."zc_id_storage_rr_stock-in" n
            JOIN chain c ON c.parent = n.ref_right AND c.depth < 50
            WHERE n.deleted_at IS NULL
        )
        SELECT parent FROM chain
        "#,
    )
    .bind(child)
    .fetch_all(&mut *conn)
    .await
    .map_err(|e| format!("ancestors_tx: {}", e))?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// 事务版：确保 (产品, 储元) 的 rr_storage 行 + 指定计量列标量行存在，返回标量行 id
///
/// 多计量维度（用户定稿：计数/重量/体积/金额独立标量）；金额维度落 `zc_id_scal-amount`，
/// 其余落 `zc_id_scal-common`。行不存在时创建（ref_left/ref_right）。
pub async fn ensure_stock_row_dim_tx(
    conn: &mut sqlx::PgConnection,
    prod: i64,
    storage: i64,
    dim: StockDim,
) -> Result<i64, String> {
    let col = dim.column();
    let sql = format!(
        r#"SELECT id, {col} FROM isahl."zc_id_production_rr_storage"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL
           FOR UPDATE"#
    );
    let rel: Option<(i64, Option<i64>)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(prod)
        .bind(storage)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| format!("ensure rel dim tx: {}", e))?;

    let (rel_id, scalar_id) = match rel {
        Some(r) => r,
        None => {
            let id: i64 = sqlx::query_scalar(
                r#"INSERT INTO isahl."zc_id_production_rr_storage" (ref_left, ref_right)
                   VALUES ($1, $2) RETURNING id"#,
            )
            .bind(prod)
            .bind(storage)
            .fetch_one(&mut *conn)
            .await
            .map_err(|e| format!("insert rel dim tx: {}", e))?;
            (id, None)
        }
    };

    if let Some(sid) = scalar_id {
        return Ok(sid);
    }

    let tbl = scalar_table(dim);
    let sid: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {tbl} (mark) VALUES (0) RETURNING id"
    )))
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| format!("insert scalar dim tx: {}", e))?;

    sqlx::query(sqlx::AssertSqlSafe(format!(
        r#"UPDATE isahl."zc_id_production_rr_storage" SET {col} = $1 WHERE id = $2"#
    )))
    .bind(sid)
    .bind(rel_id)
    .execute(&mut *conn)
    .await
    .map_err(|e| format!("backfill scalar dim tx: {}", e))?;

    Ok(sid)
}

/// 事务版：确保 (产品, 储元) 的 rr_storage 行 + 计数（qk_qty）标量行存在，返回标量行 id
/// （单计量兼容入口，委托 Qty 维度）
pub async fn ensure_stock_row_tx(
    conn: &mut sqlx::PgConnection,
    prod: i64,
    storage: i64,
) -> Result<i64, String> {
    ensure_stock_row_dim_tx(conn, prod, storage, StockDim::Qty).await
}

/// 事务版：对 (产品, 储元) 的指定计量维度累加 delta 并沿当前祖先链传播（物化 mark）
pub async fn apply_stock_delta_dim_tx(
    conn: &mut sqlx::PgConnection,
    prod: i64,
    storage: i64,
    dim: StockDim,
    delta: f64,
) -> Result<(), String> {
    if delta == 0.0 {
        return Ok(());
    }
    let mut targets = vec![storage];
    targets.extend(storage_ancestors_tx(conn, storage).await?);

    let tbl = scalar_table(dim);
    for s in targets {
        let sid = ensure_stock_row_dim_tx(conn, prod, s, dim).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            r#"UPDATE {tbl} SET mark = COALESCE(mark, 0) + $1 WHERE id = $2"#
        )))
        .bind(rust_decimal::Decimal::from_f64(delta).unwrap_or_default())
        .bind(sid)
        .execute(&mut *conn)
        .await
        .map_err(|e| format!("apply delta dim tx: {}", e))?;
    }
    Ok(())
}

/// 事务版：对 (产品, 储元) 累加 delta（计数维度，单计量兼容入口）
pub async fn apply_stock_delta_tx(
    conn: &mut sqlx::PgConnection,
    prod: i64,
    storage: i64,
    delta: f64,
) -> Result<(), String> {
    apply_stock_delta_dim_tx(conn, prod, storage, StockDim::Qty, delta).await
}

/// 事务版：当前物化 mark：(产品, 储元, 计量维度)
pub async fn stock_mark_dim_tx(
    conn: &mut sqlx::PgConnection,
    prod: i64,
    storage: i64,
    dim: StockDim,
) -> Result<f64, String> {
    let col = dim.column();
    let tbl = scalar_table(dim);
    let sql = format!(
        r#"SELECT CAST(COALESCE(sm.mark, 0) AS float8)
           FROM isahl."zc_id_production_rr_storage" r
           LEFT JOIN {tbl} sm ON sm.id = r.{col}
           WHERE r.ref_left = $1 AND r.ref_right = $2 AND r.deleted_at IS NULL"#
    );
    let mark: Option<f64> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(prod)
        .bind(storage)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| format!("stock_mark_dim_tx: {}", e))?;
    Ok(mark.unwrap_or(0.0))
}

/// 事务版：当前物化 mark：(产品, 储元) 计数维度（单计量兼容入口）
pub async fn stock_mark_tx(
    conn: &mut sqlx::PgConnection,
    prod: i64,
    storage: i64,
) -> Result<f64, String> {
    stock_mark_dim_tx(conn, prod, storage, StockDim::Qty).await
}

/// 事务版：读标量行真值（mark）——同 scalar_mark，经父表 zc_id_scale 解析
async fn scalar_mark_tx(conn: &mut sqlx::PgConnection, scalar_id: i64) -> Result<f64, String> {
    let mark: Option<f64> = sqlx::query_scalar(
        r#"SELECT CAST(COALESCE(mark, 0) AS float8) FROM isahl."zc_id_scale" WHERE id = $1"#,
    )
    .bind(scalar_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|e| format!("scalar_mark_tx: {}", e))?;
    Ok(mark.unwrap_or(0.0))
}

/// 事务版：确保数值对应的标量行存在（find_or_create by mark），返回标量 ID
async fn ensure_scalar_tx(conn: &mut sqlx::PgConnection, value: f64) -> Result<i64, String> {
    let existing: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_scal-common" WHERE mark = $1 ORDER BY id LIMIT 1"#,
    )
    .bind(value)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|e| format!("ensure_scalar find tx: {}", e))?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (mark) VALUES ($1) RETURNING id"#,
    )
    .bind(value)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| format!("ensure_scalar insert tx: {}", e))?;
    Ok(id)
}

/// 事务版：Voucher 净变（old → 冲销，new → 应用）；语义与 apply_voucher 一致，
/// 在调用方事务内执行（凭证行与物化原子）。
pub async fn apply_voucher_tx(
    conn: &mut sqlx::PgConnection,
    old: Option<&HashMap<String, Value>>,
    new: Option<&HashMap<String, Value>>,
) -> Result<(), String> {
    if let Some(old) = old {
        if let Some(prod) = get_id(old, "fk_production") {
            let income = match get_id(old, "qk_income").or_else(|| get_id(old, "qk_qty")) {
                Some(sid) => scalar_mark_tx(conn, sid).await?,
                None => 0.0,
            };
            let outgo = match get_id(old, "qk_outgo").or_else(|| get_id(old, "qk_qty")) {
                Some(sid) => scalar_mark_tx(conn, sid).await?,
                None => 0.0,
            };
            if let Some(obj) = get_id(old, "fk_obj-storage") {
                apply_stock_delta_tx(conn, prod, obj, -income).await?;
            }
            if let Some(subj) = get_id(old, "fk_subj-storage") {
                apply_stock_delta_tx(conn, prod, subj, outgo).await?;
            }
        }
    }
    if let Some(new) = new {
        if let Some(prod) = get_id(new, "fk_production") {
            let income = match get_id(new, "qk_income").or_else(|| get_id(new, "qk_qty")) {
                Some(sid) => scalar_mark_tx(conn, sid).await?,
                None => 0.0,
            };
            let outgo = match get_id(new, "qk_outgo").or_else(|| get_id(new, "qk_qty")) {
                Some(sid) => scalar_mark_tx(conn, sid).await?,
                None => 0.0,
            };
            if let Some(obj) = get_id(new, "fk_obj-storage") {
                apply_stock_delta_tx(conn, prod, obj, income).await?;
            }
            if let Some(subj) = get_id(new, "fk_subj-storage") {
                apply_stock_delta_tx(conn, prod, subj, -outgo).await?;
            }
            if let Some(id) = get_id(new, "id") {
                let main = get_id(new, "fk_obj-storage").or_else(|| get_id(new, "fk_subj-storage"));
                if let Some(storage) = main {
                    let after = stock_mark_tx(conn, prod, storage).await?;
                    // fix-wz-capacity-calculation：余额回填表感知（sto/com/tsp/slf 四族）
                    backfill_voucher_balance_tx(conn, prod, storage, id, income, outgo, after)
                        .await?;
                }
            }
        }
    }
    Ok(())
}

/// 凭证余额回填目标表探测（fix-wz-capacity-calculation：余额链表感知）——
/// 返回持有该凭证 id 的凭证族表名（静态白名单内动态选择，无注入面）。
/// 此前硬编码 stat-sto-voucher：对 com/tsp/slf 凭证恒 0 行匹配，余额链静默失明。
async fn voucher_balance_table_tx(
    conn: &mut sqlx::PgConnection,
    id: i64,
) -> Result<Option<&'static str>, String> {
    const VOUCHER_TABLES: [&str; 4] = [
        "zc_id_stat-sto-voucher",
        "zc_id_stat-com-voucher",
        "zc_id_stat-tsp-voucher",
        "zc_id_stat-slf-voucher",
    ];
    for table in VOUCHER_TABLES {
        let hit: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            r#"SELECT EXISTS (SELECT 1 FROM isahl."{table}" WHERE id = $1)"#
        )))
        .bind(id)
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| format!("voucher table probe: {e}"))?;
        if hit {
            return Ok(Some(table));
        }
    }
    Ok(None)
}

/// 余额链回填（表感知）：qk_pre_balance/qk_balance 写回凭证所在表；链首判定同表。
/// 期初余额语义（账本）：链起点（该 prod+storage 无前置凭证）期初=0；
/// 非首笔 before = after − income + outgo（反推=前笔期末）。
async fn backfill_voucher_balance_tx(
    conn: &mut sqlx::PgConnection,
    _prod: i64,
    _storage: i64,
    voucher_id: i64,
    income: f64,
    outgo: f64,
    after: f64,
) -> Result<(), String> {
    let Some(table) = voucher_balance_table_tx(conn, voucher_id).await? else {
        return Ok(()); // 凭证 id 不在四族表——诚实跳过，不伪造余额
    };
    // fix-wz-capacity-concurrency：期初统一反推 before = after − income + outgo——
    // 原「链起点期初=0」是纯凭证世界（库存从 0 起链）的约定；容量池存在未凭证化
    // 基线（种子/手工上架）时首个凭证期初被错记为 0（pre/bal 对内部不一致）。
    // 反推式在两种情形下均正确：空基线时 after−delta 与 0 分支同值。
    let before = after - income + outgo;
    let pre_id = ensure_scalar_tx(conn, before).await?;
    let bal_id = ensure_scalar_tx(conn, after).await?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        r#"UPDATE isahl."{table}"
           SET qk_pre_balance = $1, qk_balance = $2, updated_at = NOW()
           WHERE id = $3"#
    )))
    .bind(pre_id)
    .bind(bal_id)
    .bind(voucher_id)
    .execute(&mut *conn)
    .await
    .map_err(|e| format!("balance backfill tx: {e}"))?;
    Ok(())
}

/// 嵌套置入/取出 rollup：before/after 状态迁移（行存在=置入；删除/软删=取出）
pub async fn apply_nest(
    pool: &PgPool,
    old: Option<&HashMap<String, Value>>,
    new: Option<&HashMap<String, Value>>,
) -> Result<(), String> {
    let (old_parent, old_child, old_deleted) = match old {
        Some(o) => (
            get_id(o, "ref_left"),
            get_id(o, "ref_right"),
            o.get("deleted_at").map(|v| !v.is_null()).unwrap_or(false),
        ),
        None => (None, None, false),
    };
    let (new_parent, new_child, new_id) = match new {
        Some(n) => (
            get_id(n, "ref_left"),
            get_id(n, "ref_right"),
            get_id(n, "id"),
        ),
        None => (None, None, None),
    };

    let old_before = old_parent.is_some() && old_child.is_some() && !old_deleted;
    let old_after = match (old_parent, old_child) {
        (Some(p), Some(c)) => is_currently_nested(pool, p, c).await?,
        _ => false,
    };
    let new_before = match (new_parent, new_child, new_id) {
        (Some(p), Some(c), Some(id)) => is_currently_nested_excl(pool, p, c, id).await?,
        _ => false,
    };
    let new_after = match (new_parent, new_child) {
        (Some(p), Some(c)) => is_currently_nested(pool, p, c).await?,
        _ => false,
    };

    if old_before && !old_after {
        if let (Some(parent), Some(child)) = (old_parent, old_child) {
            for (prod, total) in products_with_stock(pool, child).await? {
                apply_stock_delta(pool, prod, parent, -total).await?;
            }
        }
    }
    if !new_before && new_after {
        if let (Some(parent), Some(child)) = (new_parent, new_child) {
            for (prod, total) in products_with_stock(pool, child).await? {
                apply_stock_delta(pool, prod, parent, total).await?;
            }
        }
    }
    Ok(())
}

/// 嵌套环/冗余校验（写入前调用；返回 Err 阻断写入）
pub async fn validate_nest(
    pool: &PgPool,
    new: Option<&HashMap<String, Value>>,
) -> Result<(), String> {
    let new = match new {
        Some(n) => n,
        None => return Ok(()),
    };
    let parent = match get_id(new, "ref_left") {
        Some(p) => p,
        None => return Ok(()),
    };
    let child = match get_id(new, "ref_right") {
        Some(c) => c,
        None => return Ok(()),
    };
    if parent == child {
        return Err("storage nest: ref_left cannot equal ref_right".to_string());
    }
    // 检查 A：新父的祖先链含新子（真环）
    for anc in storage_ancestors(pool, parent).await? {
        if anc == child {
            return Err(format!(
                "storage nest cycle detected (child {} under parent {})",
                child, parent
            ));
        }
    }
    // 检查 B：新父是新子的非直接祖先（冗余置入防双重 rollup）
    if !is_currently_nested(pool, parent, child).await? {
        for anc in storage_ancestors(pool, child).await? {
            if anc == parent {
                return Err(format!(
                    "storage nest redundant containment (child {} already under parent {})",
                    child, parent
                ));
            }
        }
    }
    Ok(())
}

/// 从记录取 id 字段（兼容 number 与 string 序列化）
fn get_id(record: &HashMap<String, Value>, field: &str) -> Option<i64> {
    record.get(field).and_then(|v| match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

// ============================================
// isahl.mv_inventory 自检自愈（用户定稿：视图落地 = Rust 自愈，非手工 DDL）
// ============================================

/// 自检自愈：确保 `isahl.mv_inventory` 物化视图存在（启动时调用，幂等）。
///
/// 自检：`pg_matviews` 探测 `isahl.mv_inventory`——已存在直接返回 Ok（零副作用）。
/// 自愈：基表 `zc_id_production_rr_storage` 存在时执行内嵌幂等 DDL
/// （`MV_INVENTORY_DDL`：CREATE MATERIALIZED VIEW IF NOT EXISTS + 索引）+ 初始 REFRESH；
/// 基表缺失（pre/prod 旧模型无此表）降级 `log::warn` 返回 Ok——不阻断启动，
/// 视图留待模型对齐后由自愈自动落地。
/// 失败返回 Err（由调用方 fail-fast 或周期重试兜底，仿 evo_agent::ensure_schema）。
///
/// DDL 为本模块编译期常量数组（非用户输入）——`AssertSqlSafe` 审计标注放行
/// （先例：`executor.rs` SideEffect::RawSql 同模式）。
pub async fn ensure_mv_inventory(pool: &PgPool) -> Result<(), String> {
    // 自检：视图存在且定义含全部期望列（capacity/qty/unit/variance）→ 幂等返回；
    // 视图存在但缺列（定义漂移，如 capacity 列加入）→ DROP 后重建（自愈升级）
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_matviews WHERE schemaname = 'isahl' AND matviewname = 'mv_inventory')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("mv_inventory 自检失败: {e}"))?;
    if exists {
        let has_multi_metric: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
             WHERE c.relname = 'mv_inventory' AND a.attname = 'amount' \
               AND a.attnum > 0 AND NOT a.attisdropped)",
        )
        .fetch_one(pool)
        .await
        .map_err(|e| format!("mv_inventory 列校验失败: {e}"))?;
        if has_multi_metric {
            // 定义就绪（含多计量列）——幂等返回前补一次刷新（启动自愈：覆盖快照滞后
            // ——reset/重启后视图存在但数据为旧时点或空）。
            // 批注 555ca3ab 复现链：schema 重放仅建 mv 不填充（ispopulated=false）时，
            // CONCURRENTLY 报「不能用于物化视图未被产生之前」且永不自行恢复（死锁）——
            // 未填充（f）先做非并发 REFRESH（初始填充），已填充（t）才 CONCURRENTLY。
            let populated: bool = sqlx::query_scalar(
                "SELECT ispopulated FROM pg_matviews WHERE schemaname = 'isahl' AND matviewname = 'mv_inventory'",
            )
            .fetch_one(pool)
            .await
            .unwrap_or(false);
            if populated {
                if let Err(e) =
                    sqlx::query("REFRESH MATERIALIZED VIEW CONCURRENTLY isahl.mv_inventory")
                        .execute(pool)
                        .await
                {
                    log::warn!(
                        "isahl.mv_inventory 启动 CONCURRENTLY 刷新失败（周期任务兜底）: {e}"
                    );
                }
            } else if let Err(e) = sqlx::query("REFRESH MATERIALIZED VIEW isahl.mv_inventory")
                .execute(pool)
                .await
            {
                log::warn!("isahl.mv_inventory 启动初始 REFRESH 失败（周期任务兜底）: {e}");
            }
            return Ok(()); // 定义就绪（含多计量列）——幂等返回
        }
        // 定义漂移：旧版视图缺 amount 列（单计量）→ DROP 重建（后续走自愈 DDL）
        log::warn!("isahl.mv_inventory 定义漂移（缺 amount 多计量列）——重建视图");
        sqlx::query(sqlx::AssertSqlSafe(
            "DROP MATERIALIZED VIEW IF EXISTS isahl.mv_inventory CASCADE",
        ))
        .execute(pool)
        .await
        .map_err(|e| format!("mv_inventory 漂移重建 DROP 失败: {e}"))?;
    }

    // 基表探测：pre/prod 无 zc_id_production_rr_storage → 降级跳过（不阻断启动）
    let base: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = 'isahl' AND table_name = 'zc_id_production_rr_storage')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("mv_inventory 基表探测失败: {e}"))?;
    if !base {
        log::warn!(
            "isahl.zc_id_production_rr_storage 不存在——mv_inventory 自愈跳过（环境模型未含库存基表）"
        );
        return Ok(());
    }

    // 自愈：执行内嵌 DDL（编译期常量数组）
    for &ddl in MV_INVENTORY_DDL {
        sqlx::query(sqlx::AssertSqlSafe(ddl))
            .execute(pool)
            .await
            .map_err(|e| format!("mv_inventory 自愈 DDL 失败: {e}"))?;
    }
    // 初始 REFRESH（空视图或存量数据物化）
    sqlx::query(sqlx::AssertSqlSafe(
        "REFRESH MATERIALIZED VIEW isahl.mv_inventory",
    ))
    .execute(pool)
    .await
    .map_err(|e| format!("mv_inventory 初始 REFRESH 失败: {e}"))?;
    log::info!("isahl.mv_inventory 物化视图已自愈创建（DDL + 索引 + 初始 REFRESH）");
    Ok(())
}

// ═══════════════════════════════════════════════════════
// 库存判定守卫（fix-wz-capacity-inventory-guard）
// 锁内链尾余额判定 + 边界校验 + code 幂等退化（外源/断链凭证）
// ═══════════════════════════════════════════════════════

/// 守卫应用结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoucherApply {
    /// 凭证已物化（判定通过）
    Applied,
    /// 同 code 凭证已存在——幂等跳过（未物化）
    SkippedDuplicate,
}

/// 守卫原语错误：区分业务性拒绝（边界越界）与内部失败
#[derive(Debug, Clone, PartialEq)]
pub enum GuardError {
    /// 边界拒绝（after 越界）——业务性失败（超卖/重复回补），调用方应映射为业务错误
    BoundRejected {
        product: i64,
        storage: i64,
        after: f64,
        min: Option<f64>,
        max: Option<f64>,
    },
    /// 数据库/内部失败
    Db(String),
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuardError::BoundRejected {
                product,
                storage,
                after,
                min,
                max,
            } => write!(
                f,
                "库存边界拒绝: (product={product}, storage={storage}) after={after} min={:?} max={:?}",
                min, max
            ),
            GuardError::Db(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GuardError {}

/// 从记录取 f64 守卫参数（`__min`/`__max`；兼容 number 与 numeric string）
fn get_f64(record: &HashMap<String, Value>, field: &str) -> Option<f64> {
    record.get(field).and_then(|v| match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

/// 从记录取字符串守卫参数（`__code`）
fn get_str(record: &HashMap<String, Value>, field: &str) -> Option<String> {
    record.get(field).and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

/// 库存判定守卫：锁内链尾余额判定 + 边界校验 + code 幂等退化。
///
/// `rec` 约定键（均缺省可选；无 `__code` 时行为与 `apply_voucher_tx` 完全一致）：
/// - `__code`: 凭证确定性 code。同 code 凭证已存在 → `SkippedDuplicate`（幂等跳过，不物化）。
/// - `__min` / `__max`: 边界校验（after = 当前值 ± 本凭证净变；越界 `GuardError::BoundRejected`）。
///   **回补路径（IN，如签收/取消回补）MUST NOT 传 `__max`**——回补是修正语义（把已扣减量还回），
///   被上界拒绝会阻断业务（货已送达却签收失败）；账漂移由净额核算/对账暴露（fix-tms-voucher-ledger-guards）。
///   `__max` 仅用于扣减侧（下单/出库）防超容。
///
/// 判定顺序（同一 FOR UPDATE 容量行锁内，与物化串行）：
/// 1. code 幂等检查（同 code 非本次凭证已存在 → 重放/并发重提幂等跳过）；
/// 2. 链尾判定：该 (product, storage) 最近 `qk_balance IS NOT NULL` 凭证余额为当前值；
///    链尾不可用（外源导入/断链：全 NULL 或无凭证）→ 物化 mark（COALESCE 0 兜底）为当前值；
/// 3. 边界校验（`__min`/`__max` 越界 → BoundRejected，调用方事务回滚，本凭证不落库）；
/// 4. 物化 + balance 回填（复用 `apply_voucher_tx`）。
///
/// 跨 action 双回补（如围栏入库后签收释放，code 不同）由边界校验拦截：链尾当前值已含
/// 首次回补，第二次 after 超 `__max` 拒绝。
pub async fn apply_guarded_voucher_tx(
    conn: &mut sqlx::PgConnection,
    rec: &HashMap<String, Value>,
) -> Result<VoucherApply, GuardError> {
    // 幂等索引惰性自愈（进程内 once，fix-dispatch-inventory-reconciliation D6）：
    // 不依赖 Gateway 启动时序——reset-db 重放后首个守卫写路径补齐索引，杜绝 42P10。
    if let Err(e) = ensure_voucher_idempotency_tx(conn).await {
        log::warn!("voucher idempotency 惰性自愈失败（降级继续，下次写路径重试）: {e}");
    }
    let Some(code) = get_str(rec, "__code") else {
        // 无守卫参数 → 纯物化（存量语义零变化）
        apply_voucher_tx(conn, None, Some(rec))
            .await
            .map_err(GuardError::Db)?;
        return Ok(VoucherApply::Applied);
    };
    let min = get_f64(rec, "__min");
    let max = get_f64(rec, "__max");

    let prod = get_id(rec, "fk_production")
        .ok_or_else(|| GuardError::Db("guarded voucher: fk_production 缺失".to_string()))?;
    let storage = get_id(rec, "fk_obj-storage")
        .or_else(|| get_id(rec, "fk_subj-storage"))
        .ok_or_else(|| {
            GuardError::Db(
                "guarded voucher: 主储元（fk_obj-storage/fk_subj-storage）缺失".to_string(),
            )
        })?;
    let income = match get_id(rec, "qk_income").or_else(|| get_id(rec, "qk_qty")) {
        Some(sid) => scalar_mark_tx(conn, sid).await.map_err(GuardError::Db)?,
        None => 0.0,
    };
    let outgo = match get_id(rec, "qk_outgo").or_else(|| get_id(rec, "qk_qty")) {
        Some(sid) => scalar_mark_tx(conn, sid).await.map_err(GuardError::Db)?,
        None => 0.0,
    };
    let delta = income - outgo;
    let self_id = get_id(rec, "id");

    // 1. 锁容量行（与物化同锁：串行化同 (product, storage) 的判定+物化）
    ensure_stock_row_dim_tx(conn, prod, storage, StockDim::Qty)
        .await
        .map_err(GuardError::Db)?;
    // fix-wz-capacity-concurrency：判定侧表跟随——dup/软删/链尾三处查询此前写死
    // stat-sto-voucher，对 com/tsp/slf 凭证族恒空转（dup 仅靠 INSERT 唯一索引兜底、
    // 软删恒 no-op、链尾恒 None 退物化 mark）。按凭证 id 探测族表，缺省回落 sto。
    let voucher_table: &str = match self_id {
        Some(id) => voucher_balance_table_tx(conn, id)
            .await
            .map_err(GuardError::Db)?
            .unwrap_or("zc_id_stat-sto-voucher"),
        None => "zc_id_stat-sto-voucher",
    };

    // 2. code 幂等检查（同 code 非本次凭证已存在 → 重放/并发重提，幂等跳过）
    let dup: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        r#"SELECT EXISTS (
               SELECT 1 FROM isahl."{voucher_table}" v
               WHERE v.code = $1 AND v.deleted_at IS NULL
                 AND ($2::bigint IS NULL OR v.id <> $2)
           )"#
    )))
    .bind(&code)
    .bind(self_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| GuardError::Db(format!("guarded voucher code 幂等检查失败: {e}")))?;
    if dup {
        // 本次凭证（调用方已 INSERT）为重复——软删清理（审计事实不留重复行）；
        // 调用方未传 id 时无法软删，仅返回跳过（凭证残留由调用方负责）。
        if let Some(self_id) = self_id {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                r#"UPDATE isahl."{voucher_table}"
                   SET deleted_at = NOW(), updated_at = NOW() WHERE id = $1"#
            )))
            .bind(self_id)
            .execute(&mut *conn)
            .await
            .map_err(|e| GuardError::Db(format!("guarded voucher 重复凭证软删失败: {e}")))?;
        }
        return Ok(VoucherApply::SkippedDuplicate);
    }

    // 3. 链尾余额判定（表感知 + OUT 方向入链：fk_obj-storage OR fk_subj-storage——
    // 单方向凭证（如 COM-OUT 只有 subj 储元）此前即使表对了也进不了链）
    let chain_tail: Option<f64> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        r#"SELECT CAST(COALESCE(sm.mark, 0) AS float8)
           FROM isahl."{voucher_table}" v
           JOIN isahl."zc_id_scale" sm ON sm.id = v.qk_balance
           WHERE v.fk_production = $1
             AND (v."fk_obj-storage" = $2 OR v."fk_subj-storage" = $2)
             AND v.qk_balance IS NOT NULL AND v.deleted_at IS NULL
           ORDER BY v.id DESC LIMIT 1"#
    )))
    .bind(prod)
    .bind(storage)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|e| GuardError::Db(format!("guarded voucher 链尾判定失败: {e}")))?;

    let cur = match chain_tail {
        Some(b) => b,
        // 退化分支：外源/断链（balance 全 NULL 或无凭证）→ 物化 mark 兜底
        None => stock_mark_tx(conn, prod, storage)
            .await
            .map_err(GuardError::Db)?,
    };
    let after = cur + delta;
    if let Some(mx) = max {
        if after > mx {
            return Err(GuardError::BoundRejected {
                product: prod,
                storage,
                after,
                min,
                max,
            });
        }
    }
    if let Some(mn) = min {
        if after < mn {
            return Err(GuardError::BoundRejected {
                product: prod,
                storage,
                after,
                min,
                max,
            });
        }
    }

    // 4. 物化 + balance 回填（复用既有原语，锁内重入无害）
    apply_voucher_tx(conn, None, Some(rec))
        .await
        .map_err(GuardError::Db)?;

    // 5. 库存物化视图刷新（校准：守卫判定与 UI 展示同源——mv_inventory 快照滞后
    //    会导致"判定可售但列表显示旧值"分叉；CONCURRENTLY 刷新，失败 warn 不阻断
    //    业务（周期自动刷新兜底，见 MvInventoryRefreshHandler））
    //    批注 555ca3ab：mv 首建/重建后 ispopulated=false——CONCURRENTLY 对空视图报错
    //    （"不能用于物化视图未被产生之前"）且该语句失败会 aborted 整个事务（错误被 catch
    //    后事务不可恢复，后续语句全报"当前事务被终止"）——刷新前检查 ispopulated，未填充跳过
    if let Ok(true) = sqlx::query_scalar::<_, bool>(
        r#"SELECT ispopulated FROM pg_matviews
           WHERE schemaname = 'isahl' AND matviewname = 'mv_inventory'"#,
    )
    .fetch_one(&mut *conn)
    .await
    {
        if let Err(e) = sqlx::query("REFRESH MATERIALIZED VIEW CONCURRENTLY isahl.mv_inventory")
            .execute(&mut *conn)
            .await
        {
            log::warn!("guarded voucher: mv_inventory CONCURRENTLY 刷新失败（周期任务兜底）: {e}");
        }
    } else {
        log::warn!(
            "guarded voucher: mv_inventory 未填充（ispopulated=false），跳过 CONCURRENTLY 刷新"
        );
    }
    Ok(VoucherApply::Applied)
}

/// 凭证幂等唯一索引名（与 `VOUCHER_IDEMPOTENCY_DDL` 一一对应，自愈探测用）
pub const VOUCHER_IDEMPOTENCY_INDEXES: &[&str] = &[
    "uq_zc_id_stat-tsp-voucher_code_active",
    "uq_zc_id_stat-com-voucher_code_active",
];

/// 凭证幂等唯一索引 DDL（编译期常量——AssertSqlSafe 审计标注放行，仿 MV_INVENTORY_DDL）
///
/// 部分唯一索引：`(code) WHERE deleted_at IS NULL`——同 code 未删除凭证唯一，
/// 并发重放/重提由 DB 硬拦截，配合守卫原语 code 幂等检查形成双保险。
pub const VOUCHER_IDEMPOTENCY_DDL: &[&str] = &[
    r#"CREATE UNIQUE INDEX IF NOT EXISTS "uq_zc_id_stat-tsp-voucher_code_active"
       ON isahl."zc_id_stat-tsp-voucher" (code) WHERE deleted_at IS NULL"#,
    r#"CREATE UNIQUE INDEX IF NOT EXISTS "uq_zc_id_stat-com-voucher_code_active"
       ON isahl."zc_id_stat-com-voucher" (code) WHERE deleted_at IS NULL"#,
];

/// 凭证幂等唯一索引自愈（仿 ensure_mv_inventory 模式）：探测已存在 → 幂等返回；
/// 缺失 → 执行内嵌幂等 DDL；DDL 失败 warn 降级返回 Ok（索引缺失仅弱化并发兜底，
/// 不阻断启动——业务判定仍由守卫原语边界校验兜底）。
pub async fn ensure_voucher_idempotency(pool: &PgPool) -> Result<(), String> {
    for (ddl, index_name) in VOUCHER_IDEMPOTENCY_DDL
        .iter()
        .zip(VOUCHER_IDEMPOTENCY_INDEXES.iter())
    {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = 'isahl' AND indexname = $1)",
        )
        .bind(index_name)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("voucher 幂等索引自检失败: {e}"))?;
        if exists {
            continue;
        }
        match sqlx::query(sqlx::AssertSqlSafe(*ddl)).execute(pool).await {
            Ok(_) => log::info!("voucher 幂等唯一索引已自愈创建: {index_name}"),
            Err(e) => log::warn!("voucher 幂等唯一索引创建失败（降级继续）: {e}"),
        }
    }
    Ok(())
}

/// 进程级惰性自愈标志：仅 ensure 成功后置位（失败 warn 降级、下次写路径重试）。
static VOUCHER_IDEMPOTENCY_ENSURED: AtomicBool = AtomicBool::new(false);

/// 连接内变体：在调用方事务内执行幂等唯一索引自愈（`ensure_voucher_idempotency`
/// 的 conn 版本——守卫原语只持有事务连接，不持有 pool）。
///
/// 注意：若外层事务随后回滚，索引随之消失且本进程内不再重试（标志已置位）——
/// 该窗口退化为 Gateway 启动自愈兜底，与既有语义等价（D6 已声明）。
pub async fn ensure_voucher_idempotency_tx(conn: &mut sqlx::PgConnection) -> Result<(), String> {
    if VOUCHER_IDEMPOTENCY_ENSURED.load(Ordering::Relaxed) {
        return Ok(());
    }
    for ddl in VOUCHER_IDEMPOTENCY_DDL {
        sqlx::query(sqlx::AssertSqlSafe(*ddl))
            .execute(&mut *conn)
            .await
            .map_err(|e| format!("voucher idempotency ddl: {e}"))?;
    }
    VOUCHER_IDEMPOTENCY_ENSURED.store(true, Ordering::Relaxed);
    Ok(())
}
