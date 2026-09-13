//! trigger-registry 集成测试：isahl.mv_title_ownership 物化视图自检自愈与属权语义
//!
//! 验证 `ensure_mv_title_ownership`（add-title-ownership-mv，2026-09-07 用户定稿：
//! 主体×物属权关系快速检索载体）：
//! 1. 缺失 → 内嵌 DDL 创建视图 + 索引 + 初始 REFRESH
//! 2. 再次调用 → 幂等零副作用
//! 3. 属权语义：net_qty = Σ(income 标量真值) − Σ(outgo 标量真值)；
//!    无主体（fk_subject 空）凭证不进入视图
//!
//! 需要 DATABASE_URL 指向 aliothstudio_test（`#[ignore]`，仿 mv_inventory 先例）。

use common::testing::connect_test_db;

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn ensure_mv_title_ownership_self_heals() {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    let pool = match url {
        Some(_) => connect_test_db().await,
        None => {
            eprintln!("skipped: DATABASE_URL 未设置");
            return;
        }
    };

    // 基表存在性前置：视图依赖 zc_id_stat-sto-voucher（dev/test 基准模型）
    let base: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
         WHERE table_schema = 'isahl' AND table_name = 'zc_id_stat-sto-voucher')",
    )
    .fetch_one(&pool)
    .await
    .expect("基表探测");
    if !base {
        eprintln!("skipped: test 库无 zc_id_stat-sto-voucher（基表缺失降级场景）");
        return;
    }

    // 准备：若视图已存在则 DROP（模拟缺失状态）；幂等重建由 ensure 完成
    sqlx::query("DROP MATERIALIZED VIEW IF EXISTS isahl.mv_title_ownership CASCADE")
        .execute(&pool)
        .await
        .expect("清理视图");

    // 自愈：首次调用创建视图 + 索引 + 初始 REFRESH
    trigger_registry::stock_materialization::ensure_mv_title_ownership(&pool)
        .await
        .expect("ensure_mv_title_ownership 应自愈创建成功");

    // 验证 1：视图存在于 pg_matviews
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_matviews \
         WHERE schemaname = 'isahl' AND matviewname = 'mv_title_ownership')",
    )
    .fetch_one(&pool)
    .await
    .expect("视图存在性查询");
    assert!(exists, "自愈后视图应就绪");

    // 验证 2：视图可查询（结构正确——net_qty 列齐备；空集时查询本身不报错）
    let row: Option<(i64, i64, f64)> = sqlx::query_as(
        "SELECT subject_id, production_id, net_qty::float8 FROM isahl.mv_title_ownership LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .expect("视图应可查询（无数据时返回 None 而非报错）");
    let _ = row; // 空集合法——证明视图可执行查询且含 net_qty 列

    // 验证 3：幂等——再次调用零副作用
    trigger_registry::stock_materialization::ensure_mv_title_ownership(&pool)
        .await
        .expect("ensure_mv_title_ownership 应幂等返回 Ok");
    let exists_after: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_matviews \
         WHERE schemaname = 'isahl' AND matviewname = 'mv_title_ownership')",
    )
    .fetch_one(&pool)
    .await
    .expect("视图存在性复查");
    assert!(exists_after, "幂等调用后视图仍就绪");
}

/// 属权语义物化（add-title-ownership-mv）：(物权人, 物) 净属权 = Σincome − Σoutgo
/// （标量真值经父表 zc_id_scale 解析）；无主体凭证（纯库存位移）不进入视图。
#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn mv_title_ownership_aggregates_net_title() {
    use trigger_registry::stock_materialization::ensure_mv_title_ownership;

    let pool = connect_test_db().await;
    ensure_mv_title_ownership(&pool).await.expect("视图就绪");

    // fixture：标量（income 10 / outgo 4 / 无主体凭证 income 100）
    let in_s: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), 'TO-IN', 'TO', 10::numeric, 1) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("income 标量");
    let out_s: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), 'TO-OUT', 'TO', 4::numeric, 1) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("outgo 标量");
    let nosubj_s: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), 'TO-NOSUBJ', 'TO', 100::numeric, 1) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("无主体凭证标量");

    // 凭证 fixture 落仓储凭证叶表（§8.5 叶表插入纪律）——视图源为
    // zc_id_stat-sto-voucher 父表查询，叶表行经继承聚合可见（leaf-family 语义同验）
    // 类列形态 1（§4.3.3 首选）：_f_/_t_ 禁止字面量直写，由 dk_function.code
    // 前缀派生（lifecycle.rs derive_form_type）——fixture 只提供 dk_* 派生源
    // 入库凭证：物权人 910001 拥有物 910002，初始物权 +10（单边 IN）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_stat-whs-voucher"
           (id, code, notice, qk_income, fk_subject, fk_production, dk_scene, dk_factor, dk_function, created_by_id)
           VALUES (isahl.gen_next_zuid(), 'TO-V-IN', 'TO 初始物权', $1, 910001, 910002,
                   (SELECT id FROM isahl.zc_id_scene LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code LIKE '↓\_%' LIMIT 1),
                   1)"#,
    )
    .bind(in_s)
    .execute(&pool)
    .await
    .expect("入库凭证");
    // 出库凭证：同一 (物权人, 物) 属权 −4（同样 dk_* 派生形态）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_stat-whs-voucher"
           (id, code, notice, qk_outgo, fk_subject, fk_production, dk_scene, dk_factor, dk_function, created_by_id)
           VALUES (isahl.gen_next_zuid(), 'TO-V-OUT', 'TO 属权转出', $1, 910001, 910002,
                   (SELECT id FROM isahl.zc_id_scene LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code LIKE '↓\_%' LIMIT 1),
                   1)"#,
    )
    .bind(out_s)
    .execute(&pool)
    .await
    .expect("出库凭证");
    // 无主体凭证：纯库存位移，不构成属权关系（物 910003；同样 dk_* 派生形态）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_stat-whs-voucher"
           (id, code, notice, qk_income, fk_production, dk_scene, dk_factor, dk_function, created_by_id)
           VALUES (isahl.gen_next_zuid(), 'TO-V-NOSUBJ', 'TO 无主体', $1, 910003,
                   (SELECT id FROM isahl.zc_id_scene LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code LIKE '↓\_%' LIMIT 1),
                   1)"#,
    )
    .bind(nosubj_s)
    .execute(&pool)
    .await
    .expect("无主体凭证");

    sqlx::query("REFRESH MATERIALIZED VIEW isahl.mv_title_ownership")
        .execute(&pool)
        .await
        .expect("refresh");

    // 断言 1：(910001, 910002) 净属权 10 − 4 = 6，凭证计数 2
    let (income, outgo, net, count): (f64, f64, f64, i64) = sqlx::query_as(
        "SELECT income_total::float8, outgo_total::float8, net_qty::float8, voucher_count \
         FROM isahl.mv_title_ownership WHERE subject_id = 910001 AND production_id = 910002",
    )
    .fetch_one(&pool)
    .await
    .expect("属权行");
    assert!(
        (income - 10.0).abs() < 0.01,
        "income_total 应 10，实际 {income}"
    );
    assert!((outgo - 4.0).abs() < 0.01, "outgo_total 应 4，实际 {outgo}");
    assert!((net - 6.0).abs() < 0.01, "net_qty 应 6，实际 {net}");
    assert_eq!(count, 2, "voucher_count 应 2");

    // 断言 2：无主体凭证不进入视图（物 910003 无任何属权行）
    let nosubj_rows: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM isahl.mv_title_ownership WHERE production_id = 910003",
    )
    .fetch_one(&pool)
    .await
    .expect("无主体凭证行数");
    assert_eq!(nosubj_rows.0, 0, "fk_subject 为空的凭证不得进入属权视图");

    // 清理
    for sql in [
        r#"DELETE FROM isahl."zc_id_stat-whs-voucher" WHERE code LIKE 'TO-V-%'"#,
        r#"DELETE FROM isahl."zc_id_scal-common" WHERE code LIKE 'TO-%'"#,
    ] {
        sqlx::query(sql).execute(&pool).await.expect("cleanup");
    }
    sqlx::query("REFRESH MATERIALIZED VIEW isahl.mv_title_ownership")
        .execute(&pool)
        .await
        .expect("refresh after cleanup");
}
