//! Framework 内层判定的**条件门**回归守卫（NGAC_SPEC §2.4 + §2.5，change `consolidate-ngac-decision-convergence`）。
//!
//! 为什么需要：M16 收敛前，Framework 侧 `require_resource_access` **完全不评估 association 的
//! `conditions`**（仅评无条件 prohibition）——即「条件不满足的授权仍被放行」的**过授**方向。
//! 该方向是安全敏感的（漏判 = 越权），既有用例只覆盖 prohibition 侧（拒绝方向）。
//!
//! 本文件守护三类：
//! ① v1 时间窗未满足（`not_after` 已过）⇒ association **不授予**（Err）；
//! ② v1 时间窗满足（`not_before` 已过）⇒ **授予**（Ok；证明关联本身可用，①的拒绝来自条件）；
//! ③ v2 `object_attr_in` 未命中目标 OA 闭包名 ⇒ **不授予**（Err）。
//!
//! 口径：`TEST_DATABASE_URL="postgres://<user>@localhost:5432/aliothstudio_test"` + `--ignored`。

use common::ngac_grants::{ensure_resource_oa, ResourceOaSpec};
use common::permissions::require_resource_access;
use serde_json::json;
use sqlx::PgPool;

const RT: &str = "zz-cond";
const UA_NAME: &str = "zz-cond-ua";

async fn open_test_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    if url.is_none() {
        eprintln!("skipped: DATABASE_URL 未设置");
        return None;
    }
    Some(common::testing::connect_test_db().await)
}

async fn cleanup(pool: &PgPool) {
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
    let _ =
        sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE o_name LIKE 'zz-cond%'")
            .execute(pool)
            .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'zz-cond%'")
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username LIKE 'zz-cond%'")
        .execute(pool)
        .await;
}

/// 夹具：用户 + 直派 UA（闭包含该 UA）+ 行 OA + 一条带 `conditions` 的 association（`read`）。
/// 返回 (user_id, row_id, oa_name)。
async fn setup(pool: &PgPool, conditions: serde_json::Value) -> (i64, i64, String) {
    let user: i64 =
        sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id")
            .bind("zz-cond-user")
            .fetch_one(pool)
            .await
            .expect("insert user");

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
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (o_name, fk_user, fk_user_attribute) \
         VALUES ($1, $2, $3)",
    )
    .bind("zz-cond-rr")
    .bind(user)
    .bind(ua_id)
    .execute(pool)
    .await
    .expect("bind UA to user");

    // 行 OA（`fk_resource` 用固定哨兵值，避免依赖业务表；resource_identifier 供展示链）
    let row_id: i64 = 9_900_777;
    ensure_resource_oa(
        pool,
        &ResourceOaSpec {
            o_name: format!("{RT}-{row_id}"),
            resource_type: RT,
            resource_id: row_id,
            resource_identifier: Some("ZZ-COND"),
            created_by: Some(user),
            policy_class_name: Some("default"),
        },
    )
    .await
    .expect("ensure row OA");

    let read_right: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'read' \
         UNION ALL SELECT id FROM isahl_auth.ngac_access_right LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("read access right");
    let pc_default: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default'")
            .fetch_one(pool)
            .await
            .expect("default policy class");
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
         (o_name, fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights, conditions) \
         VALUES ($1, $2, $3, $4, ARRAY[$5::bigint], $6)",
    )
    .bind("zz-cond-assoc")
    .bind(ua_id)
    .bind(oa_id)
    .bind(pc_default)
    .bind(read_right)
    .bind(conditions)
    .execute(pool)
    .await
    .expect("insert conditional association");

    (user, row_id, format!("{RT}-{row_id}"))
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn association_with_unmet_time_window_does_not_grant() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;
    // ① 时间窗已过 ⇒ 不授予
    let (user, row_id, _oa) = setup(&pool, json!({ "not_after": "2000-01-01T00:00:00Z" })).await;
    let res = require_resource_access(&pool, user, RT, row_id, "read").await;
    assert!(
        res.is_err(),
        "条件未满足的 association MUST NOT 授予（got={res:?}）"
    );
    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn association_with_met_time_window_grants() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;
    // ② 时间窗已满足 ⇒ 授予（证明 ① 的拒绝来自条件而非关联失效）
    let (user, row_id, _oa) = setup(&pool, json!({ "not_before": "2000-01-01T00:00:00Z" })).await;
    let res = require_resource_access(&pool, user, RT, row_id, "read").await;
    assert!(
        res.is_ok(),
        "条件满足的 association MUST 授予（got={res:?}）"
    );
    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn association_with_unmet_object_attr_in_does_not_grant() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;
    // ③ v2 `object_attr_in` 未命中目标 OA 闭包名 ⇒ 不授予
    let (user, row_id, oa_name) =
        setup(&pool, json!({ "object_attr_in": ["zz-cond-not-this-oa"] })).await;
    let res = require_resource_access(&pool, user, RT, row_id, "read").await;
    assert!(
        res.is_err(),
        "object_attr_in 未命中 MUST NOT 授予（got={res:?}）"
    );

    // 正向对照：命中真实 OA 名 ⇒ 授予
    sqlx::query("UPDATE isahl_auth.ngac_association SET conditions = $1::jsonb WHERE o_name = $2")
        .bind(json!({ "object_attr_in": [oa_name] }))
        .bind("zz-cond-assoc")
        .execute(&pool)
        .await
        .expect("update conditions");
    let res_ok = require_resource_access(&pool, user, RT, row_id, "read").await;
    assert!(
        res_ok.is_ok(),
        "object_attr_in 命中真实 OA 名 MUST 授予（got={res_ok:?}）"
    );

    cleanup(&pool).await;
}
