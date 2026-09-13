//! Runtime tests for `require_resource_access` against a real PG.
//!
//! These tests connect to aliothstudio_test and exercise the actual helper
//! function. They require that the test fixture is in place — see
//! `docs/specs/NGAC_SPEC.md` for the bootstrap script:
//!   1. reset-db.sh --test --reset
//!   2. Apply 007_ngac_extension_tables.sql
//!   3. Apply 005_seed_ngac_and_policy_class.sql
//!   4. Bootstrap: 1 admin user (id=1002) + admin UA binding + foreign-trade module OA + association
//!
//! Run with:
//!   cargo test -p common --test permissions_runtime_test -- --ignored --test-threads=1

use common::permissions::require_resource_access;
use common::AliothError;
use sqlx::PgPool;

const ADMIN_USER_ID: i64 = 1002;
const FOREIGN_TRADE_OA_FK_RESOURCE: i64 = 1;

async fn connect() -> PgPool {
    {
        let database_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| common::testing::test_database_url());
        sqlx::PgPool::connect(&database_url)
            .await
            .expect("connect_test_db failed")
    }
}

#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn admin_can_admin_foreign_trade_module() {
    let pool = connect().await;
    let result = require_resource_access(
        &pool,
        ADMIN_USER_ID,
        "module",
        FOREIGN_TRADE_OA_FK_RESOURCE,
        "admin",
    )
    .await;
    assert!(
        result.is_ok(),
        "admin should be able to admin foreign-trade module, got: {:?}",
        result
    );
}

#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn admin_cannot_perform_unknown_action() {
    let pool = connect().await;
    let result = require_resource_access(
        &pool,
        ADMIN_USER_ID,
        "nonexistent-resource",
        99999,
        "delete",
    )
    .await;
    assert!(result.is_err(), "nonexistent resource should be denied");
}

#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn zero_user_id_is_denied() {
    let pool = connect().await;
    let result =
        require_resource_access(&pool, 0, "module", FOREIGN_TRADE_OA_FK_RESOURCE, "admin").await;
    assert!(result.is_err(), "user_id=0 should not pass any NGAC check");
}

// ── M218 framework-prohibition 决策序（deny-overrides）──
// 语义对齐 SSO PDP decide（NGAC_SPEC §6.2 / M218）：
//   a) 无条件 prohibition 命中 → Forbidden，对 admin 同样生效（先于 admin 兜底）；
//   b) admin 无 association/prohibition 命中 → admin 治理豁免兜底 Ok；
//   c) 条件式 prohibition（conditions 非空）Framework 不评估 → 不阻断无条件 association。
// 夹具完全自包含：测试专属 auth_users 用户 + admin UA（bootstrap o_name='admin'）
// 绑定 + 前缀（wt218_m218*）行。清理走前缀硬删——OA 的 uq_ngac_oa_resource 对
// (resource_type, fk_resource) 全行唯一（不随 deleted_at 释放），软删会阻塞重跑插
// 入，故本组自建行一律 DELETE（SSO 侧 ngac 测试同款先例）；前缀唯一保证不波及他行。

async fn m218_default_policy_class_id(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default'",
    )
    .fetch_one(pool)
    .await
    .expect("default policy class must be seeded (005_seed_ngac_and_policy_class.sql)")
}

async fn m218_admin_ua_id(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM isahl_auth.ngac_user_attribute \
         WHERE o_name = 'admin' AND deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await
    .expect("admin UA must be seeded (bootstrap)")
}

/// 前缀硬删清理（幂等，测试前后各一次）：prohibition → association → rr → OA →
/// access_right → auth_users（auth_users 无 deleted_at 列）。顺序遵守外键引用。
async fn m218_cleanup_fixture(pool: &PgPool, prefix: &str) {
    let pattern = format!("{prefix}%");
    sqlx::query("DELETE FROM isahl_auth.ngac_prohibition WHERE o_name LIKE $1")
        .bind(&pattern)
        .execute(pool)
        .await
        .expect("cleanup prohibitions");
    sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE o_name LIKE $1")
        .bind(&pattern)
        .execute(pool)
        .await
        .expect("cleanup associations");
    sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE o_name LIKE $1")
        .bind(&pattern)
        .execute(pool)
        .await
        .expect("cleanup UA bindings");
    sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE o_name LIKE $1")
        .bind(&pattern)
        .execute(pool)
        .await
        .expect("cleanup OAs");
    sqlx::query("DELETE FROM isahl_auth.ngac_access_right WHERE o_name LIKE $1")
        .bind(&pattern)
        .execute(pool)
        .await
        .expect("cleanup access rights");
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE name LIKE $1")
        .bind(&pattern)
        .execute(pool)
        .await
        .expect("cleanup fixture users");
}

/// 自建测试用户（fk_user → auth_users 外键约束）+ 绑定 admin UA。
async fn m218_bind_admin_user(pool: &PgPool, name: &str) -> i64 {
    let user_id: i64 =
        sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (name) VALUES ($1) RETURNING id")
            .bind(name)
            .fetch_one(pool)
            .await
            .expect("insert fixture user");
    let admin_ua = m218_admin_ua_id(pool).await;
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute \
            (o_name, fk_user, fk_user_attribute) \
         VALUES ($1, $2, $3)",
    )
    .bind(format!("{name}_bind"))
    .bind(user_id)
    .bind(admin_ua)
    .execute(pool)
    .await
    .expect("bind fixture user to admin UA");
    user_id
}

#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn m218_unconditional_prohibition_denies_admin_despite_association() {
    let pool = connect().await;
    const PREFIX: &str = "wt218_m218a";
    const RES_ID: i64 = 9_400_002;
    const RTYPE: &str = "wt218_m218a_rsrc";
    const ACTION: &str = "wt218_m218a_act";
    let oa_name = format!("{PREFIX}_oa");
    let assoc_name = format!("{PREFIX}_assoc");
    let proh_name = format!("{PREFIX}_proh");
    let ar_name = format!("{PREFIX}_act");
    let user_name = format!("{PREFIX}_user");

    m218_cleanup_fixture(&pool, PREFIX).await;
    let pc = m218_default_policy_class_id(&pool).await;
    let user = m218_bind_admin_user(&pool, &user_name).await;
    let admin_ua = m218_admin_ua_id(&pool).await;

    let oa_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_object_attribute \
            (o_name, fk_policy_class, resource_type, fk_resource) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(&oa_name)
    .bind(pc)
    .bind(RTYPE)
    .bind(RES_ID)
    .fetch_one(&pool)
    .await
    .expect("insert OA");
    let ar_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ($1) RETURNING id",
    )
    .bind(&ar_name)
    .fetch_one(&pool)
    .await
    .expect("insert access right");
    // 同 UA/OA 存在放行 association——prohibition 命中时应 deny-overrides 压过它
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_association \
            (o_name, fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights) \
         VALUES ($1, $2, $3, $4, ARRAY[$5])",
    )
    .bind(&assoc_name)
    .bind(admin_ua)
    .bind(oa_id)
    .bind(pc)
    .bind(ar_id)
    .execute(&pool)
    .await
    .expect("insert association");
    // 无条件 prohibition：conditions 缺省 '{}' → Framework 必须评估并命中
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_prohibition \
            (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active) \
         VALUES ($1, $2, $3, ARRAY[$4], TRUE)",
    )
    .bind(&proh_name)
    .bind(admin_ua)
    .bind(oa_id)
    .bind(ar_id)
    .execute(&pool)
    .await
    .expect("insert prohibition");

    let result = require_resource_access(&pool, user, RTYPE, RES_ID, ACTION).await;
    match result {
        Err(AliothError::Forbidden(_)) => {}
        other => panic!(
            "无条件 prohibition 应对 admin + association 仍 deny-overrides，got: {:?}",
            other
        ),
    }

    m218_cleanup_fixture(&pool, PREFIX).await;
}

#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn m218_admin_without_association_or_prohibition_ok() {
    let pool = connect().await;
    const PREFIX: &str = "wt218_m218b";
    const RES_ID: i64 = 9_400_012;
    const RTYPE: &str = "wt218_m218b_rsrc";
    const ACTION: &str = "wt218_m218b_act";
    let user_name = format!("{PREFIX}_user");

    m218_cleanup_fixture(&pool, PREFIX).await;
    let user = m218_bind_admin_user(&pool, &user_name).await;

    // 无 association、无 prohibition（唯一 action 名保证不可能被共享库他行命中）
    let result = require_resource_access(&pool, user, RTYPE, RES_ID, ACTION).await;
    assert!(
        result.is_ok(),
        "admin 兜底（无 association/prohibition 命中）应 Ok，got: {:?}",
        result
    );

    m218_cleanup_fixture(&pool, PREFIX).await;
}

#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn m218_conditional_prohibition_skipped_by_framework() {
    let pool = connect().await;
    const PREFIX: &str = "wt218_m218c";
    const RES_ID: i64 = 9_400_022;
    const RTYPE: &str = "wt218_m218c_rsrc";
    const ACTION: &str = "wt218_m218c_act";
    let oa_name = format!("{PREFIX}_oa");
    let assoc_name = format!("{PREFIX}_assoc");
    let proh_name = format!("{PREFIX}_proh");
    let ar_name = format!("{PREFIX}_act");
    let user_name = format!("{PREFIX}_user");

    m218_cleanup_fixture(&pool, PREFIX).await;
    let pc = m218_default_policy_class_id(&pool).await;
    let user = m218_bind_admin_user(&pool, &user_name).await;
    let admin_ua = m218_admin_ua_id(&pool).await;

    let oa_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_object_attribute \
            (o_name, fk_policy_class, resource_type, fk_resource) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(&oa_name)
    .bind(pc)
    .bind(RTYPE)
    .bind(RES_ID)
    .fetch_one(&pool)
    .await
    .expect("insert OA");
    let ar_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ($1) RETURNING id",
    )
    .bind(&ar_name)
    .fetch_one(&pool)
    .await
    .expect("insert access right");
    // 无条件 association → permitted 命中
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_association \
            (o_name, fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights) \
         VALUES ($1, $2, $3, $4, ARRAY[$5])",
    )
    .bind(&assoc_name)
    .bind(admin_ua)
    .bind(oa_id)
    .bind(pc)
    .bind(ar_id)
    .execute(&pool)
    .await
    .expect("insert association");
    // 条件式 prohibition（conditions 非空）——Framework 边界外（SSO PDP 评估），必须跳过
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_prohibition \
            (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active, conditions) \
         VALUES ($1, $2, $3, ARRAY[$4], TRUE, $5::jsonb)",
    )
    .bind(&proh_name)
    .bind(admin_ua)
    .bind(oa_id)
    .bind(ar_id)
    .bind("{\"x\": 1}")
    .execute(&pool)
    .await
    .expect("insert conditional prohibition");

    let result = require_resource_access(&pool, user, RTYPE, RES_ID, ACTION).await;
    assert!(
        result.is_ok(),
        "条件式 prohibition 不应阻断无条件 association，got: {:?}",
        result
    );

    m218_cleanup_fixture(&pool, PREFIX).await;
}
