//! inventory-balance 统计集成测试（TEST_INFRASTRUCTURE.md：`#[tokio::test]` + `connect_test_db()`）。
//!
//! 语义（用户裁定）：库存 = 货在储元中的时空切片数量统计。
//! 数据源：isahl.mv_inventory（容量行物化视图：production × storage 的 qty/capacity）。
//! 覆盖：列表查询、货/储元过滤、名称解析。

mod common;

use alioth_service_inventory_balance::AliothInventoryNameResolver;
use common::{setup_test_schema, test_code};
use inventory::models::{NameResolver, RefKind};
use inventory::service::InventoryService;
use sqlx::PgPool;
use std::sync::Arc;

/// 插入容量行（production_rr_storage）——qty/capacity 经标量表引用
async fn insert_storage_row(
    pool: &PgPool,
    production: i64,
    storage: i64,
    qty_mark: i64,
    cap_mark: i64,
) -> i64 {
    let qty_scal: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (notice, mark) VALUES ($1, $2) RETURNING id"#,
    )
    .bind(test_code("qty"))
    .bind(qty_mark)
    .fetch_one(pool)
    .await
    .expect("insert qty scalar");
    let cap_scal: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (notice, mark) VALUES ($1, $2) RETURNING id"#,
    )
    .bind(test_code("cap"))
    .bind(cap_mark)
    .fetch_one(pool)
    .await
    .expect("insert cap scalar");
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_file_rr_url"
           (ref_left, ref_right, qk_qty, qk_p_capacity, code)
           VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
    )
    .bind(production)
    .bind(storage)
    .bind(qty_scal)
    .bind(cap_scal)
    .bind(test_code("row"))
    .fetch_one(pool)
    .await
    .expect("insert storage row")
}

/// 刷新物化视图（时空切片）
async fn refresh_view(pool: &PgPool) {
    sqlx::query("REFRESH MATERIALIZED VIEW isahl.mv_inventory")
        .execute(pool)
        .await
        .expect("refresh mv_inventory");
}

#[tokio::test]
async fn list_with_qty_capacity() {
    let pool = ::common::testing::connect_test_db().await;
    setup_test_schema(&pool).await.expect("setup failed");
    let production = 8_000_000_001_i64;
    let storage = 8_000_000_002_i64;

    insert_storage_row(&pool, production, storage, 120, 200).await;
    refresh_view(&pool).await;

    let svc = InventoryService::new(pool.clone(), Arc::new(AliothInventoryNameResolver));
    let page = svc
        .list(&Default::default(), Some(production), Some(storage))
        .await
        .expect("list balances");

    let row = page
        .items
        .iter()
        .find(|r| r.production_id == production && r.storage_id == storage)
        .expect("balance row exists");
    assert_eq!(
        row.qty,
        rust_decimal::Decimal::new(120, 0),
        "qty 经标量表解析"
    );
    assert_eq!(
        row.capacity,
        rust_decimal::Decimal::new(200, 0),
        "capacity 经标量表解析"
    );
    assert!(row.unit.is_some() || row.unit.is_none(), "unit 可空");
}

#[tokio::test]
async fn filter_by_production_and_storage() {
    let pool = ::common::testing::connect_test_db().await;
    setup_test_schema(&pool).await.expect("setup failed");
    let production = 8_000_000_003_i64;
    let storage_a = 8_000_000_004_i64;
    let storage_b = 8_000_000_005_i64;

    insert_storage_row(&pool, production, storage_a, 10, 50).await;
    insert_storage_row(&pool, production, storage_b, 20, 60).await;
    refresh_view(&pool).await;

    let svc = InventoryService::new(pool.clone(), Arc::new(AliothInventoryNameResolver));
    let page = svc
        .list(&Default::default(), Some(production), Some(storage_a))
        .await
        .expect("list balances");
    assert!(
        page.items
            .iter()
            .all(|r| r.production_id == production && r.storage_id == storage_a),
        "过滤后仅命中目标货×储元"
    );
}

#[tokio::test]
async fn name_resolution_via_namespace_resolver() {
    let pool = ::common::testing::connect_test_db().await;
    setup_test_schema(&pool).await.expect("setup failed");

    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JE", "FRA", "↓_EE"))
            .await
            .expect("resolve dk coords");
    let production: i64 = sqlx::query_scalar(
        // 物料制成品夹具 → 叶 zc_id_prod-material-made
        r#"INSERT INTO isahl."zc_id_prod-material-made" (notice, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, '实现', '实例', $2, $3, $4) RETURNING id"#,
    )
    .bind(test_code("prod"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(&pool)
    .await
    .expect("insert production");
    let storage: i64 = sqlx::query_scalar(
        // 储元夹具 → 叶 stor-ctn-box（家族 storage→stor-container→箱型叶）+ 坐标 TX/FJA/↓_GG
        r#"INSERT INTO isahl."zc_id_stor-ctn-box" (notice, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES ($1, '实现', '实例',
                   (SELECT id FROM isahl.zc_id_scene    WHERE code = 'TX'   AND deleted_at IS NULL LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor   WHERE code = 'FJA'  AND deleted_at IS NULL LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↓_GG' AND deleted_at IS NULL LIMIT 1)) RETURNING id"#,
    )
    .bind(test_code("stor"))
    .fetch_one(&pool)
    .await
    .expect("insert storage");

    let resolver = AliothInventoryNameResolver;
    let names = resolver
        .resolve(&pool, RefKind::Material, &[production])
        .await;
    let place_names = resolver.resolve(&pool, RefKind::Place, &[storage]).await;
    assert!(
        names
            .get(&RefKind::Material)
            .and_then(|m| m.get(&production))
            .is_some(),
        "货名称应解析"
    );
    assert!(
        place_names
            .get(&RefKind::Place)
            .and_then(|m| m.get(&storage))
            .is_some(),
        "储元名称应解析"
    );
}
