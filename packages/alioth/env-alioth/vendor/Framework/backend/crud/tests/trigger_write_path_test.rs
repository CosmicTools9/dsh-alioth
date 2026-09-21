//! 触发器写入口集成测试（真实测试库）。
//!
//! 契约（`openspec/changes/wire-generated-writes-through-triggers`）：
//! ① pool 版 `insert_with_triggers` 经运行时触发器产出规范形 `o_number`（证明该通道确实写派生列）；
//! ② tx 版 `insert_with_triggers_tx` 的 DML 参与调用方事务 —— `COMMIT` 持久、`ROLLBACK` 无痕。
//!
//! 注册表口径与 **Gateway 生产一致**：只做 `init_smart_registry_global`（**编译期**继承图，
//! `load_default_alioth_hierarchy`）+ `refresh_smart_registry_from_pg_catalog`（Gateway 启动序列）。
//! 测试覆盖：① 图内表（`zc_id_status`）经通道产出规范形 `o_number`；② 事务语义（tx 版随调用方
//! 事务回滚/提交）；③ 图外表（`zc_id_stus-project`）在 pg_catalog 图刷新后才命中触发器；
//! ④ 维度表（`zc_id_scene`）写入正常（历史 `v_sort` 模板已于 2026-09-15 退役：`v_sort` 全 schema 0 列 ⇒ 死码）。

use common::testing::connect_test_db;
use crud::trigger::{delete_with_triggers, insert_with_triggers, insert_with_triggers_tx};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;

const TEST_TABLE: &str = "zc_id_status";

/// 断言查询（静态 SQL，遵循 dynamic-table-name 门禁）；与 `TEST_TABLE` MUST 一致。
const COUNT_ROW_SQL: &str = r#"SELECT count(*) FROM ONLY isahl."zc_id_status" WHERE id = $1"#;

/// 编译期硬编码图**未收录**的叶表（`zc_id_stus-project -> zc_id_status -> zc_id_object`）：
/// 只在 pg_catalog 图刷新后才应命中触发器。
const LEAF_TABLE: &str = "zc_id_stus-project";

/// 维度终端表：会被历史遗留 `DimensionVSortTemplate` 命中（该模板写 `v_sort`，而全 schema 无该列）。
const DIM_TABLE: &str = "zc_id_scene";

/// 把运行时继承图刷成数据库真实树（`pg_catalog.pg_inherits`；Gateway 容器口径，不读 isahl_meta）。
async fn refresh_graph_from_pg_catalog(pool: &PgPool) {
    trigger_registry::init::refresh_smart_registry_from_pg_catalog(pool)
        .await
        .expect("refresh inheritance graph from pg_catalog");
}

/// 注册表是进程级 `OnceLock`：无条件调用即可（并发下「已初始化」返回 Err 属正常，忽略之；
/// 拿到 Err 时值必已就位 ⇒ 各测试线程都不会踩到未初始化窗口）。
async fn ensure_registry(pool: &PgPool) {
    let _ = trigger_registry::init::init_smart_registry_global(
        pool,
        trigger_registry::AppContainer::Gateway,
    )
    .await;
}

fn notice_record(notice: &str) -> HashMap<String, Value> {
    let mut record = HashMap::new();
    record.insert("notice".to_string(), Value::String(notice.to_string()));
    record
}

/// 规范形判据 `{YYYYMMDD}_{HHMMSSmmm}_{crc32hex}`：8 位日期 + `_` + 9 位时分秒毫秒 + `_` + 8 位小写 hex。
fn is_canonical_o_number(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 27 || bytes[8] != b'_' || bytes[18] != b'_' {
        return false;
    }
    bytes[..8].iter().all(u8::is_ascii_digit)
        && bytes[9..18].iter().all(u8::is_ascii_digit)
        && bytes[19..]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
}

/// id 取值：`to_jsonb` 可能给出数值或字符串（ID 精度规范），两种都接受。
fn id_of(row: &HashMap<String, Value>) -> i64 {
    match row.get("id") {
        Some(Value::Number(n)) => n.as_i64().expect("id 数值越界"),
        Some(Value::String(s)) => s.parse().expect("id 字符串解析"),
        other => panic!("返回记录缺 id: {other:?}"),
    }
}

fn o_number_of(row: &HashMap<String, Value>) -> String {
    row.get("o_number")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

async fn row_exists(pool: &PgPool, id: i64) -> bool {
    let n: i64 = sqlx::query_scalar(COUNT_ROW_SQL)
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("count test row");
    n > 0
}

#[tokio::test]
async fn pool_variant_writes_canonical_o_number() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;

    let row = insert_with_triggers(
        &pool,
        TEST_TABLE,
        notice_record("trigger-write-path-pool"),
        None,
    )
    .await
    .expect("insert_with_triggers");

    let id = id_of(&row);
    let o_number = o_number_of(&row);
    assert!(
        is_canonical_o_number(&o_number),
        "触发器应写规范形 o_number，实得 {o_number:?}"
    );

    // 清理：同时覆盖重构后的 delete 路径（单语句 DELETE … RETURNING + AFTER）
    assert!(
        delete_with_triggers(&pool, TEST_TABLE, id, None)
            .await
            .expect("delete_with_triggers"),
        "清理应删除刚插入的行"
    );
    assert!(!row_exists(&pool, id).await, "清理后该行不得存在");
}

#[tokio::test]
async fn tx_variant_rollback_leaves_no_row() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;

    let mut tx = pool.begin().await.expect("begin");
    let row = insert_with_triggers_tx(
        &mut tx,
        &pool,
        TEST_TABLE,
        notice_record("trigger-write-path-tx-rollback"),
        None,
    )
    .await
    .expect("insert_with_triggers_tx");
    let id = id_of(&row);
    let o_number = o_number_of(&row);
    assert!(
        is_canonical_o_number(&o_number),
        "事务内亦应产出规范形 o_number，实得 {o_number:?}"
    );

    let inside: i64 = sqlx::query_scalar(COUNT_ROW_SQL)
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .expect("count inside tx");
    assert_eq!(inside, 1, "同一事务内应可见该行");

    tx.rollback().await.expect("rollback");
    assert!(
        !row_exists(&pool, id).await,
        "ROLLBACK 后该行不得存在 —— DML 确在调用方事务内"
    );
}

#[tokio::test]
async fn tx_variant_commit_persists_row() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;

    let mut tx = pool.begin().await.expect("begin");
    let row = insert_with_triggers_tx(
        &mut tx,
        &pool,
        TEST_TABLE,
        notice_record("trigger-write-path-tx-commit"),
        None,
    )
    .await
    .expect("insert_with_triggers_tx");
    let id = id_of(&row);
    tx.commit().await.expect("commit");

    assert!(row_exists(&pool, id).await, "COMMIT 后该行应存在");

    delete_with_triggers(&pool, TEST_TABLE, id, None)
        .await
        .expect("cleanup");
}

/// 图外叶表：编译期硬编码图未收录 ⇒ 不刷新图时触发器不触发；刷新 pg_catalog 图后应命中并产出 o_number。
#[tokio::test]
async fn leaf_table_gets_o_number_after_pg_catalog_refresh() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;
    refresh_graph_from_pg_catalog(&pool).await;

    let row = insert_with_triggers(
        &pool,
        LEAF_TABLE,
        notice_record("trigger-write-path-leaf"),
        None,
    )
    .await
    .expect("insert into leaf table");
    let id = id_of(&row);
    let o_number = o_number_of(&row);
    assert!(
        is_canonical_o_number(&o_number),
        "pg_catalog 图刷新后叶表应产出规范形 o_number，实得 {o_number:?}"
    );

    // 清理返回 true 即证明该行确实写入过
    assert!(
        delete_with_triggers(&pool, LEAF_TABLE, id, None)
            .await
            .expect("cleanup"),
        "清理应删除刚插入的叶表行"
    );
}

/// 维度终端表 `zc_id_scene` 写路径（历史 `DimensionVSortTemplate` 已退役；此用例保留为维度表写路径回归）。
#[tokio::test]
async fn dimension_table_write_path() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;

    let row = insert_with_triggers(
        &pool,
        DIM_TABLE,
        notice_record("trigger-write-path-dim"),
        None,
    )
    .await
    .expect("insert into dimension table");
    let id = id_of(&row);
    let o_number = o_number_of(&row);
    assert!(
        is_canonical_o_number(&o_number),
        "维度表应产出规范形 o_number，实得 {o_number:?}"
    );

    assert!(
        delete_with_triggers(&pool, DIM_TABLE, id, None)
            .await
            .expect("cleanup"),
        "清理应删除刚插入的维度行"
    );
}
