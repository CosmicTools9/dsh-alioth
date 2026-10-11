//! `ngac_obligation` 表结构与 ensure 幂等契约（NGAC_SPEC §2.6）。
//!
//! 背景：该表**历史由仓外迁移引入**（本仓原无其 CREATE TABLE），fresh 库会缺表 ⇒
//! 义务求值静默失效。2026-09-30 起由 `ngac/ensure.rs::ensure_ngac_extension_tables`
//! 幂等补齐；本测试守护两件事：① ensure 可重复执行不报错（幂等）；② 表存在且
//! `ngac/obligation.rs` 依赖的列齐备（缺任一列 = 求值路径静默退化为空义务）。
//!
//! 口径：`TEST_DATABASE_URL="postgres://<user>@localhost:5432/aliothstudio_test"` + `--ignored`。

use sqlx::PgPool;

async fn open_test_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    if url.is_none() {
        eprintln!("skipped: DATABASE_URL 未设置");
        return None;
    }
    Some(common::testing::connect_test_db().await)
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn ensure_creates_obligation_table_with_required_columns() {
    let Some(pool) = open_test_pool().await else {
        return;
    };

    // ① ensure 幂等（连跑两次不报错；第二次由进程内 once 守卫短路）
    gateway_sso::ngac::ensure::ensure_ngac_extension_tables(&pool).await;
    gateway_sso::ngac::ensure::ensure_ngac_extension_tables(&pool).await;

    // ② 表存在（fresh 库由 ensure 建；存量库 IF NOT EXISTS 保持原样）
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('isahl_auth.ngac_obligation') IS NOT NULL")
            .fetch_one(&pool)
            .await
            .expect("probe ngac_obligation");
    assert!(exists, "ensure 后 MUST 存在 isahl_auth.ngac_obligation");

    // ③ `obligation.rs` 依赖的列齐备（求值 SELECT 与求值语义都落在这 8 列上）
    let cols: Vec<String> = sqlx::query_scalar(
        "SELECT column_name FROM information_schema.columns \
         WHERE table_schema = 'isahl_auth' AND table_name = 'ngac_obligation'",
    )
    .fetch_all(&pool)
    .await
    .expect("read columns");
    for required in [
        "o_name",
        "event_pattern",
        "response_ops",
        "is_active",
        "priority",
        "conditions",
        "phase",
        "deleted_at",
    ] {
        assert!(
            cols.iter().any(|c| c == required),
            "缺列 `{required}` ⇒ 义务求值会静默退化（现有列：{cols:?}）"
        );
    }
}
