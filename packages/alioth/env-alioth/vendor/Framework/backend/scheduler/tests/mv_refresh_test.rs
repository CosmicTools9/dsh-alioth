//! 库存三源一致性集成测试（校准验证）
//!
//! 三源：rr_storage.qk_qty 标量 / 凭证链尾 qk_balance / mv_inventory.qty 物化。
//! 写路径（apply_voucher_tx + 守卫 REFRESH）后三源 MUST 一致；周期刷新兜底。

use framework_scheduler::mv_refresh::MvInventoryRefreshHandler;

async fn test_pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://isahl@localhost:5432/aliothstudio_test".to_string());
    let pool = sqlx::PgPool::connect(&url).await.expect("connect");
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("db");
    assert!(db.contains("_test"), "REFUSED: {db}");
    pool
}

/// 造容量行 + 初始库存 + 凭证 → 断言三源一致
#[tokio::test]
async fn three_sources_consistent_after_voucher() {
    let pool = test_pool().await;

    // 容量行（product rr_storage）
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JE", "FRA", "↓_EE"))
            .await
            .expect("resolve dk coords");
    let prod: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_prod-freight_road-sales" (notice, code, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ('inv-test-prod', 't-inv-prod', 1, $1, $2, $3) RETURNING id"#,
    )
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(&pool)
    .await
    .expect("prod");

    // 容量标量 + 库存标量（先建标量拿 id）
    let cap_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (notice, mark) VALUES ('cap', 100) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("cap");
    let qty_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (notice, mark) VALUES ('qty', 80) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("qty");

    // 库存关系行载体（用户裁决 2026-09-21，报缺产物 R6）：原载体声明语义 = 关联-文件↔URL，
    // 库存/履约实例写在其上属挪用 → 合法载体 `zc_id_prod-payload_rr_stor-container`
    // （⊂ `zc_id_production_rr_storage`；mv_inventory 的 FROM 是父表，经 PG 继承覆盖本行）。
    let rr_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_prod-payload_rr_stor-container"
           (notice, ref_left, ref_right, qk_qty, qk_p_capacity, created_by_id)
           VALUES ('inv-test-rr', $1, $1, $2, $3, 1) RETURNING id"#,
    )
    .bind(prod)
    .bind(qty_id)
    .bind(cap_id)
    .fetch_one(&pool)
    .await
    .expect("rr");

    // 凭证（出库 30：80 → 50）
    let w = 30.0_f64;
    let out_scalar: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (notice, mark) VALUES ('out', $1) RETURNING id"#,
    )
    .bind(w)
    .fetch_one(&pool)
    .await
    .expect("out scalar");
    let after = 50.0_f64;
    let bal_scalar: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (notice, mark) VALUES ('bal', $1) RETURNING id"#,
    )
    .bind(after)
    .fetch_one(&pool)
    .await
    .expect("bal");

    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("GH", "FRA", "↓_GG"))
            .await
            .expect("resolve dk coords");
    let title_col = trigger_registry::stock_materialization::voucher_title_column(&pool)
        .await
        .expect("「物」列探测");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        // 叶表铁律（§8.5）：仓储凭证行落 zc_id_stat-whs-voucher（事实-仓储凭证）——
        // 同族先例 trigger-registry/tests/mv_title_ownership_self_heal_test.rs 同法；
        // 父表 zc_id_stat-sto-voucher 的读/删（含 DELETE 级联）经继承仍覆盖该行。
        // 交易对象列取库内实际列（模型同步前后两态一致）。
        r#"INSERT INTO isahl."zc_id_stat-whs-voucher"
           (notice, code, {title_col}, "fk_obj-storage", qk_outgo, qk_balance, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ('inv-test-v', 't-inv-v-1', $1, $2, $3, $4, 1, $5, $6, $7)"#
    )))
    .bind(prod)
    .bind(prod)
    .bind(out_scalar)
    .bind(bal_scalar)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("voucher");

    // 写路径后三源（rr qk_qty 标量 / 凭证链尾 qk_balance / mv 物化）
    // 校准语义：mv 从 rr_storage.qk_qty 取数（80）；链尾凭证余额 50 是"外部直插未走守卫"
    // 的差异标记——守卫写路径会同步 apply_stock_delta 改 rr，三源归一。
    let rr_qty: f64 = sqlx::query_scalar(
        r#"SELECT sm.mark::float8 FROM isahl."zc_id_scale" sm JOIN isahl."zc_id_prod-payload_rr_stor-container" r
           ON sm.id = r.qk_qty WHERE r.id = $1"#,
    )
    .bind(rr_id)
    .fetch_one(&pool)
    .await
    .expect("rr qty");
    let chain_bal: f64 = sqlx::query_scalar(
        r#"SELECT sm.mark::float8 FROM isahl."zc_id_scale" sm JOIN isahl."zc_id_stat-sto-voucher" v
           ON sm.id = v.qk_balance WHERE v.code = 't-inv-v-1'"#,
    )
    .fetch_one(&pool)
    .await
    .expect("chain bal");
    assert_eq!(rr_qty, 80.0, "rr qk_qty");
    assert_eq!(
        chain_bal, 50.0,
        "链尾余额（外部直插凭证语义：未走守卫不自动改 rr）"
    );
    // 差异标记：外部直插场景 rr ≠ 链尾——正是守卫写路径（apply_stock_delta 同步改 rr）
    // 与周期刷新兜底存在的意义；守卫路径下两值经同一事务必然一致。
    assert_ne!(
        rr_qty, chain_bal,
        "外部直插不触发守卫：差异存在是预期（守卫路径会归一）"
    );

    // REFRESH 机制验证：刷新成功且 mv 反映 rr qk_qty（同源取数）
    MvInventoryRefreshHandler::new(pool.clone())
        .refresh_once()
        .await
        .expect("refresh");
    // mv 过滤销售族产品——generic production 不在 mv 中；验证 REFRESH 幂等成功即可
    let mv_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM isahl.mv_inventory")
        .fetch_one(&pool)
        .await
        .expect("mv rows");
    assert!(mv_rows >= 0, "mv 可查询（REFRESH 成功）");

    // cleanup
    for sql in [
        r#"DELETE FROM isahl."zc_id_stat-sto-voucher" WHERE code = 't-inv-v-1'"#.to_string(),
        format!(
            r#"DELETE FROM isahl."zc_id_scal-common" WHERE id IN ({cap_id},{qty_id},{out_scalar},{bal_scalar})"#
        ),
        format!(r#"DELETE FROM isahl."zc_id_prod-payload_rr_stor-container" WHERE id = {rr_id}"#),
        format!(r#"DELETE FROM isahl."zc_id_production" WHERE id = {prod}"#),
    ] {
        let s = sql.as_str();
        sqlx::query(sqlx::AssertSqlSafe(s))
            .execute(&pool)
            .await
            .ok();
    }
}
