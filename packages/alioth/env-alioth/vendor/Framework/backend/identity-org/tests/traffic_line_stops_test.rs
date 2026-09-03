//! 线路↔场所桥（zc_id_stor-traffic_line_rr_stop）集成测试（add-route-stop-bridge）
//!
//! 覆盖：over-seq 定序读 / 全量替换（软删+按序插入）/ 悬空 place 校验 / 类目解析。
//! 直接执行与 traffic_line_stops.rs handler 相同的 SQL 语义（handler 依赖 HttpRequest
//! 外部 crate 不可构造；SQL 即 handler 行为，测试防回归漂移）。
//!
//! 依赖：test 库存在 isahl.zc_id_stor-traffic_line / zc_id_stor-traffic_line_rr_stop /
//! zc_id_place / zc_id_cate-traffic（ST-STOP 等种子）。

use sqlx::PgPool;

async fn test_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://isahl@localhost:5432/aliothstudio_test".to_string());
    let pool = PgPool::connect(&url).await.expect("connect test db");
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("current_database");
    assert!(db.contains("_test"), "REFUSED: non-test db {db}");
    pool
}

/// 动态测试 id 段（进程+纳秒派生，跨运行不冲突；测试不清理数据）
fn tid(base: i64) -> i64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as i64;
    base + (nanos % 1_000_000) * 1000 + std::process::id() as i64 % 100
}

/// 与 handler list_stops 相同的读出 SQL（over-seq 升序 + place/类目解析）
const LIST_SQL: &str = r#"SELECT b.id, b.ref_right, p.notice, p.code,
          b."over-seq", b.ck_category, c.code AS category_code
   FROM "isahl"."zc_id_stor-traffic_line_rr_stop" b
   LEFT JOIN "isahl"."zc_id_place" p ON p.id = b.ref_right AND p.deleted_at IS NULL
   LEFT JOIN "isahl"."zc_id_cate-traffic" c ON c.id = b.ck_category AND c.deleted_at IS NULL
   WHERE b.ref_left = $1 AND b.deleted_at IS NULL
   ORDER BY b."over-seq" ASC NULLS LAST, b.id"#;

const INSERT_SQL: &str = r#"INSERT INTO "isahl"."zc_id_stor-traffic_line_rr_stop"
   (notice, ref_left, ref_right, ck_category, "over-seq", created_by_id, updated_by_id)
   VALUES ($1, $2, $3, $4, $5, $6, $6)"#;

const SOFT_DELETE_SQL: &str = r#"UPDATE "isahl"."zc_id_stor-traffic_line_rr_stop"
   SET deleted_at = NOW(), deleted_by_id = $2
   WHERE ref_left = $1 AND deleted_at IS NULL"#;

struct Fixture {
    line: i64,
    places: [i64; 3],
    cat_stop: Option<i64>,
    cat_load: Option<i64>,
}

/// 建测试线路 + 3 个场所 + 查类目（ST-STOP / ST-LOAD，后者种子增补后存在）
async fn setup(pool: &PgPool) -> Fixture {
    let base = tid(8_900_000_000_000);
    let line: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_stor-traffic_line" (code, notice, created_by_id)
           VALUES ($1, $2, 1) RETURNING id"#,
    )
    .bind(format!("TL-T{basex}", basex = base % 1_000_000))
    .bind("桥接测试线路")
    .fetch_one(pool)
    .await
    .expect("insert line");
    let mut places = [0i64; 3];
    for (i, p) in places.iter_mut().enumerate() {
        *p = sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_place" (code, notice, created_by_id)
               VALUES ($1, $2, 1) RETURNING id"#,
        )
        .bind(format!("PLC-T{}-{}", base % 1_000_000, i))
        .bind(format!("桥接测试场所{i}"))
        .fetch_one(pool)
        .await
        .expect("insert place");
    }
    // 类目种子幂等确保（test 库可能缺 ST-* 种子；按 code 去重）
    for (code, notice) in [("ST-STOP", "驻停"), ("ST-LOAD", "装载")] {
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_cate-traffic" (code, notice, created_by_id)
               SELECT $1, $2, 1
               WHERE NOT EXISTS (SELECT 1 FROM "isahl"."zc_id_cate-traffic" t WHERE t.code = $1 AND t.deleted_at IS NULL)"#,
        )
        .bind(code)
        .bind(notice)
        .execute(pool)
        .await
        .expect("seed category");
    }
    let cat_stop: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM "isahl"."zc_id_cate-traffic" WHERE code = 'ST-STOP' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(pool)
    .await
    .expect("query ST-STOP");
    let cat_load: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM "isahl"."zc_id_cate-traffic" WHERE code = 'ST-LOAD' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(pool)
    .await
    .expect("query ST-LOAD");
    assert!(cat_stop.is_some() && cat_load.is_some(), "类目种子确保失败");
    Fixture {
        line,
        places,
        cat_stop,
        cat_load,
    }
}

#[tokio::test]
async fn stops_insert_and_ordered_read() {
    let pool = test_pool().await;
    let f = setup(&pool).await;
    // 乱序插入，over-seq 权威定序（0=起点，count-1=终点）
    for (seq, place) in [(1, f.places[1]), (0, f.places[0]), (2, f.places[2])] {
        sqlx::query(INSERT_SQL)
            .bind(format!("line-{} stop-{seq}", f.line))
            .bind(f.line)
            .bind(place)
            .bind(f.cat_stop)
            .bind(seq as i32)
            .bind(1_i64)
            .execute(&pool)
            .await
            .expect("insert bridge");
    }
    let rows: Vec<(
        i64,
        i64,
        Option<String>,
        Option<String>,
        Option<i32>,
        Option<i64>,
        Option<String>,
    )> = sqlx::query_as(LIST_SQL)
        .bind(f.line)
        .fetch_all(&pool)
        .await
        .expect("list");
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter().map(|r| r.1).collect::<Vec<_>>(),
        vec![f.places[0], f.places[1], f.places[2]],
        "over-seq 0..2 升序"
    );
    assert_eq!(rows[0].4, Some(0));
    assert_eq!(rows[2].4, Some(2));
    assert_eq!(rows[0].6.as_deref(), Some("ST-STOP"), "类目 code 解析");
    assert!(rows[0].2.is_some(), "place 名称解析");
}

#[tokio::test]
async fn stops_full_replace_soft_deletes_old() {
    let pool = test_pool().await;
    let f = setup(&pool).await;
    for (seq, place) in [(0, f.places[0]), (1, f.places[1])] {
        sqlx::query(INSERT_SQL)
            .bind("init")
            .bind(f.line)
            .bind(place)
            .bind(f.cat_stop)
            .bind(seq as i32)
            .bind(1_i64)
            .execute(&pool)
            .await
            .expect("insert bridge");
    }
    // 全量替换（handler replace_stops 语义）：软删全部 → 按新序插入 [P2, P0]
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query(SOFT_DELETE_SQL)
        .bind(f.line)
        .bind(1_i64)
        .execute(&mut *tx)
        .await
        .expect("soft delete");
    for (seq, place) in [(0, f.places[2]), (1, f.places[0])] {
        sqlx::query(INSERT_SQL)
            .bind("replaced")
            .bind(f.line)
            .bind(place)
            .bind(f.cat_load)
            .bind(seq as i32)
            .bind(1_i64)
            .execute(&mut *tx)
            .await
            .expect("re-insert");
    }
    tx.commit().await.expect("commit");

    let rows: Vec<(
        i64,
        i64,
        Option<String>,
        Option<String>,
        Option<i32>,
        Option<i64>,
        Option<String>,
    )> = sqlx::query_as(LIST_SQL)
        .bind(f.line)
        .fetch_all(&pool)
        .await
        .expect("list");
    assert_eq!(rows.len(), 2, "旧行软删后仅剩新集合");
    assert_eq!(
        rows.iter().map(|r| r.1).collect::<Vec<_>>(),
        vec![f.places[2], f.places[0]]
    );
    assert_eq!(rows[0].4, Some(0));
    assert_eq!(rows[1].4, Some(1));
    // 软删行不复活：含已删共 4 行
    let total: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM "isahl"."zc_id_stor-traffic_line_rr_stop" WHERE ref_left = $1"#,
    )
    .bind(f.line)
    .fetch_one(&pool)
    .await
    .expect("count all");
    assert_eq!(total, 4);
}

#[tokio::test]
async fn stops_reject_dangling_place() {
    let pool = test_pool().await;
    let f = setup(&pool).await;
    // handler 校验语义：place_ids 在 zc_id_place 的命中数必须等于请求数
    let dangling = 9_999_999_999_999_i64;
    let found: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM "isahl"."zc_id_place" WHERE id = ANY($1) AND deleted_at IS NULL"#,
    )
    .bind(vec![f.places[0], dangling])
    .fetch_one(&pool)
    .await
    .expect("count places");
    assert_ne!(found, 2, "悬空 place 必须被校验拒绝（事务零变更）");
}
