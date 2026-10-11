//! `ngac_host::load_policy_graph` 对 **NULL 侧 prohibition** 的容错。
//!
//! DB 真值（`mise run schema-info -- describe-table isahl_auth.ngac_prohibition`）：
//! `fk_user_attribute` / `fk_object_attribute` / `is_active` **可空**。NULL 侧无法等于任何
//! 具体 id ⇒ 该行永不匹配任何 (UA, OA) 对，MUST 被跳过，MUST NOT 因单行 NULL 使整图加载
//! fail-closed（历史隐患：`ProhRow` 曾按非空 `i64`/`bool` 解码 ⇒ NULL 行致 PDP 全量拒绝）。
//!
//! 运行：`bash scripts/cargo-test.sh -p common --test ngac_prohibition_null_test`

use common::testing::connect_test_db;

#[tokio::test]
async fn null_side_prohibition_is_skipped_not_fatal() {
    let pool = connect_test_db().await;
    let name = "z-test-proh-null-side";
    sqlx::query("DELETE FROM isahl_auth.ngac_prohibition WHERE o_name = $1")
        .bind(name)
        .execute(&pool)
        .await
        .ok();

    // 仅 `id`/`o_name` 有值，UA/OA/アクティブ 侧全 NULL（DB 允许）。
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_prohibition \
             (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active) \
         VALUES ($1, NULL, NULL, '{}'::bigint[], NULL) RETURNING id",
    )
    .bind(name)
    .fetch_one(&pool)
    .await
    .expect("insert NULL-side prohibition");

    let pg = ngac_host::load_policy_graph(&pool)
        .await
        .expect("load_policy_graph MUST NOT fail on a NULL-side prohibition row");
    assert!(
        !pg.prohibitions.contains_key(&id),
        "NULL-side prohibition MUST be skipped (never matches any (UA, OA) pair)"
    );

    sqlx::query("DELETE FROM isahl_auth.ngac_prohibition WHERE o_name = $1")
        .bind(name)
        .execute(&pool)
        .await
        .ok();
}
