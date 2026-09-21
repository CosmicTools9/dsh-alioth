//! 围栏删除的引用清除（逻辑触发器）集成测试。
//!
//! 契约：`FenceRepository::delete` MUST 在同一事务内把引用被删围栏的地点 `qk_fence` 置空
//! （经继承根 `zc_id_place` 覆盖全部地点叶表），并 MUST 只影响指向该围栏的地点。
//!
//! 背景：原 DB 触发器 `trg_clear_place_fence_ref` 挂在非叶表 `zc_id_geom-circle`，
//! 而围栏实体叶表为 `zc_id_geog-*`（fence-gis 规约）——行级触发器不向子表传播，
//! 该触发器恒不触发；逻辑改由 Rust 承载，本测试锁定行为防回归。
//!
//! 依赖：test 库存在 isahl 几何叶表 / `zc_id_stor-plc-division` / 坐标三元组绑定。

use common::testing::connect_test_db;
use crud::repository::AliothRepository;
use identity_org::repository::FenceRepository;
use sqlx::PgPool;

/// 动态测试 id 段（进程+纳秒派生，跨运行不冲突；测试不清理数据）
fn tid(base: i64) -> i64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as i64;
    base + (nanos % 1_000_000) * 1000 + std::process::id() as i64 % 100
}

/// 三类围栏叶表 + 各自几何列写入表达式（PostGIS geometry(4326)，与写路径同源）。
const LEAVES: &[(&str, &str)] = &[
    (
        "zc_id_geog-circle",
        "ST_SetSRID(ST_MakePoint(116.4, 39.9), 4326)",
    ),
    (
        "zc_id_geog-area",
        "ST_SetSRID(ST_MakeEnvelope(116, 39, 117, 40), 4326)",
    ),
    (
        "zc_id_geog-polygon",
        "ST_SetSRID(ST_GeomFromText('POLYGON((116 39,117 39,117 40,116 40,116 39))'), 4326)",
    ),
];

/// 建一条围栏叶表行，返回 id。
async fn insert_fence(pool: &PgPool, leaf: &str, geom: &str, code: &str) -> i64 {
    // 三叶表静态 INSERT 头（表名/列名编译期固化；geom 为测试常量 SQL 片段）
    macro_rules! fence_insert_sql {
        ($leaf:literal, $col:literal) => {
            concat!(
                r#"INSERT INTO isahl.""#,
                $leaf,
                r#"" (code, notice, created_by_id, ""#,
                $col,
                r#"") VALUES ($1, $2, 1, "#,
                "{}",
                ") RETURNING id"
            )
        };
    }
    let sql = match leaf {
        "zc_id_geog-circle" => format!(fence_insert_sql!("zc_id_geog-circle", "circle"), geom),
        "zc_id_geog-area" => format!(fence_insert_sql!("zc_id_geog-area", "box"), geom),
        "zc_id_geog-polygon" => format!(fence_insert_sql!("zc_id_geog-polygon", "polygon"), geom),
        other => panic!("未知围栏叶表: {other}"),
    };
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(code)
        .bind(format!("围栏删除测试 {code}"))
        .fetch_one(pool)
        .await
        .expect("insert fence")
}

/// 建一个绑定了 `qk_fence` 的地点（叶表 zc_id_stor-plc-division），返回 id。
async fn insert_place(pool: &PgPool, fence_id: i64, code: &str) -> i64 {
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve place coords");
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_stor-plc-division"
           (code, notice, created_by_id, dk_scene, dk_factor, dk_function, qk_fence)
           VALUES ($1, $2, 1, $3, $4, $5, $6) RETURNING id"#,
    )
    .bind(code)
    .bind(format!("围栏引用测试 {code}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .bind(fence_id)
    .fetch_one(pool)
    .await
    .expect("insert place")
}

async fn place_fence(pool: &PgPool, place_id: i64) -> Option<i64> {
    sqlx::query_scalar(r#"SELECT qk_fence FROM isahl."zc_id_place" WHERE id = $1"#)
        .bind(place_id)
        .fetch_one(pool)
        .await
        .expect("read place qk_fence")
}

#[tokio::test]
async fn fence_delete_clears_place_ref() {
    let pool = connect_test_db().await;
    let base = tid(9_100_000_000_000);
    let repo = FenceRepository::new(pool.clone());

    for (i, (leaf, geom)) in LEAVES.iter().enumerate() {
        let code = format!("FENCE-DEL-{}-{i}", base % 1_000_000);
        let fence = insert_fence(&pool, leaf, geom, &code).await;
        let bound = insert_place(
            &pool,
            fence,
            &format!("PLC-FENCE-DEL-{}-{i}", base % 1_000_000),
        )
        .await;
        // 对照组：绑定另一条存活围栏的地点，删除后引用必须保留
        let keep_fence = insert_fence(&pool, leaf, geom, &format!("{code}-KEEP")).await;
        let keeper = insert_place(
            &pool,
            keep_fence,
            &format!("PLC-FENCE-KEEP-{}-{i}", base % 1_000_000),
        )
        .await;

        assert_eq!(place_fence(&pool, bound).await, Some(fence));
        repo.delete(fence, 1).await.expect("delete fence");

        let fence_deleted: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_scalar(r#"SELECT deleted_at FROM isahl."zc_id_geometry" WHERE id = $1"#)
                .bind(fence)
                .fetch_one(&pool)
                .await
                .expect("read fence deleted_at");
        assert!(fence_deleted.is_some(), "{leaf}: 围栏未软删");

        assert_eq!(
            place_fence(&pool, bound).await,
            None,
            "{leaf}: 围栏删除后地点 qk_fence 未清除（悬空引用）"
        );
        assert_eq!(
            place_fence(&pool, keeper).await,
            Some(keep_fence),
            "{leaf}: 非目标围栏的地点引用被误清"
        );
    }
}

/// 幂等/一致性：对已删除围栏重复删除 MUST 返回 NotFound 且不产生写入。
#[tokio::test]
async fn fence_delete_twice_is_rejected() {
    let pool = connect_test_db().await;
    let base = tid(9_200_000_000_000);
    let repo = FenceRepository::new(pool.clone());
    let fence = insert_fence(
        &pool,
        "zc_id_geog-circle",
        "ST_SetSRID(ST_MakePoint(116.4, 39.9), 4326)",
        &format!("FENCE-DEL-TWICE-{}", base % 1_000_000),
    )
    .await;
    repo.delete(fence, 1).await.expect("first delete");
    assert!(
        repo.delete(fence, 1).await.is_err(),
        "重复删除已软删围栏 MUST 报错"
    );
}
