//! 委托派生 → **决策级**生效测试（NGAC_SPEC §2.2.4 + §2.5，change `consolidate-ngac-decision-convergence`）。
//!
//! 为什么需要：既有 `SSO/backend/tests/ngac_delegation_test.rs` 覆盖委托的授予/撤销/边界与
//! 时间窗（**API/状态级**），但**没有任何决策级断言**——「被委托人是否真的因此获得访问」
//! 未被验证；而 M16 收敛项之一正是「委托经 `effective_ua_ids`（DDL 内嵌 `DELEGATED_CTE`）
//! 并入 Framework 内层判定」。
//!
//! 本文件守护：① active 且窗口内 ⇒ 被委托人经委托 UA **放行**；② `status='revoked'` ⇒ **拒绝**；
//! ③ 窗口过期（`date_ed` 已过）⇒ **拒绝**；
//! ④ **授权方账号禁用**（`is_active=FALSE`）⇒ **停供**，重新启用 ⇒ 恢复，且委托行 status 不被改写
//!    （harden-ngac-delegation-dead-principal：禁用账号 MUST NOT 经委托链持续供权）。
//!
//! 口径：`TEST_DATABASE_URL="postgres://<user>@localhost:5432/aliothstudio_test"` + `--ignored`。

use common::ngac_grants::{ensure_resource_oa, ResourceOaSpec};
use common::permissions::require_resource_access;
use sqlx::PgPool;

const RT: &str = "zz-dlg";
const UA_NAME: &str = "zz-dlg-ua";

async fn open_test_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    if url.is_none() {
        eprintln!("skipped: DATABASE_URL 未设置");
        return None;
    }
    Some(common::testing::connect_test_db().await)
}

async fn cleanup(pool: &PgPool) {
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_delegation WHERE fk_delegatee IN (SELECT id FROM isahl_auth.auth_users WHERE username LIKE 'zz-dlg%')")
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.ngac_association a USING isahl_auth.ngac_object_attribute oa \
         WHERE a.fk_object_attribute = oa.id AND oa.resource_type = $1",
    )
    .bind(RT)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type = $1")
        .bind(RT)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'zz-dlg%'")
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username LIKE 'zz-dlg%'")
        .execute(pool)
        .await;
}

/// 夹具：委托 UA + 行 OA + UA 的 `read` association + 委托人/被委托人 + 一条委托行（窗口/状态可调）。
/// 返回 (delegatee, row_id, delegation_id)。
async fn setup(pool: &PgPool, status: &str, date_st: &str, date_ed: &str) -> (i64, i64, i64) {
    // 决策路径消费 DB 函数 `isahl_auth.effective_ua_ids`（其体由 `DELEGATED_CTE` 常量生成），
    // 测试库可能残留旧代际函数体 ⇒ 先按当前常量幂等重建（否则断言的是陈旧语义）。
    common::permissions::ensure_ngac_predicate_fns(pool)
        .await
        .expect("ensure ngac predicate fns");
    let delegator: i64 =
        sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id")
            .bind("zz-dlg-delegator")
            .fetch_one(pool)
            .await
            .expect("insert delegator");
    let delegatee: i64 =
        sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id")
            .bind("zz-dlg-delegatee")
            .fetch_one(pool)
            .await
            .expect("insert delegatee");

    let pc: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default'")
            .fetch_one(pool)
            .await
            .expect("default policy class");
    let ua_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class) \
         VALUES ($1, $2) RETURNING id",
    )
    .bind(UA_NAME)
    .bind(pc)
    .fetch_one(pool)
    .await
    .expect("insert UA");

    let row_id: i64 = 9_900_881;
    ensure_resource_oa(
        pool,
        &ResourceOaSpec {
            o_name: format!("{RT}-{row_id}"),
            resource_type: RT,
            resource_id: row_id,
            resource_identifier: Some("ZZ-DLG"),
            created_by: Some(delegator),
            policy_class_name: Some("default"),
        },
    )
    .await
    .expect("ensure row OA");
    let read_right: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'read'")
            .fetch_one(pool)
            .await
            .expect("read access right");
    let oa_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type = $1 AND fk_resource = $2",
    )
    .bind(RT)
    .bind(row_id)
    .fetch_one(pool)
    .await
    .expect("row OA id");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_association \
         (o_name, fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights) \
         VALUES ($1, $2, $3, $4, ARRAY[$5::bigint])",
    )
    .bind("zz-dlg-assoc")
    .bind(ua_id)
    .bind(oa_id)
    .bind(pc)
    .bind(read_right)
    .execute(pool)
    .await
    .expect("insert association");

    let delegation_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_delegation \
         (fk_delegator, fk_delegatee, fk_user_attribute, date_st, date_ed, status) \
         VALUES ($1, $2, $3, $4::timestamptz, $5::timestamptz, $6) RETURNING id",
    )
    .bind(delegator)
    .bind(delegatee)
    .bind(ua_id)
    .bind(date_st)
    .bind(date_ed)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("insert delegation");

    (delegatee, row_id, delegation_id)
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn active_delegation_grants_delegatee_access() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;
    // ① active 且窗口内 ⇒ 放行
    let (delegatee, row_id, _d) = setup(
        &pool,
        "active",
        "2000-01-01T00:00:00Z",
        "2999-01-01T00:00:00Z",
    )
    .await;
    let res = require_resource_access(&pool, delegatee, RT, row_id, "read").await;
    assert!(
        res.is_ok(),
        "被委托人在 active 且窗口内 MUST 经委托 UA 放行（got={res:?}）"
    );
    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn revoked_delegation_denies_delegatee() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;
    // ② 已撤销 ⇒ 拒绝
    let (delegatee, row_id, _d) = setup(
        &pool,
        "revoked",
        "2000-01-01T00:00:00Z",
        "2999-01-01T00:00:00Z",
    )
    .await;
    let res = require_resource_access(&pool, delegatee, RT, row_id, "read").await;
    assert!(res.is_err(), "撤销后的委托 MUST NOT 授予（got={res:?}）");
    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn expired_delegation_window_denies_delegatee() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;
    // ③ 窗口已过 ⇒ 拒绝
    let (delegatee, row_id, _d) = setup(
        &pool,
        "active",
        "2000-01-01T00:00:00Z",
        "2000-01-02T00:00:00Z",
    )
    .await;
    let res = require_resource_access(&pool, delegatee, RT, row_id, "read").await;
    assert!(res.is_err(), "窗口已过的委托 MUST NOT 授予（got={res:?}）");
    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn disabled_delegator_stops_supplying_delegatee() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;
    // ④ 授权方禁用 ⇒ 停供；重新启用 ⇒ 恢复（委托行 status 不变）
    let (delegatee, row_id, _d) = setup(
        &pool,
        "active",
        "2000-01-01T00:00:00Z",
        "2999-01-01T00:00:00Z",
    )
    .await;
    // 基线：授权方活跃 ⇒ 放行
    let res = require_resource_access(&pool, delegatee, RT, row_id, "read").await;
    assert!(
        res.is_ok(),
        "授权方活跃时被委托人 MUST 经委托 UA 放行（got={res:?}）"
    );

    // 禁用授权方
    sqlx::query(
        "UPDATE isahl_auth.auth_users SET is_active = FALSE WHERE username = 'zz-dlg-delegator'",
    )
    .execute(&pool)
    .await
    .expect("disable delegator");
    let res = require_resource_access(&pool, delegatee, RT, row_id, "read").await;
    assert!(res.is_err(), "授权方禁用后委托 MUST 停供（got={res:?}）");

    // 委托行 status MUST 未被改写（停供只读账号态）
    let status: String =
        sqlx::query_scalar("SELECT status FROM isahl_auth.ngac_delegation WHERE fk_delegatee = $1")
            .bind(delegatee)
            .fetch_one(&pool)
            .await
            .expect("delegation status");
    assert_eq!(status, "active", "停供 MUST NOT 改写委托行 status");

    // 重新启用 ⇒ 恢复
    sqlx::query(
        "UPDATE isahl_auth.auth_users SET is_active = TRUE WHERE username = 'zz-dlg-delegator'",
    )
    .execute(&pool)
    .await
    .expect("re-enable delegator");
    let res = require_resource_access(&pool, delegatee, RT, row_id, "read").await;
    assert!(res.is_ok(), "授权方重新启用后 MUST 恢复放行（got={res:?}）");
    cleanup(&pool).await;
}
