//! Integration tests for the reference (enum-source) read path.
//!
//! reference/{table} 只读路由解决「枚举下拉需要读非叶表」场景：
//! leaf endpoint 的 is_leaf_table 契约正确拒绝非叶表（见
//! schema_repository_leaf_test.rs），但枚举源（zc_id_process 等）数据在父表。
//! 授权（fk_index 覆盖 或 service 注入 allowlist）在 handler 层完成；
//! repository 层仅保证表存在于 isahl schema（读路径的 DB 侧兜底）。

use ::common::testing::connect_test_db;
use crud::schema_repository::SchemaRepository;

/// 非叶但被 FK 引用的表（合法引用目标，repository 层存在性检查应放行）。
const NON_LEAF_REF: &str = "zc_id_process";
/// 独立叶表（存在于 isahl，repository 层存在性检查应放行）。
const LEAF_ONLY: &str = "zc_id_leve-health";
/// 不存在的表（应拒绝）。
const NONEXISTENT: &str = "zc_id_does_not_exist_xyz";
/// 非 isahl 表（存在但 schema 不符，应拒绝）。
const UNMANAGED: &str = "pg_stat_activity";

#[tokio::test]
async fn reference_reads_non_leaf_fk_target() {
    let pool = connect_test_db().await;
    let d = SchemaRepository::new(pool);
    let rows = d.list_reference(NON_LEAF_REF, 1, 20).await;
    assert!(
        rows.is_ok(),
        "isahl 表应可读（存在性检查）: {:?}",
        rows.err()
    );
    let rows = rows.unwrap();
    assert!(rows.len() <= 20);
}

#[tokio::test]
async fn reference_reads_leaf_enum_table() {
    let pool = connect_test_db().await;
    let d = SchemaRepository::new(pool);
    let rows = d.list_reference(LEAF_ONLY, 1, 20).await;
    assert!(rows.is_ok(), "isahl 叶表应可读: {:?}", rows.err());
}

#[tokio::test]
async fn reference_rejects_nonexistent_table() {
    let pool = connect_test_db().await;
    let d = SchemaRepository::new(pool);
    let res = d.list_reference(NONEXISTENT, 1, 20).await;
    assert!(res.is_err(), "不存在的表应被拒绝");
}

#[tokio::test]
async fn reference_rejects_unmanaged_table() {
    let pool = connect_test_db().await;
    let d = SchemaRepository::new(pool);
    let res = d.list_reference(UNMANAGED, 1, 20).await;
    assert!(res.is_err(), "非 isahl 表应被拒绝: {:?}", res);
}
