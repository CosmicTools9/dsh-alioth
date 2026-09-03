//! trigger-registry 集成测试：isahl.mv_inventory 物化视图自检自愈
//!
//! 验证 `ensure_mv_inventory`（用户 2026-08-07 定稿：视图落地 = Rust 自愈）：
//! 1. 缺失 → 内嵌 DDL 创建视图 + 索引 + 初始 REFRESH
//! 2. 再次调用 → 幂等零副作用
//! 3. 视图可查询（与 stock_stat 语义一致：qty 来自标量 mark 真值）
//!
//! 需要 DATABASE_URL 指向 aliothstudio_test（`#[ignore]`，仿 evo_agent 先例）。

use common::testing::connect_test_db;

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn ensure_mv_inventory_self_heals() {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    let pool = match url {
        Some(_) => connect_test_db().await,
        None => {
            eprintln!("skipped: DATABASE_URL 未设置");
            return;
        }
    };

    // 基表存在性前置：视图依赖 zc_id_production_rr_storage（dev/test 基准模型）
    let base: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
         WHERE table_schema = 'isahl' AND table_name = 'zc_id_production_rr_storage')",
    )
    .fetch_one(&pool)
    .await
    .expect("基表探测");
    if !base {
        eprintln!("skipped: test 库无 zc_id_production_rr_storage（基表缺失降级场景）");
        return;
    }

    // 准备：若视图已存在则 DROP（模拟缺失状态）；幂等重建由 ensure 完成
    sqlx::query("DROP MATERIALIZED VIEW IF EXISTS isahl.mv_inventory CASCADE")
        .execute(&pool)
        .await
        .expect("清理视图");

    // 自愈：首次调用创建视图 + 索引 + 初始 REFRESH
    trigger_registry::stock_materialization::ensure_mv_inventory(&pool)
        .await
        .expect("ensure_mv_inventory 应自愈创建成功");

    // 验证 1：视图存在于 pg_matviews
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_matviews \
         WHERE schemaname = 'isahl' AND matviewname = 'mv_inventory')",
    )
    .fetch_one(&pool)
    .await
    .expect("视图存在性查询");
    assert!(exists, "自愈后视图应就绪");

    // 验证 2：视图可查询（结构正确——列齐备；空集时查询本身不报错）
    let row: Option<(i64, i64, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT production_id, storage_id, qty::float8, capacity::float8 FROM isahl.mv_inventory LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .expect("视图应可查询（无数据时返回 None 而非报错）");
    let _ = row; // 空集合法——证明视图可执行查询且含 capacity 列

    // 验证 2b：capacity 列存在（定义漂移校验的前提）
    let has_cap: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
         WHERE c.relname = 'mv_inventory' AND a.attname = 'capacity' \
           AND a.attnum > 0 AND NOT a.attisdropped)",
    )
    .fetch_one(&pool)
    .await
    .expect("capacity 列查询");
    assert!(has_cap, "自愈后视图应含 capacity 列");

    // 验证 3：幂等——再次调用零副作用
    trigger_registry::stock_materialization::ensure_mv_inventory(&pool)
        .await
        .expect("ensure_mv_inventory 应幂等返回 Ok");
    let exists_after: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_matviews \
         WHERE schemaname = 'isahl' AND matviewname = 'mv_inventory')",
    )
    .fetch_one(&pool)
    .await
    .expect("视图存在性复查");
    assert!(exists_after, "幂等调用后视图仍就绪");
}

/// 多计量物化（add-inventory-multi-metric）：(商品, 储元) 行四计量标量独立物化，
/// mv_inventory 输出 qty/w_qty/v_qty/amount 四列（REFRESH 后）。
#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn mv_inventory_multi_metric_materializes() {
    use trigger_registry::stock_materialization::{ensure_mv_inventory, StockDim};

    let pool = connect_test_db().await;
    ensure_mv_inventory(&pool).await.expect("视图就绪");

    let mut tx = pool.begin().await.expect("begin tx");

    // fixture：rr_storage 行 + 四计量标量（scal-common ×3 + scal-amount ×1）
    let qty_s: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, 'MM', $2::numeric, 1) RETURNING id"#,
    )
    .bind("MM-QTY")
    .bind(10)
    .fetch_one(&mut *tx)
    .await
    .expect("qty");
    let w_s: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, 'MM', $2::numeric, 1) RETURNING id"#,
    )
    .bind("MM-W")
    .bind(20)
    .fetch_one(&mut *tx)
    .await
    .expect("w");
    let v_s: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, 'MM', $2::numeric, 1) RETURNING id"#,
    )
    .bind("MM-V")
    .bind(30)
    .fetch_one(&mut *tx)
    .await
    .expect("v");
    let amt_s: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-amount" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), 'MM-AMT', 'MM', 40::numeric, 1) RETURNING id"#,
    )
    .fetch_one(&mut *tx)
    .await
    .expect("amount");
    let row_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_production_rr_storage"
           (id, code, notice, ref_left, ref_right, qk_qty, qk_w_qty, qk_v_qty, qk_amount, created_by_id)
           VALUES (isahl.gen_next_zuid(), 'MM-ROW', 'MM多计量', 900001, 900002, $1, $2, $3, $4, 1)
           RETURNING id"#,
    )
    .bind(qty_s).bind(w_s).bind(v_s).bind(amt_s)
    .fetch_one(&mut *tx).await.expect("row");
    let _ = row_id;

    // 维度原语独立累加（+5 计数 / +7 重量），互不干扰
    trigger_registry::stock_materialization::apply_stock_delta_dim_tx(
        &mut tx,
        900001,
        900002,
        StockDim::Qty,
        5.0,
    )
    .await
    .expect("qty delta");
    trigger_registry::stock_materialization::apply_stock_delta_dim_tx(
        &mut tx,
        900001,
        900002,
        StockDim::Weight,
        7.0,
    )
    .await
    .expect("weight delta");

    // 原子性：金额维度累加后回滚 → 全部回滚（qty/w 也回滚）
    trigger_registry::stock_materialization::apply_stock_delta_dim_tx(
        &mut tx,
        900001,
        900002,
        StockDim::Amount,
        100.0,
    )
    .await
    .expect("amount delta");
    tx.rollback().await.expect("rollback");

    // 回滚后无残留（无物化行）
    let leftover: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM isahl.\"zc_id_production_rr_storage\" WHERE code = 'MM-ROW'",
    )
    .fetch_one(&pool)
    .await
    .expect("leftover");
    assert_eq!(leftover.0, 0, "事务回滚后不得残留物化行");

    // 重新提交路径：独立事务内四维度累加后 REFRESH 读视图
    let mut tx2 = pool.begin().await.expect("begin tx2");
    let qty_s2: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, 'MM', $2::numeric, 1) RETURNING id"#,
    )
    .bind("MM-QTY2")
    .bind(10)
    .fetch_one(&mut *tx2)
    .await
    .expect("qty2");
    let w_s2: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, 'MM', $2::numeric, 1) RETURNING id"#,
    )
    .bind("MM-W2")
    .bind(20)
    .fetch_one(&mut *tx2)
    .await
    .expect("w2");
    let v_s2: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, 'MM', $2::numeric, 1) RETURNING id"#,
    )
    .bind("MM-V2")
    .bind(30)
    .fetch_one(&mut *tx2)
    .await
    .expect("v2");
    let amt_s2: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-amount" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), 'MM-AMT2', 'MM', 40::numeric, 1) RETURNING id"#,
    )
    .fetch_one(&mut *tx2)
    .await
    .expect("amt2");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_production_rr_storage"
           (id, code, notice, ref_left, ref_right, qk_qty, qk_w_qty, qk_v_qty, qk_amount, created_by_id)
           VALUES (isahl.gen_next_zuid(), 'MM-ROW2', 'MM多计量2', 900003, 900004, $1, $2, $3, $4, 1)"#,
    )
    .bind(qty_s2).bind(w_s2).bind(v_s2).bind(amt_s2)
    .execute(&mut *tx2).await.expect("row2");
    trigger_registry::stock_materialization::apply_stock_delta_dim_tx(
        &mut tx2,
        900003,
        900004,
        StockDim::Qty,
        5.0,
    )
    .await
    .expect("qty delta2");
    trigger_registry::stock_materialization::apply_stock_delta_dim_tx(
        &mut tx2,
        900003,
        900004,
        StockDim::Weight,
        7.0,
    )
    .await
    .expect("weight delta2");
    tx2.commit().await.expect("commit tx2");

    sqlx::query("REFRESH MATERIALIZED VIEW isahl.mv_inventory")
        .execute(&pool)
        .await
        .expect("refresh");

    let (qty, w, v, amt): (f64, f64, f64, f64) = sqlx::query_as(
        "SELECT qty::float8, w_qty::float8, v_qty::float8, amount::float8 \
         FROM isahl.mv_inventory WHERE production_id = 900003 AND storage_id = 900004",
    )
    .fetch_one(&pool)
    .await
    .expect("view row");
    assert!((qty - 15.0).abs() < 0.01, "qty 应 10+5=15，实际 {qty}");
    assert!((w - 27.0).abs() < 0.01, "w_qty 应 20+7=27，实际 {w}");
    assert!((v - 30.0).abs() < 0.01, "v_qty 应 30，实际 {v}");
    assert!((amt - 40.0).abs() < 0.01, "amount 应 40，实际 {amt}");

    // 清理
    for sql in [
        r#"DELETE FROM isahl."zc_id_production_rr_storage" WHERE code LIKE 'MM-ROW%'"#,
        r#"DELETE FROM isahl."zc_id_scal-common" WHERE code LIKE 'MM-%'"#,
        r#"DELETE FROM isahl."zc_id_scal-amount" WHERE code LIKE 'MM-%'"#,
    ] {
        sqlx::query(sql).execute(&pool).await.expect("cleanup");
    }
    sqlx::query("REFRESH MATERIALIZED VIEW isahl.mv_inventory")
        .execute(&pool)
        .await
        .expect("refresh after cleanup");
}
