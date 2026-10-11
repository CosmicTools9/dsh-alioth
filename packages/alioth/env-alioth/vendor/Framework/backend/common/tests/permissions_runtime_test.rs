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
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};

/// 文件级串行化。
///
/// 11 个测试共用同一 `ADMIN_USER_ID`（1002）与 m218 夹具前缀：`admin_can_admin_foreign_trade_module`
/// 会为该用户**建立 admin 绑定**，而 `admin_cannot_perform_unknown_action` 断言该用户对未知资源
/// 被拒（仅在**非 admin** 时成立——admin 治理豁免会兜底放行）。并行执行 ⇒ 两测试互相翻转前提
///（实测 64/128 行随机红）。故串行而非改名。
static SERIAL_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn lock_serial() -> MutexGuard<'static, ()> {
    SERIAL_LOCK.lock().await
}

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
    let _guard = lock_serial().await;
    let pool = connect().await;
    ensure_admin_binding(&pool, ADMIN_USER_ID).await;
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
    m218_cleanup_fixture(&pool, "wt218_adm").await;
}

#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn admin_cannot_perform_unknown_action() {
    let _guard = lock_serial().await;
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
    let _guard = lock_serial().await;
    let pool = connect().await;
    let result =
        require_resource_access(&pool, 0, "module", FOREIGN_TRADE_OA_FK_RESOURCE, "admin").await;
    assert!(result.is_err(), "user_id=0 should not pass any NGAC check");
}

// ── M218 framework-prohibition 决策序（deny-overrides）──
// 语义对齐 SSO PDP decide（NGAC_SPEC §2.4/§2.5）：
//   a) 无条件 prohibition 命中 → Forbidden，对 admin 同样生效（先于 admin 兜底）；
//   b) admin 无 association/prohibition 命中 → admin 治理豁免兜底 Ok；
//   c) conditions 求值唯一实现 = `common::ngac_org::evaluate_conditions`（M16 判定链同源）：
//      **条件满足**的 prohibition 命中 → Forbidden；**条件未满足** → 该策略不参与本次决策
//      （不阻断同一资源上的 association，也不构成拒绝）。
// 夹具完全自包含：测试专属 auth_users 用户 + admin UA（bootstrap o_name='admin'）
// 绑定 + 前缀（wt218_m218*）行。清理走前缀硬删——OA 的 uq_ngac_oa_resource 对
// (resource_type, fk_resource) 全行唯一（不随 deleted_at 释放），软删会阻塞重跑插
// 入，故本组自建行一律 DELETE（SSO 侧 ngac 测试同款先例）；前缀唯一保证不波及他行。

/// 确保 `user_id` 绑定到 admin UA（幂等；绑定行前缀 `wt218_adm_`，供清理）。
///
/// 为什么测试自备：本机 `aliothstudio_test` 的 bootstrap 绑定行可能缺失
/// （2026-09-30 实测 `fk_user=1002 → admin UA` 绑定行 = 0 ⇒ `isahl_auth.is_admin(1002)=false`），
/// 而 `is_admin_user` 与 admin 治理兜底的断言 MUST 只依赖被测实现，MUST NOT 依赖外部 seed 状态。
async fn ensure_admin_binding(pool: &PgPool, user_id: i64) -> i64 {
    // 用户行缺失 ⇒ 自备（幂等）：test 库重建后 bootstrap 夹具常已丢失（实测
    // `auth_users.id=1002` 不存在 ⇒ 绑定插入撞 FK `ngac_user_rr_attribute_fk_user_fkey`）。
    // 行名带 `wt218_adm_` 前缀，随测试末尾的 `m218_cleanup_fixture` 一并硬删。
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, username) VALUES ($1, $2) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(user_id)
    .bind(format!("wt218_adm_{user_id}"))
    .execute(pool)
    .await
    .expect("ensure fixture user");
    let ua = m218_admin_ua_id(pool).await;
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user = $1 AND fk_user_attribute = $2 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(user_id)
    .bind(ua)
    .fetch_optional(pool)
    .await
    .expect("read admin binding");
    if let Some(id) = existing {
        return id;
    }
    sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (o_name, fk_user, fk_user_attribute) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("wt218_adm_bind_{user_id}"))
    .bind(user_id)
    .bind(ua)
    .fetch_one(pool)
    .await
    .expect("insert admin binding")
}

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
    // 自建 UA 行（`m16_bind_regular_user` 的 `{name}_ua`）MUST 一并清理：UA 的
    // `(o_name, fk_policy_class) WHERE deleted_at IS NULL` 唯一索引不随软删释放，
    // 残留即令重跑插入撞 23505（实测：清理缺失导致第二次运行 `insert fixture UA` 失败）。
    sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE $1")
        .bind(&pattern)
        .execute(pool)
        .await
        .expect("cleanup UAs");
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
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username LIKE $1")
        .bind(&pattern)
        .execute(pool)
        .await
        .expect("cleanup fixture users");
}

/// 自建测试用户（fk_user → auth_users 外键约束）+ 绑定 admin UA。
async fn m218_bind_admin_user(pool: &PgPool, name: &str) -> i64 {
    let user_id: i64 =
        sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id")
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
    let _guard = lock_serial().await;
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
    let _guard = lock_serial().await;
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
async fn m16_unmet_condition_prohibition_is_skipped() {
    let _guard = lock_serial().await;
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
    // 条件式 prohibition：`not_before` 在未来 ⇒ 条件**不满足** ⇒ 该策略不参与决策
    // （不阻断无条件 association；M16 起 Framework 与 SSO PDP 同源求值）
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_prohibition \
            (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active, conditions) \
         VALUES ($1, $2, $3, ARRAY[$4], TRUE, $5::jsonb)",
    )
    .bind(&proh_name)
    .bind(admin_ua)
    .bind(oa_id)
    .bind(ar_id)
    .bind("{\"not_before\": \"2099-01-01T00:00:00Z\"}")
    .execute(&pool)
    .await
    .expect("insert conditional prohibition");

    let result = require_resource_access(&pool, user, RTYPE, RES_ID, ACTION).await;
    assert!(
        result.is_ok(),
        "条件未满足的 prohibition MUST 不参与决策（不阻断 association），got: {:?}",
        result
    );

    m218_cleanup_fixture(&pool, PREFIX).await;
}

/// M16（判定链同源）：条件**满足**的 prohibition 命中 → Forbidden（v1 时间窗）。
#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn m16_met_condition_prohibition_denies() {
    let _guard = lock_serial().await;
    let pool = connect().await;
    const PREFIX: &str = "wt218_m16e";
    const RES_ID: i64 = 9_400_042;
    const RTYPE: &str = "wt218_m16e_rsrc";
    const ACTION: &str = "wt218_m16e_act";
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
    .bind(format!("{PREFIX}_oa"))
    .bind(pc)
    .bind(RTYPE)
    .bind(RES_ID)
    .fetch_one(&pool)
    .await
    .expect("insert OA");
    let ar_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ($1) RETURNING id",
    )
    .bind(ACTION)
    .fetch_one(&pool)
    .await
    .expect("insert access right");
    // association 放行 + conditions 满足的 prohibition（not_before 已过）→ deny-overrides
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_association \
            (o_name, fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights) \
         VALUES ($1, $2, $3, $4, ARRAY[$5])",
    )
    .bind(format!("{PREFIX}_assoc"))
    .bind(admin_ua)
    .bind(oa_id)
    .bind(pc)
    .bind(ar_id)
    .execute(&pool)
    .await
    .expect("insert association");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_prohibition \
            (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active, conditions) \
         VALUES ($1, $2, $3, ARRAY[$4], TRUE, $5::jsonb)",
    )
    .bind(format!("{PREFIX}_proh"))
    .bind(admin_ua)
    .bind(oa_id)
    .bind(ar_id)
    .bind("{\"not_before\": \"2000-01-01T00:00:00Z\"}")
    .execute(&pool)
    .await
    .expect("insert prohibition");

    match require_resource_access(&pool, user, RTYPE, RES_ID, ACTION).await {
        Err(AliothError::Forbidden(_)) => {}
        other => panic!(
            "条件满足的 prohibition MUST deny-overrides（对 admin 同样生效），got: {:?}",
            other
        ),
    }

    m218_cleanup_fixture(&pool, PREFIX).await;
}

/// M16（判定链同源）：v2 条件 `user_attr_in` 双向——命中（用户有效 UA 名含目标）→ 拒绝；
/// 未命中（集合内 UA 名用户不持有）→ 策略不参与决策。
#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn m16_v2_user_attr_in_gates_prohibition() {
    let _guard = lock_serial().await;
    let pool = connect().await;
    const PREFIX: &str = "wt218_m16f";
    const RES_ID: i64 = 9_400_052;
    const RTYPE: &str = "wt218_m16f_rsrc";
    const ACTION: &str = "wt218_m16f_act";
    let proh_name = format!("{PREFIX}_proh");
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
    .bind(format!("{PREFIX}_oa"))
    .bind(pc)
    .bind(RTYPE)
    .bind(RES_ID)
    .fetch_one(&pool)
    .await
    .expect("insert OA");
    let ar_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ($1) RETURNING id",
    )
    .bind(ACTION)
    .fetch_one(&pool)
    .await
    .expect("insert access right");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_prohibition \
            (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active, conditions) \
         VALUES ($1, $2, $3, ARRAY[$4], TRUE, $5::jsonb)",
    )
    .bind(&proh_name)
    .bind(admin_ua)
    .bind(oa_id)
    .bind(ar_id)
    .bind("{\"user_attr_in\": [\"admin\"]}")
    .execute(&pool)
    .await
    .expect("insert prohibition");

    // 用户有效 UA 名含 `admin`（夹具绑定）⇒ user_attr_in 满足 ⇒ 拒绝
    match require_resource_access(&pool, user, RTYPE, RES_ID, ACTION).await {
        Err(AliothError::Forbidden(_)) => {}
        other => panic!(
            "user_attr_in 命中 MUST 使 prohibition 生效，got: {:?}",
            other
        ),
    }

    // 条件改为不含用户持有 UA 名 ⇒ 不满足 ⇒ 策略跳过（无 association 时由 admin 兜底 Ok）
    sqlx::query("UPDATE isahl_auth.ngac_prohibition SET conditions = $1::jsonb WHERE o_name = $2")
        .bind("{\"user_attr_in\": [\"wt218_m16f_nonexistent\"]}")
        .bind(&proh_name)
        .execute(&pool)
        .await
        .expect("update conditions");
    assert!(
        require_resource_access(&pool, user, RTYPE, RES_ID, ACTION)
            .await
            .is_ok(),
        "user_attr_in 未命中 MUST 使 prohibition 跳过（交 admin 遍历后兜底）"
    );

    m218_cleanup_fixture(&pool, PREFIX).await;
}

/// 自建非 admin 用户 + 专属 UA（前缀命名，随 `m218_cleanup_fixture` 一起硬删）。
async fn m16_bind_regular_user(pool: &PgPool, name: &str, pc: i64) -> (i64, i64) {
    let user_id: i64 =
        sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id")
            .bind(name)
            .fetch_one(pool)
            .await
            .expect("insert fixture user");
    let ua_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class) \
         VALUES ($1, $2) RETURNING id",
    )
    .bind(format!("{name}_ua"))
    .bind(pc)
    .fetch_one(pool)
    .await
    .expect("insert fixture UA");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (o_name, fk_user, fk_user_attribute) \
         VALUES ($1, $2, $3)",
    )
    .bind(format!("{name}_bind"))
    .bind(user_id)
    .bind(ua_id)
    .execute(pool)
    .await
    .expect("bind fixture user");
    (user_id, ua_id)
}

/// M16：删除 `resource_attrs` 隐式 `fk_resource=0` 回落分支后的行为（NGAC_SPEC §7.3
/// 「行级 OA 缺失 ⇒ 该行不可见/不可判」）——集合级（`fk_resource = 0`）授予**不**外溢到
/// 未建模行 id；回落语义只保留在显式二元入口 [`common::permissions::require_row_or_collection_access`]。
#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn m16_collection_grant_does_not_fall_back_to_row_id() {
    let _guard = lock_serial().await;
    let pool = connect().await;
    const PREFIX: &str = "wt218_m16g";
    const RTYPE: &str = "wt218_m16g_rsrc";
    const ROW_ID: i64 = 9_400_062;
    let action = format!("{PREFIX}_act");
    let user_name = format!("{PREFIX}_user");

    m218_cleanup_fixture(&pool, PREFIX).await;
    let pc = m218_default_policy_class_id(&pool).await;
    let (user, ua) = m16_bind_regular_user(&pool, &user_name, pc).await;

    let coll_oa: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_object_attribute \
            (o_name, fk_policy_class, resource_type, fk_resource) \
         VALUES ($1, $2, $3, 0) RETURNING id",
    )
    .bind(format!("{PREFIX}_coll"))
    .bind(pc)
    .bind(RTYPE)
    .fetch_one(&pool)
    .await
    .expect("insert collection OA");
    let ar_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ($1) RETURNING id",
    )
    .bind(&action)
    .fetch_one(&pool)
    .await
    .expect("insert access right");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_association \
            (o_name, fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights) \
         VALUES ($1, $2, $3, $4, ARRAY[$5])",
    )
    .bind(format!("{PREFIX}_assoc"))
    .bind(ua)
    .bind(coll_oa)
    .bind(pc)
    .bind(ar_id)
    .execute(&pool)
    .await
    .expect("insert association");

    // 集合级判定（fk_resource = 0）命中 → Ok
    assert!(
        require_resource_access(&pool, user, RTYPE, 0, &action)
            .await
            .is_ok(),
        "集合级 OA 上的 association MUST 放行 collection 判定"
    );
    // 行级判定：该行无行级 OA ⇒ 回落分支已删 ⇒ Forbidden（非 admin）
    match require_resource_access(&pool, user, RTYPE, ROW_ID, &action).await {
        Err(AliothError::Forbidden(_)) => {}
        other => panic!(
            "行级 OA 缺失时 MUST NOT 回落集合 OA（NGAC_SPEC §7.3），got: {:?}",
            other
        ),
    }
    // 显式二元回落入口仍放行（回落语义唯一保留点）
    assert!(
        common::permissions::require_row_or_collection_access(&pool, user, RTYPE, ROW_ID, &action)
            .await
            .is_ok(),
        "require_row_or_collection_access 的显式集合级回落 MUST 保留"
    );

    m218_cleanup_fixture(&pool, PREFIX).await;
}

// ── is_admin_user（admin 判定唯一实现，change consolidate-admin-approval-queue）──
//
// `require_resource_access` 的 admin 治理兜底与富化实例端点的 `scope=all` 门禁
// 共用本函数（提取自前者内联 SQL，行为不变）。

#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn is_admin_user_true_for_admin_ua_holder() {
    let _guard = lock_serial().await;
    let pool = connect().await;
    ensure_admin_binding(&pool, ADMIN_USER_ID).await;
    assert!(
        common::permissions::is_admin_user(&pool, ADMIN_USER_ID)
            .await
            .expect("admin 判定不应报错"),
        "bootstrap admin 用户应判定为 admin"
    );
    m218_cleanup_fixture(&pool, "wt218_adm").await;
}

#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn is_admin_user_false_without_admin_ua() {
    let _guard = lock_serial().await;
    let pool = connect().await;
    assert!(
        !common::permissions::is_admin_user(&pool, -999_999)
            .await
            .expect("admin 判定不应报错"),
        "无 admin UA 的用户 MUST NOT 判定为 admin（fail-closed）"
    );
}

// ── 判定链 SQL 可执行性兜底（change `fix-permission-check-double-union` 防再犯）──
//
// 本文件其余用例均 `#[ignore]`（依赖特定 NGAC 夹具）；本用例**默认参与** `cargo test`——它不依赖任何夹具
// （任意入参，只判「非 Internal」），仅在测试库不可达 / `isahl_auth` 缺失时**自跳过**（那两类情形下
// `require_resource_access` 走早返回，断言无从生效 ⇒ 跳过并打印理由，不做假绿）。
//
// 判据由来：模板 + 片段 `format!` 拼接的三类缺陷（冗余 UNION / 占位符字面写进注释被注入 / 注释含裸双引号）
// 在编译期与常规静态检查下都不可见，只在**调用**时以 `AliothError::Internal` 暴露（生产表现为判定链整体 500）。
// 源码级不变量见 `src/permissions.rs::sql_template_invariants`（同一 change，互补）。

#[tokio::test]
async fn judgment_sql_never_yields_internal_error() {
    let Some(pool) = common::testing::try_connect_test_db().await else {
        eprintln!("跳过：测试库不可达（try_connect_test_db 返回 None）");
        return;
    };
    let has_schema: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT FROM information_schema.schemata WHERE schema_name='isahl_auth')",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(false);
    if !has_schema {
        eprintln!("跳过：测试库缺 isahl_auth 面（判定函数走早返回，断言无从生效）");
        return;
    }
    // 判定链 PIP 段函数（`effective_ua_ids`/`is_admin`）由 ensure 幂等建出；NGAC 扩展表（`ngac_delegation` 等）
    // 由 **SSO 启动期** ensure 建出（`SSO/backend/src/ngac/ensure.rs`，`common` 不可达）⇒ 测试库未预置扩展表时
    // 本用例自跳过（打印理由），待 `reset-db.sh --test` + NGAC 种子预置后自动生效；亦可用
    // `DATABASE_URL=<已就绪库> cargo test -p common --test permissions_runtime_test judgment_sql_never_yields_internal_error` 实跑。
    if let Err(e) = common::permissions::ensure_ngac_predicate_fns(&pool).await {
        eprintln!("跳过：判定链函数面 ensure 失败（{e}）——夹具不完整，本用例只守 SQL 拼装/语法面");
        return;
    }
    for (resource_type, resource_id) in [
        ("no-such-resource-type", 0_i64),
        ("no-such-resource-type", -1_i64),
    ] {
        match require_resource_access(&pool, -999_999, resource_type, resource_id, "read").await {
            Ok(()) | Err(AliothError::Forbidden(_)) => {}
            Err(e) => panic!(
                "判定链 MUST NOT 返回非 Forbidden 错误（SQL 拼装回归？resource_type={resource_type}）: {e}"
            ),
        }
    }
}

/// 指派级条件接线（`wire-ngac-zero-consumer-faces`，2026-10-10）：`ngac_user_rr_attribute.conditions`
/// 参与有效 UA 解析——时间窗满足 ⇒ 保留；不满足 / 值非法 / 含解析点不可求值键（`rhai` 等）⇒ 丢弃（失败封闭）。
/// 写径键集契约由 `SSO/ngac/pip.rs::validate_assignment_conditions` 同批强制（400），二者合并即完整契约。
#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn assignment_conditions_gate_effective_ua_set() {
    let _guard = lock_serial().await;
    let pool = connect().await;
    if let Err(e) = common::permissions::ensure_ngac_predicate_fns(&pool).await {
        eprintln!("跳过：判定链函数面 ensure 失败（{e}）——夹具不完整");
        return;
    }
    let uid: i64 = sqlx::query_scalar("SELECT isahl.gen_next_zuid()")
        .fetch_one(&pool)
        .await
        .expect("zuid");
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, username, status, is_active, created_at, updated_at) \
         VALUES ($1, $2, 'active', true, NOW(), NOW())",
    )
    .bind(uid)
    .bind(format!("m218cond_{uid}"))
    .execute(&pool)
    .await
    .expect("user");

    let cases: [(&str, serde_json::Value, bool); 6] = [
        ("m218cond_ok", serde_json::json!({}), true),
        (
            "m218cond_window_ok",
            serde_json::json!({ "not_before": "2000-01-01T00:00:00Z", "not_after": "2999-01-01T00:00:00Z" }),
            true,
        ),
        (
            "m218cond_future",
            serde_json::json!({ "not_before": "2999-01-01T00:00:00Z" }),
            false,
        ),
        (
            "m218cond_expired",
            serde_json::json!({ "not_after": "2000-01-01T00:00:00Z" }),
            false,
        ),
        // 解析点不可求值键（`rhai`）：失败封闭 ⇒ 该指派不生效
        (
            "m218cond_rhai",
            serde_json::json!({ "rhai": "true" }),
            false,
        ),
        // 时间值非法 ⇒ 不满足
        (
            "m218cond_badts",
            serde_json::json!({ "not_before": "not-a-date" }),
            false,
        ),
    ];
    let mut expected_in: Vec<i64> = Vec::new();
    for (name, cond, in_set) in &cases {
        let ua_id: i64 = sqlx::query_scalar(
            "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at) \
             VALUES ($1, (SELECT MIN(id) FROM isahl_auth.ngac_policy_class), NOW(), NOW()) RETURNING id",
        )
        .bind(format!("{name}_{uid}"))
        .fetch_one(&pool)
        .await
        .expect("ua");
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_user_rr_attribute \
             (fk_user, fk_user_attribute, conditions, created_at, updated_at) \
             VALUES ($1, $2, $3, NOW(), NOW())",
        )
        .bind(uid)
        .bind(ua_id)
        .bind(cond)
        .execute(&pool)
        .await
        .expect("rr");
        let present: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM isahl_auth.effective_ua_ids($1) e WHERE e.ua_id = $2)",
        )
        .bind(uid)
        .bind(ua_id)
        .fetch_one(&pool)
        .await
        .expect("query");
        if *in_set {
            expected_in.push(ua_id);
        } else {
            assert!(
                !present,
                "UA {name} 条件不满足 MUST NOT 进入有效集（失败封闭）"
            );
        }
    }
    for ua_id in &expected_in {
        let present: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM isahl_auth.effective_ua_ids($1) e WHERE e.ua_id = $2)",
        )
        .bind(uid)
        .bind(ua_id)
        .fetch_one(&pool)
        .await
        .expect("query");
        assert!(present, "UA {ua_id} 条件满足 MUST 进入有效集");
    }

    sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
        .bind(uid)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE $1")
        .bind(format!("%_{uid}"))
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(uid)
        .execute(&pool)
        .await
        .ok();
}
