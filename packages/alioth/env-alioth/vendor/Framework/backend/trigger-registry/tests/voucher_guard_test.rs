//! trigger-registry 集成测试：库存判定守卫原语（fix-wz-capacity-inventory-guard）
//!
//! 验证 `apply_guarded_voucher_tx` 与 `ensure_voucher_idempotency`：
//! 1. 链尾余额判定：OUT 防负库存（after < __min 拒绝）、IN 防重复回补（after > __max 拒绝）
//! 2. code 幂等退化：同 code 重放 → SkippedDuplicate（不物化、重复凭证软删）
//! 3. 外源 NULL 凭证：链尾跳过 NULL 凭证，退化物化 mark 兜底判定
//! 4. 并发同 code：FOR UPDATE 串行化 → 单凭证单物化
//! 5. 唯一索引自愈：ensure_voucher_idempotency 创建 + 幂等
//!
//! 需要 DATABASE_URL 指向 aliothstudio_test（`#[ignore]`，仿 mv_inventory_self_heal_test 先例）。

use common::testing::connect_test_db;
use std::collections::HashMap;
use trigger_registry::stock_materialization::{
    apply_guarded_voucher_tx, ensure_voucher_idempotency, stock_mark_tx, GuardError, VoucherApply,
};

/// 测试 fixture：容量池产品 + 线路储元 + 容量行（qk_p_capacity=cap, qk_qty=initial）
/// 返回 (pool_product, line_storage)
async fn seed_capacity_row(
    pool: &sqlx::PgPool,
    code_suffix: &str,
    cap: f64,
    initial: f64,
) -> (i64, i64) {
    let pool_product: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_prod-freight_road-sales"
           (id, code, notice, _t_, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, '范例', 1) RETURNING id"#,
    )
    .bind(format!("TEST-GUARD-POOL-{code_suffix}"))
    .bind(format!("测试容量池 {code_suffix}"))
    .fetch_one(pool)
    .await
    .expect("insert pool product");

    let line: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_stor-traffic_line"
           (id, code, notice, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1) RETURNING id"#,
    )
    .bind(format!("TEST-GUARD-LINE-{code_suffix}"))
    .bind(format!("测试线路 {code_suffix}"))
    .fetch_one(pool)
    .await
    .expect("insert line");

    // 容量标量（qk_p_capacity）与库存标量（qk_qty）
    let cap_scalar: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_scal-common" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, '总容量', $2::numeric, 1) RETURNING id"#,
    )
    .bind(format!("TEST-GUARD-CAP-{code_suffix}"))
    .bind(cap)
    .fetch_one(pool)
    .await
    .expect("insert cap scalar");
    let qty_scalar: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_scal-common" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, '可售', $2::numeric, 1) RETURNING id"#,
    )
    .bind(format!("TEST-GUARD-QTY-{code_suffix}"))
    .bind(initial)
    .fetch_one(pool)
    .await
    .expect("insert qty scalar");

    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_production_rr_storage"
           (id, code, notice, ref_left, ref_right, qk_p_capacity, qk_qty, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, '容量行', $2, $3, $4, $5, 1)"#,
    )
    .bind(format!("TEST-GUARD-ROW-{code_suffix}"))
    .bind(pool_product)
    .bind(line)
    .bind(cap_scalar)
    .bind(qty_scalar)
    .execute(pool)
    .await
    .expect("insert capacity row");

    (pool_product, line)
}

/// 构造守卫原语 rec（单边凭证：obj=线路储元；IN 用 qk_income，OUT 用 qk_outgo）
#[allow(clippy::too_many_arguments)]
fn guard_rec(
    voucher_id: i64,
    code: &str,
    pool_product: i64,
    line: i64,
    weight_scalar: i64,
    side: &str,
    min: Option<f64>,
    max: Option<f64>,
) -> HashMap<String, serde_json::Value> {
    let mut rec = HashMap::new();
    rec.insert("id".to_string(), serde_json::json!(voucher_id));
    rec.insert("fk_production".to_string(), serde_json::json!(pool_product));
    match side {
        // 出库位 = fk_subj-storage（与 WZ 下单 OUT 同语义）；入库位 = fk_obj-storage
        "OUT" => {
            rec.insert("fk_subj-storage".to_string(), serde_json::json!(line));
            rec.insert("qk_outgo".to_string(), serde_json::json!(weight_scalar));
        }
        _ => {
            rec.insert("fk_obj-storage".to_string(), serde_json::json!(line));
            rec.insert("qk_income".to_string(), serde_json::json!(weight_scalar));
        }
    }
    rec.insert("__code".to_string(), serde_json::json!(code));
    if let Some(mn) = min {
        rec.insert("__min".to_string(), serde_json::json!(mn));
    }
    if let Some(mx) = max {
        rec.insert("__max".to_string(), serde_json::json!(mx));
    }
    rec
}

/// 在事务内：INSERT 凭证（ON CONFLICT 幂等）→ 命中则调守卫原语
/// 返回 (voucher_id 或 None=唯一索引拦截, 原语结果或 None=未走原语)
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
async fn insert_and_guard(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    code: &str,
    pool_product: i64,
    line: i64,
    weight: f64,
    side: &str,
    min: Option<f64>,
    max: Option<f64>,
    comment_action: &str,
) -> (Option<i64>, Option<Result<VoucherApply, GuardError>>) {
    let weight_scalar: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_scal-common" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3::numeric, 1) RETURNING id"#,
    )
    .bind(format!(
        "WT-GUARD-{}-{}",
        code,
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ))
    .bind(format!("守卫测试 {}吨", weight))
    .bind(weight)
    .fetch_one(&mut **tx)
    .await
    .expect("insert weight scalar");

    let comments = serde_json::json!({
        "type": "inbound",
        "action": comment_action,
        "direction": side,
    })
    .to_string();
    let voucher_id: Option<i64> = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_stat-tsp-voucher"
           (id, code, notice, comments, fk_production, "fk_subj-storage", "fk_obj-storage",
            qk_outgo, qk_income, qk_total, "ck_sto-title", _t_, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4,
                   CASE WHEN $5 = 'OUT' THEN $6 ELSE NULL END,
                   CASE WHEN $5 = 'IN' THEN $6 ELSE NULL END,
                   CASE WHEN $5 = 'OUT' THEN $7 ELSE NULL END,
                   CASE WHEN $5 = 'IN' THEN $7 ELSE NULL END,
                   $7,
                   (SELECT id FROM "isahl"."zc_id_cate-sto-title" WHERE code = 'STO-IDLE' LIMIT 1),
                   '实例', 1)
           ON CONFLICT (code) WHERE deleted_at IS NULL DO NOTHING
           RETURNING id"#,
    )
    .bind(code)
    .bind(format!("守卫测试 {code}"))
    .bind(&comments)
    .bind(pool_product)
    .bind(side)
    .bind(line)
    .bind(weight_scalar)
    .fetch_optional(&mut **tx)
    .await
    .expect("insert voucher");

    let Some(voucher_id) = voucher_id else {
        return (None, None); // 唯一索引拦截（同 code 已存在）——幂等跳过，未走原语
    };
    let rec = guard_rec(
        voucher_id,
        code,
        pool_product,
        line,
        weight_scalar,
        side,
        min,
        max,
    );
    let result = apply_guarded_voucher_tx(&mut *tx, &rec).await;
    (Some(voucher_id), Some(result))
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn guarded_voucher_chain_tail_and_bounds() {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    let pool = match url {
        Some(u) => sqlx::PgPool::connect(&u).await.expect("connect"),
        None => connect_test_db().await,
    };

    let suffix = format!("ct{}", std::process::id());
    let (pool_product, line) = seed_capacity_row(&pool, &suffix, 100.0, 100.0).await;

    let mut tx = pool.begin().await.expect("begin");

    // 1. 首笔 OUT 40：无链（退化）→ cur=mark=100 → after=60（min=0/max=100 内）→ Applied
    let (_, r1) = insert_and_guard(
        &mut tx,
        &format!("TSP-T{}-OUT1", suffix),
        pool_product,
        line,
        40.0,
        "OUT",
        Some(0.0),
        Some(100.0),
        "test",
    )
    .await;
    assert_eq!(r1, Some(Ok(VoucherApply::Applied)), "首笔 OUT 应通过");
    let mark = stock_mark_tx(&mut tx, pool_product, line)
        .await
        .expect("mark");
    assert!(
        (mark - 60.0).abs() < 1e-9,
        "OUT 40 后可售应=60，实际={mark}"
    );

    // 2. 再 OUT 70：链尾 balance=60 → after=-10 < min=0 → Err（防负库存）
    let (_, r2) = insert_and_guard(
        &mut tx,
        &format!("TSP-T{}-OUT2", suffix),
        pool_product,
        line,
        70.0,
        "OUT",
        Some(0.0),
        Some(100.0),
        "test",
    )
    .await;
    assert!(matches!(r2, Some(Err(_))), "链尾判定：after<0 应拒绝");

    // 3. IN 30：链尾 balance=60 → after=90 ≤ 100 → Applied
    let (_, r3) = insert_and_guard(
        &mut tx,
        &format!("TSP-T{}-IN1", suffix),
        pool_product,
        line,
        30.0,
        "IN",
        None,
        Some(100.0),
        "test",
    )
    .await;
    assert_eq!(r3, Some(Ok(VoucherApply::Applied)), "IN 30 应通过");
    let mark3 = stock_mark_tx(&mut tx, pool_product, line)
        .await
        .expect("mark");
    assert!(
        (mark3 - 90.0).abs() < 1e-9,
        "IN 30 后可售应=90，实际={mark3}"
    );

    // 4. 再 IN 40：链尾 balance=90 → after=130 > max=100 → Err（跨 action 双回补拦截）
    let (_, r4) = insert_and_guard(
        &mut tx,
        &format!("TSP-T{}-IN2", suffix),
        pool_product,
        line,
        40.0,
        "IN",
        None,
        Some(100.0),
        "test",
    )
    .await;
    assert!(
        matches!(r4, Some(Err(_))),
        "链尾判定：after>max 应拒绝（重复回补）"
    );

    tx.rollback().await.expect("rollback");
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn guarded_voucher_code_idempotent_replay() {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    let pool = match url {
        Some(u) => sqlx::PgPool::connect(&u).await.expect("connect"),
        None => connect_test_db().await,
    };

    let suffix = format!("rp{}", std::process::id());
    let (pool_product, line) = seed_capacity_row(&pool, &suffix, 100.0, 50.0).await;
    let code = format!("TSP-T{}-REPLAY", suffix);

    // 第一次：Applied
    let mut tx1 = pool.begin().await.expect("begin");
    let (_, r1) = insert_and_guard(
        &mut tx1,
        &code,
        pool_product,
        line,
        20.0,
        "IN",
        None,
        Some(100.0),
        "test",
    )
    .await;
    assert_eq!(r1, Some(Ok(VoucherApply::Applied)), "首次应 Applied");
    tx1.commit().await.expect("commit");

    // 第二次（同 code 重放）：唯一索引硬拦截（ON CONFLICT DO NOTHING → 无返回行），不物化
    let mut tx2 = pool.begin().await.expect("begin");
    let (v2, r2) = insert_and_guard(
        &mut tx2,
        &code,
        pool_product,
        line,
        20.0,
        "IN",
        None,
        Some(100.0),
        "test",
    )
    .await;
    assert!(v2.is_none(), "重放应被唯一索引拦截（无凭证行）");
    assert!(r2.is_none(), "重放不应走守卫原语");
    tx2.commit().await.expect("commit");

    // 断言：同 code 未删除凭证仅 1 张；mark 仅 +20（=70）
    let live: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_stat-tsp-voucher"
           WHERE code = $1 AND deleted_at IS NULL"#,
    )
    .bind(&code)
    .fetch_one(&pool)
    .await
    .expect("live count");
    assert_eq!(live, 1, "同 code 未删除凭证应仅 1 张");
    let mut conn = pool.acquire().await.unwrap();
    let mark = stock_mark_tx(&mut conn, pool_product, line)
        .await
        .expect("mark");
    assert!(
        (mark - 70.0).abs() < 1e-9,
        "重放不物化：可售应=70，实际={mark}"
    );
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn guarded_voucher_external_null_balance_fallback() {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    let pool = match url {
        Some(u) => sqlx::PgPool::connect(&u).await.expect("connect"),
        None => connect_test_db().await,
    };

    let suffix = format!("ext{}", std::process::id());
    let (pool_product, line) = seed_capacity_row(&pool, &suffix, 100.0, 100.0).await;

    // 外源凭证：直插 tsp 叶表，无 balance 回填（qk_pre_balance/qk_balance 恒 NULL）
    let weight_scalar: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_scal-common" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, '外源 15', 15::numeric, 1) RETURNING id"#,
    )
    .bind(format!("WT-T{}-EXT", suffix))
    .fetch_one(&pool)
    .await
    .expect("ext scalar");
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_stat-tsp-voucher"
           (id, code, notice, fk_production, "fk_obj-storage", qk_income, qk_total, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, '外源导入', $2, $3, $4, $4, 1)"#,
    )
    .bind(format!("EXT-T{}-1", suffix))
    .bind(pool_product)
    .bind(line)
    .bind(weight_scalar)
    .execute(&pool)
    .await
    .expect("external voucher");

    // 守卫原语：链尾应为 NULL（外源凭证无 balance）→ 退化 mark 兜底（mark=100）
    // IN 10 → after=110 > max=100 → Err（退化分支同样过边界）
    let mut tx = pool.begin().await.expect("begin");
    let (_, r1) = insert_and_guard(
        &mut tx,
        &format!("TSP-T{}-EXT-IN1", suffix),
        pool_product,
        line,
        10.0,
        "IN",
        None,
        Some(100.0),
        "test",
    )
    .await;
    assert!(
        matches!(r1, Some(Err(_))),
        "退化分支仍应过 __max 边界（外源凭证不计入链）"
    );

    // 再试 IN 5 → after=105 > 100 → 仍 Err
    let (_, r2) = insert_and_guard(
        &mut tx,
        &format!("TSP-T{}-EXT-IN2", suffix),
        pool_product,
        line,
        5.0,
        "IN",
        None,
        Some(100.0),
        "test",
    )
    .await;
    assert!(
        matches!(r2, Some(Err(_))),
        "退化分支 mark 兜底判定：100+5=105 > 100 拒绝"
    );

    tx.rollback().await.expect("rollback");
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn guarded_voucher_concurrent_same_code_single_materialization() {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    let pool = match url {
        Some(u) => sqlx::PgPool::connect(&u).await.expect("connect"),
        None => connect_test_db().await,
    };
    ensure_voucher_idempotency(&pool).await.expect("ensure idx");

    let suffix = format!("cc{}", std::process::id());
    let (pool_product, line) = seed_capacity_row(&pool, &suffix, 100.0, 50.0).await;
    let code = format!("TSP-T{}-CONC", suffix);

    // 两个并发事务：同 code IN 20，各自 INSERT（ON CONFLICT）后命中者走守卫原语
    let pool2 = pool.clone();
    let (ra, rb) = tokio::join!(
        async {
            let mut tx = pool.begin().await.expect("begin a");
            let (_, r) = insert_and_guard(
                &mut tx,
                &code,
                pool_product,
                line,
                20.0,
                "IN",
                None,
                Some(100.0),
                "test",
            )
            .await;
            tx.commit().await.expect("commit a");
            (r,)
        },
        async {
            let mut tx = pool2.begin().await.expect("begin b");
            let (_, r) = insert_and_guard(
                &mut tx,
                &code,
                pool_product,
                line,
                20.0,
                "IN",
                None,
                Some(100.0),
                "test",
            )
            .await;
            tx.commit().await.expect("commit b");
            (r,)
        }
    );

    let applied = [ra.0.as_ref(), rb.0.as_ref()]
        .iter()
        .filter(|r| matches!(r, Some(Ok(VoucherApply::Applied))))
        .count();
    let skipped = [ra.0.as_ref(), rb.0.as_ref()]
        .iter()
        .filter(|r| r.is_none())
        .count();
    assert_eq!(applied, 1, "并发同 code 应恰好 1 次物化");
    assert_eq!(
        skipped, 1,
        "并发同 code 应恰好 1 次幂等跳过（唯一索引拦截）"
    );

    let live: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_stat-tsp-voucher"
           WHERE code = $1 AND deleted_at IS NULL"#,
    )
    .bind(&code)
    .fetch_one(&pool)
    .await
    .expect("live count");
    assert_eq!(live, 1, "并发后同 code 未删除凭证应仅 1 张");
    let mut conn = pool.acquire().await.unwrap();
    let mark = stock_mark_tx(&mut conn, pool_product, line)
        .await
        .expect("mark");
    assert!(
        (mark - 70.0).abs() < 1e-9,
        "并发单物化：可售应=70，实际={mark}"
    );
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn voucher_idempotency_index_self_heals() {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    let pool = match url {
        Some(u) => sqlx::PgPool::connect(&u).await.expect("connect"),
        None => connect_test_db().await,
    };

    // 清理（模拟缺失状态）
    for idx in [
        "uq_zc_id_stat-tsp-voucher_code_active",
        "uq_zc_id_stat-com-voucher_code_active",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP INDEX IF EXISTS isahl.\"{idx}\""
        )))
        .execute(&pool)
        .await
        .expect("drop index");
    }

    // 首次：创建
    ensure_voucher_idempotency(&pool).await.expect("ensure 1");
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = 'isahl' AND indexname = $1)",
    )
    .bind("uq_zc_id_stat-tsp-voucher_code_active")
    .fetch_one(&pool)
    .await
    .expect("index exists");
    assert!(exists, "自愈后 tsp 唯一索引应就绪");

    // 幂等：再次调用零副作用
    ensure_voucher_idempotency(&pool).await.expect("ensure 2");
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_indexes WHERE schemaname = 'isahl' AND indexname LIKE 'uq_zc_id_stat-%_code_active'",
    )
    .fetch_one(&pool)
    .await
    .expect("index count");
    assert_eq!(count, 2, "幂等调用不应重复创建");
}

/// 链起点期初余额=0：首笔 IN 凭证（无前置凭证）→ qk_pre_balance 标量 mark 为 0
/// （账本语义：期初余额默认 0，非反推 before = after − income + outgo 产生负值）
#[tokio::test]
async fn chain_start_opening_balance_zero() {
    let pool = common::testing::connect_test_db().await;
    ensure_voucher_idempotency(&pool)
        .await
        .expect("ensure idempotency indexes");
    let (pool_product, line) = seed_capacity_row(&pool, "chain-start", 100.0, 0.0).await;

    let mut tx = pool.begin().await.expect("tx");
    let (voucher_id, guard_result) = insert_and_guard(
        &mut tx,
        "t-chain-start-1",
        pool_product,
        line,
        100.0,
        "IN",
        None,
        None,
        "chain-start",
    )
    .await;
    tx.commit().await.expect("commit");

    let vid = voucher_id.expect("voucher created");
    guard_result.expect("guard applied").expect("applied ok");

    // 期初余额（qk_pre_balance）标量 mark 应为 0（链起点默认），期末（qk_balance）= 100
    let (pre, bal): (Option<f64>, Option<f64>) = sqlx::query_as(
        r#"SELECT (SELECT sm.mark::float8 FROM isahl."zc_id_scale" sm WHERE sm.id = v.qk_pre_balance),
                  (SELECT sm.mark::float8 FROM isahl."zc_id_scale" sm WHERE sm.id = v.qk_balance)
           FROM isahl."zc_id_stat-sto-voucher" v WHERE v.id = $1"#,
    )
    .bind(vid)
    .fetch_one(&pool)
    .await
    .expect("balances");
    assert_eq!(
        pre,
        Some(0.0),
        "链起点期初余额应为 0（账本默认），而非反推负值"
    );
    assert_eq!(bal, Some(100.0), "首笔入账期末余额 = income");

    // cleanup
    sqlx::query(r#"DELETE FROM isahl."zc_id_stat-sto-voucher" WHERE id = $1"#)
        .bind(vid)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_production_rr_storage" WHERE ref_left = $1"#)
        .bind(pool_product)
        .execute(&pool)
        .await
        .ok();
}
