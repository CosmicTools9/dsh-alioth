//! 单位 `dimension`（维度键）MUST 与连接 `search_path` 无关
//! （change: fix-leaf-derivation-render-dependence）
//!
//! 根因：`replace(replace(tableoid::regclass::text, '"zc_id_unit-', ''), '"', '')` 在
//! `search_path` 不含 `isahl` 时得 `isahl.angle`（应 `angle`）⇒ 维度键匹配/展示失效。
//! 单连接池 + `SET search_path = public` 固化该会话状态。

use common::data::ListQuery;
use crud::AliothRepository as _;
use measurement::biz::models::MeasurementUnit;
use measurement::biz::repositories::unit::MeasurementUnitRepository;

#[tokio::test]
async fn unit_dimension_is_bare_key_under_public_search_path() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://isahl@localhost:5432/aliothstudio_test".to_string());
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect test db");
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("current_database");
    assert!(db.contains("_test"), "REFUSED: non-test db {db}");
    sqlx::query("SET search_path = public")
        .execute(&pool)
        .await
        .expect("set search_path=public");

    let repo = MeasurementUnitRepository::from(pool.clone());
    let page = repo
        .list(&ListQuery {
            page: 1,
            page_size: 100,
            filter_field: None,
            filter_op: None,
            filter_value: None,
            sort_field: None,
            sort_order: None,
        })
        .await
        .expect("list units");

    let leaf_units: Vec<&MeasurementUnit> = page
        .items
        .iter()
        .filter(|u| u.dimension.is_some())
        .collect();
    assert!(!leaf_units.is_empty(), "测试库应有维度叶表单位行");
    for u in leaf_units {
        let dim = u.dimension.as_deref().unwrap();
        assert!(
            !dim.contains("isahl") && !dim.contains('"'),
            "dimension MUST 为裸维度键（旧实现落 isahl.angle）: {dim}"
        );
        assert!(
            !dim.starts_with("zc_id_"),
            "dimension 不应带表名前缀: {dim}"
        );
    }
}
