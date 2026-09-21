//! Integration tests for the leaf-write path (SchemaRepository, 原 ontology dispatcher 迁入)。

use ::common::testing::connect_test_db;
use crud::schema_repository::{AliothLeaf, Binding, SchemaRepository};

/// 夹具：`SchemaRepository` API 参数用表名 + 三条夹具语句同源（表名以 `$table:literal`
/// 出现一次，`concat!` 编译期固化；运行期无表名插值）。
macro_rules! biz_leaf_fixture {
    ($table:literal) => {
        /// 叶表名（`SchemaRepository` API 参数用值）。
        const TEST_BIZ_LEAF: &str = $table;
        /// 清场（按 notice）。
        const CLEAN_FIXTURE_ROWS_SQL: &str =
            concat!(r#"DELETE FROM isahl.""#, $table, r#"" WHERE notice = $1"#);
        /// 断言坐标三元组已写入。
        const LEAF_BINDING_SQL: &str = concat!(
            r#"SELECT dk_scene, dk_factor, dk_function FROM isahl.""#,
            $table,
            r#"" WHERE id = $1"#
        );
        /// 清场（按 id）。
        const DELETE_FIXTURE_ROW_SQL: &str =
            concat!(r#"DELETE FROM isahl.""#, $table, r#"" WHERE id = $1"#);
    };
}

biz_leaf_fixture!("zc_id_agre-pricing");

#[tokio::test]
async fn is_leaf_table_accepts_real_leaf() {
    let pool = connect_test_db().await;
    let d = SchemaRepository::new(pool);
    assert!(
        d.is_leaf_table(TEST_BIZ_LEAF).await.unwrap(),
        "{} should be a leaf",
        TEST_BIZ_LEAF
    );
}

#[tokio::test]
async fn is_leaf_table_rejects_internal_node() {
    let pool = connect_test_db().await;
    let d = SchemaRepository::new(pool);
    assert!(!d.is_leaf_table("zc_id_lifecycle").await.unwrap());
    assert!(!d.is_leaf_table("zc_id_storage").await.unwrap());
}

#[tokio::test]
async fn writable_columns_excludes_protected() {
    let pool = connect_test_db().await;
    let d = SchemaRepository::new(pool);
    let cols = d.writable_columns(TEST_BIZ_LEAF).await.unwrap();
    assert!(!cols.contains(&"id".to_string()));
    assert!(!cols.contains(&"created_at".to_string()));
    assert!(!cols.contains(&"deleted_at".to_string()));
    assert!(cols.contains(&"notice".to_string()) || cols.contains(&"code".to_string()));
}

#[tokio::test]
async fn create_in_leaf_persists_binding() {
    let pool = connect_test_db().await;
    let d = SchemaRepository::new(pool.clone());
    // Clean prior test data
    sqlx::query(CLEAN_FIXTURE_ROWS_SQL)
        .bind("__test_ontology_dispatcher__")
        .execute(&pool)
        .await
        .unwrap();

    let binding: Binding = (
        Some(111_111_111_111_111_111),
        Some(222_222_222_222_222_222),
        Some(333_333_333_333_333_333),
    );
    let leaf = AliothLeaf::new()
        .with("notice", "__test_ontology_dispatcher__")
        .with("code", "OD-001")
        .with("public", true);

    let new_id = d
        .create_in_leaf(TEST_BIZ_LEAF, binding, leaf, 1)
        .await
        .expect("create");
    assert!(new_id > 0);

    let row: (Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(LEAF_BINDING_SQL)
        .bind(new_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, Some(111_111_111_111_111_111));
    assert_eq!(row.1, Some(222_222_222_222_222_222));
    assert_eq!(row.2, Some(333_333_333_333_333_333));

    sqlx::query(DELETE_FIXTURE_ROW_SQL)
        .bind(new_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn list_leaf_returns_paginated_rows() {
    let pool = connect_test_db().await;
    let d = SchemaRepository::new(pool);
    let rows = d.list_leaf(TEST_BIZ_LEAF, 1, 5).await.unwrap();
    assert!(rows.len() <= 5);
}

#[tokio::test]
async fn rejects_non_leaf_table() {
    let pool = connect_test_db().await;
    let d = SchemaRepository::new(pool);
    let res = d
        .create_in_leaf("zc_id_lifecycle", (None, None, None), AliothLeaf::new(), 1)
        .await;
    assert!(res.is_err());
}
