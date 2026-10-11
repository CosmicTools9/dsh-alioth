//! `isahl_auth.effective_ua_ids`（判定链 PIP 段单源）的语义测试。
//!
//! 运行：`cargo test -p common --test ngac_effective_ua_test -- --ignored --test-threads=1`
//! 需要 `aliothstudio_test`（`bash scripts/db/reset-db.sh --test` 预置 NGAC 面）。
//! 夹具自建自清（仅动本文件创建的 UA/指派）。

use common::permissions::{
    effective_ua_ids_fn_ddl, ensure_ngac_predicate_fns, EFFECTIVE_UA_IDS_FN,
};
use common::testing::connect_test_db;
use sqlx::PgPool;
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};

/// 文件级串行化。
///
/// 三个测试共用**同一个 UA 名** `z-test-effective-ua` 与同一测试用户，且 `cleanup` 按该
/// UA 名清空 ⇒ 并行执行时一方清掉共享 UA/指派，另一方断言随机失败（实测 38/64/108 行随机红）。
/// 该场景刻意共享（同一 UA 的三态语义），故串行而非改名——并使本文件不再依赖
/// `--test-threads=1` 这一运行参数约束。
static SERIAL_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn lock_serial() -> MutexGuard<'static, ()> {
    SERIAL_LOCK.lock().await
}

const MARK: &str = "z";
const UA_NAME: &str = "z-test-effective-ua";

/// 建一个专用 UA + 对测试用户的指派（`expires_hours` = 相对现在的过期小时数，`None` = 永不过期），
/// 返回 (user_id, ua_id)。
async fn fixture(pool: &PgPool, expires_hours: Option<i32>) -> (i64, i64) {
    let user_id: i64 = sqlx::query_scalar(
        "SELECT ur.fk_user FROM isahl_auth.ngac_user_rr_attribute ur \
         JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute \
         WHERE ua.o_name = 'admin' AND ur.deleted_at IS NULL ORDER BY ur.fk_user LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("测试库需至少一个 admin UA 持有者：bash scripts/db/reset-db.sh --test");

    let ua_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_user_attribute (id, o_name, fk_policy_class, created_at) \
         SELECT isahl.gen_next_zuid(), $1, \
                (SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1), NOW() \
         WHERE NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL) \
         RETURNING id",
    )
    .bind(UA_NAME)
    .fetch_optional(pool)
    .await
    .expect("insert UA")
    .unwrap_or(0);
    let ua_id = if ua_id == 0 {
        sqlx::query_scalar::<_, i64>(
            "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL LIMIT 1",
        )
        .bind(UA_NAME)
        .fetch_one(pool)
        .await
        .expect("existing UA id")
    } else {
        ua_id
    };

    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute \
           (id, o_name, fk_user, fk_user_attribute, assigned_at, created_at, expires_at) \
         VALUES (isahl.gen_next_zuid(), $1, $2, $3, NOW(), NOW(), \
                 CASE WHEN $4::int IS NULL THEN NULL ELSE NOW() + make_interval(hours => $4::int) END)",
    )
    .bind(format!("{MARK}-assignment"))
    .bind(user_id)
    .bind(ua_id)
    .bind(expires_hours)
    .execute(pool)
    .await
    .expect("insert assignment");
    (user_id, ua_id)
}

async fn cleanup(pool: &PgPool, user_id: i64, ua_id: i64) {
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1 AND fk_user_attribute = $2")
        .bind(user_id)
        .bind(ua_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name = $1")
        .bind(UA_NAME)
        .execute(pool)
        .await;
}

fn contains(ids: &[i64], ua_id: i64) -> bool {
    ids.contains(&ua_id)
}

/// 直派（未过期）⇒ 进入有效 UA 集。
#[tokio::test]
#[ignore = "requires live DB (aliothstudio_test)"]
async fn direct_assignment_enters_effective_set() {
    let _guard = lock_serial().await;
    let pool = connect_test_db().await;
    ensure_ngac_predicate_fns(&pool).await.expect("ensure fns");
    let (user_id, ua_id) = fixture(&pool, None).await;
    let ids: Vec<i64> = sqlx::query_scalar("SELECT ua_id FROM isahl_auth.effective_ua_ids($1)")
        .bind(user_id)
        .fetch_all(&pool)
        .await
        .expect("effective_ua_ids");
    cleanup(&pool, user_id, ua_id).await;
    assert!(
        contains(&ids, ua_id),
        "未过期的直派指派 MUST 进入有效 UA 集"
    );
}

/// 过期指派 ⇒ 不进入有效 UA 集（NGAC_SPEC §2.2.3 末：过期指派被全部决策路径视为无效）。
#[tokio::test]
#[ignore = "requires live DB (aliothstudio_test)"]
async fn expired_assignment_excluded() {
    let _guard = lock_serial().await;
    let pool = connect_test_db().await;
    ensure_ngac_predicate_fns(&pool).await.expect("ensure fns");
    let (user_id, ua_id) = fixture(&pool, Some(-1)).await;
    let ids: Vec<i64> = sqlx::query_scalar("SELECT ua_id FROM isahl_auth.effective_ua_ids($1)")
        .bind(user_id)
        .fetch_all(&pool)
        .await
        .expect("effective_ua_ids");
    cleanup(&pool, user_id, ua_id).await;
    assert!(
        !contains(&ids, ua_id),
        "已过期的指派 MUST NOT 进入有效 UA 集"
    );
}

/// `is_admin` MUST 建立在有效 UA 集之上（链内唯一：DDL 文本引用该函数名）。
#[tokio::test]
#[ignore = "requires live DB (aliothstudio_test)"]
async fn is_admin_is_built_on_effective_ua_ids() {
    let _guard = lock_serial().await;
    let pool = connect_test_db().await;
    ensure_ngac_predicate_fns(&pool).await.expect("ensure fns");
    let ddl = common::permissions::admin_predicate_fn_ddl();
    assert!(
        ddl.contains(EFFECTIVE_UA_IDS_FN),
        "is_admin 的 DDL MUST 引用 {EFFECTIVE_UA_IDS_FN}（同一判定链）"
    );
    assert!(
        effective_ua_ids_fn_ddl().contains("ngac_user_rr_attribute"),
        "有效 UA 集函数 MUST 含直派查询（链基座）"
    );
    // 运行时口径：库内 admin UA 持有者经有效 UA 集仍判为 admin
    let admin_id: i64 = sqlx::query_scalar(
        "SELECT ur.fk_user FROM isahl_auth.ngac_user_rr_attribute ur \
         JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute \
         WHERE ua.o_name = 'admin' AND ur.deleted_at IS NULL ORDER BY ur.fk_user LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("admin holder");
    assert!(common::permissions::is_admin_user(&pool, admin_id)
        .await
        .unwrap());
    assert!(!common::permissions::is_admin_user(&pool, 987_654_321)
        .await
        .unwrap());
}
