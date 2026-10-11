//! `common::ngac_grants`（授权写入单源）的语义测试。
//!
//! derive-creator-grants-from-lifecycle 后的 API 面：`ensure_resource_oa`（锚点单行）+
//! `grant_rights(ua_name, …)`（具名 UA 授权；创建者授权已派生化，无 CreatorDirect）。
//! 覆盖：策略类选择（首个类 vs 具名 `default`、不存在不写 OA）、幂等（重跑零新增/零改写）、
//! right 集（`RIGHTS_CREATOR` / `RIGHTS_ADMIN`）、**加性 union**（同 `(UA, OA, class)` 重授取并集；
//! 软删行不复活）、事务入口（`&mut *conn` 重借）。
//!
//! 运行：`bash scripts/cargo-test.sh -p common --test ngac_grants_test -- --ignored --test-threads=1`
//! 需要 `aliothstudio_test`（`bash scripts/db/reset-db.sh --test` 预置 NGAC 面）。
//! 夹具自建自清（仅动本文件创建的资源类型 `z-test-grants-*`）。

use common::ngac_grants::{
    ensure_resource_oa, ensure_resource_oa_tx, grant_rights, ResourceOaSpec,
};
use common::testing::connect_test_db;
use ngac_host::{ADMIN_UA_NAME, RIGHTS_ADMIN, RIGHTS_CREATOR};
use sqlx::PgPool;
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};

/// 文件级串行化。
///
/// 4 个测试共用同一组资源类型（`RT_PLAIN`/`RT_IDENT`/`RT_NAMED_PC`/`RT_MISSING_PC`）且
/// `cleanup(pool)` 按该组**全局**清空 ⇒ 并行执行时一方删掉另一方的 OA/association 行
///（实测 136/183/210 行随机红）。共享是设计选择（同一资源的多入口语义），故串行而非改名。
static SERIAL_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn lock_serial() -> MutexGuard<'static, ()> {
    SERIAL_LOCK.lock().await
}

const RT_PLAIN: &str = "z-test-grants-plain";
const RT_IDENT: &str = "z-test-grants-ident";
const RT_NAMED_PC: &str = "z-test-grants-named-pc";
const RT_MISSING_PC: &str = "z-test-grants-missing-pc";

fn names(rights: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = rights.iter().map(|r| (*r).to_string()).collect();
    v.sort();
    v
}

async fn admin_user(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT ur.fk_user FROM isahl_auth.ngac_user_rr_attribute ur \
         JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute \
         WHERE ua.o_name = 'admin' AND ur.deleted_at IS NULL ORDER BY ur.fk_user LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("测试库需至少一个 admin UA 持有者：bash scripts/db/reset-db.sh --test")
}

async fn cleanup(pool: &PgPool) {
    for rt in [RT_PLAIN, RT_IDENT, RT_NAMED_PC, RT_MISSING_PC] {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type = $1",
        )
        .bind(rt)
        .fetch_all(pool)
        .await
        .unwrap();
        for id in ids {
            sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE fk_object_attribute = $1")
                .bind(id)
                .execute(pool)
                .await
                .unwrap();
            sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE id = $1")
                .bind(id)
                .execute(pool)
                .await
                .unwrap();
        }
    }
}

async fn oa_rows(pool: &PgPool, rt: &str, rid: i64) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.ngac_object_attribute \
         WHERE resource_type = $1 AND fk_resource = $2 AND deleted_at IS NULL",
    )
    .bind(rt)
    .bind(rid)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn assoc_rows(pool: &PgPool, rt: &str, rid: i64) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.ngac_association a \
         JOIN isahl_auth.ngac_object_attribute oa ON oa.id = a.fk_object_attribute \
         WHERE oa.resource_type = $1 AND oa.fk_resource = $2 AND a.deleted_at IS NULL",
    )
    .bind(rt)
    .bind(rid)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 该资源上全部活 association 的 right 集（升序字符串，去重后集合化）。
async fn right_sets(pool: &PgPool, rt: &str, rid: i64) -> Vec<Vec<String>> {
    sqlx::query_scalar(
        "SELECT array_agg(r.o_name ORDER BY r.o_name) FROM isahl_auth.ngac_association a \
           JOIN isahl_auth.ngac_object_attribute oa ON oa.id = a.fk_object_attribute \
           LEFT JOIN isahl_auth.ngac_access_right r ON r.id = ANY(a.ak_access_rights) \
         WHERE oa.resource_type = $1 AND oa.fk_resource = $2 AND a.deleted_at IS NULL \
         GROUP BY a.id",
    )
    .bind(rt)
    .bind(rid)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn has_right_set(pool: &PgPool, rt: &str, rid: i64, rights: &[&str]) -> bool {
    let want = names(rights);
    right_sets(pool, rt, rid).await.contains(&want)
}

#[tokio::test]
#[ignore = "需 aliothstudio_test；reset-db.sh --test 后运行"]
async fn anchor_and_ua_grant_idempotency() {
    let _guard = lock_serial().await;
    let pool = connect_test_db().await;
    cleanup(&pool).await;
    let user = admin_user(&pool).await;
    let rid = 9_900_001_i64;

    let spec = ResourceOaSpec {
        o_name: format!("{RT_PLAIN}-{rid}"),
        resource_type: RT_PLAIN,
        resource_id: rid,
        resource_identifier: None,
        created_by: Some(user),
        policy_class_name: None,
    };
    ensure_resource_oa(&pool, &spec)
        .await
        .expect("anchor")
        .expect("OA 应存在");
    grant_rights(&pool, RT_PLAIN, rid, ADMIN_UA_NAME, RIGHTS_CREATOR)
        .await
        .expect("ua grant");
    assert_eq!(oa_rows(&pool, RT_PLAIN, rid).await, 1, "锚点应恰一行");
    assert!(
        has_right_set(&pool, RT_PLAIN, rid, RIGHTS_CREATOR).await,
        "应存在 RIGHTS_CREATOR 集的 association"
    );
    let assoc_after_first = assoc_rows(&pool, RT_PLAIN, rid).await;

    // 幂等：重跑零新增、零改写（union 已包含 ⇒ 不写）
    ensure_resource_oa(&pool, &spec).await.expect("re-anchor");
    grant_rights(&pool, RT_PLAIN, rid, ADMIN_UA_NAME, RIGHTS_CREATOR)
        .await
        .expect("re-grant");
    assert_eq!(
        oa_rows(&pool, RT_PLAIN, rid).await,
        1,
        "重跑 MUST NOT 新增锚点"
    );
    assert_eq!(
        assoc_rows(&pool, RT_PLAIN, rid).await,
        assoc_after_first,
        "重跑 MUST NOT 新增 association"
    );
    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "需 aliothstudio_test；reset-db.sh --test 后运行"]
async fn grants_identifier_admin_and_union() {
    let _guard = lock_serial().await;
    let pool = connect_test_db().await;
    cleanup(&pool).await;
    let user = admin_user(&pool).await;
    let rid = 9_900_002_i64;

    let spec = ResourceOaSpec {
        o_name: format!("{RT_IDENT}-{rid}"),
        resource_type: RT_IDENT,
        resource_id: rid,
        resource_identifier: Some("ident-value"),
        created_by: Some(user),
        policy_class_name: None,
    };
    let oa_id = ensure_resource_oa(&pool, &spec)
        .await
        .expect("ensure oa")
        .expect("OA 应存在");
    let ident: Option<String> = sqlx::query_scalar(
        "SELECT resource_identifier FROM isahl_auth.ngac_object_attribute WHERE id = $1",
    )
    .bind(oa_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ident.as_deref(), Some("ident-value"), "标识列应写入");

    // admin 全量授权（UaName）：与上面 CREATOR 集同 `(UA, OA, class)` 行，
    // union 语义下终态 MUST 含 approve（原 `DO NOTHING` 首写者胜会丢 approve）。
    grant_rights(&pool, RT_IDENT, rid, ADMIN_UA_NAME, RIGHTS_ADMIN)
        .await
        .expect("admin grant");
    assert!(
        has_right_set(&pool, RT_IDENT, rid, RIGHTS_ADMIN).await,
        "admin UA 行 MUST 落到 RIGHTS_ADMIN 集（含 approve）"
    );
    assert_eq!(
        RIGHTS_ADMIN.len(),
        RIGHTS_CREATOR.len() + 1,
        "RIGHTS_ADMIN = RIGHTS_CREATOR + approve"
    );

    // 加性：再次授予同一 admin 集 ⇒ 零改写、零新增
    let before = assoc_rows(&pool, RT_IDENT, rid).await;
    let sets_before = right_sets(&pool, RT_IDENT, rid).await;
    grant_rights(&pool, RT_IDENT, rid, ADMIN_UA_NAME, RIGHTS_ADMIN)
        .await
        .expect("re-grant");
    assert_eq!(
        assoc_rows(&pool, RT_IDENT, rid).await,
        before,
        "重复授予零新增"
    );
    assert_eq!(
        right_sets(&pool, RT_IDENT, rid).await,
        sets_before,
        "重复授予零改写（已包含 ⇒ 不写）"
    );
    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "需 aliothstudio_test；reset-db.sh --test 后运行"]
async fn grants_policy_class_and_tx_entry() {
    let _guard = lock_serial().await;
    let pool = connect_test_db().await;
    cleanup(&pool).await;
    let user = admin_user(&pool).await;

    // 具名 'default' 类存在 ⇒ 写 OA
    let spec = ResourceOaSpec {
        o_name: format!("{RT_NAMED_PC}-1"),
        resource_type: RT_NAMED_PC,
        resource_id: 9_900_003,
        resource_identifier: Some("named"),
        created_by: Some(user),
        policy_class_name: Some("default"),
    };
    assert!(
        ensure_resource_oa(&pool, &spec).await.unwrap().is_some(),
        "具名 default 类应写 OA"
    );

    // 事务入口（`&mut *conn` 重借）与池入口等价；事务内授权随回滚撤销
    let mut tx = pool.begin().await.unwrap();
    assert!(
        ensure_resource_oa_tx(&mut tx, &spec)
            .await
            .unwrap()
            .is_some(),
        "事务入口应复用具名类的 OA"
    );
    grant_rights(
        &mut *tx,
        RT_NAMED_PC,
        9_900_003,
        ADMIN_UA_NAME,
        RIGHTS_CREATOR,
    )
    .await
    .expect("事务内授权");
    tx.rollback().await.unwrap();
    assert_eq!(
        assoc_rows(&pool, RT_NAMED_PC, 9_900_003).await,
        0,
        "事务回滚后 MUST NOT 残留 association"
    );

    // 不存在的具名类 ⇒ 不写 OA（保真原站点语义）
    let spec_missing = ResourceOaSpec {
        o_name: format!("{RT_MISSING_PC}-1"),
        resource_type: RT_MISSING_PC,
        resource_id: 9_900_004,
        resource_identifier: None,
        created_by: Some(user),
        policy_class_name: Some("z-test-no-such-policy-class"),
    };
    assert!(
        ensure_resource_oa(&pool, &spec_missing)
            .await
            .unwrap()
            .is_none(),
        "不存在的策略类 MUST NOT 写 OA"
    );
    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "需 aliothstudio_test；reset-db.sh --test 后运行"]
async fn grants_soft_deleted_row_not_resurrected() {
    let _guard = lock_serial().await;
    let pool = connect_test_db().await;
    cleanup(&pool).await;
    let user = admin_user(&pool).await;
    let rid = 9_900_005_i64;

    let spec = ResourceOaSpec {
        o_name: format!("{RT_PLAIN}-{rid}"),
        resource_type: RT_PLAIN,
        resource_id: rid,
        resource_identifier: None,
        created_by: Some(user),
        policy_class_name: None,
    };
    ensure_resource_oa(&pool, &spec).await.expect("anchor");
    grant_rights(&pool, RT_PLAIN, rid, ADMIN_UA_NAME, RIGHTS_CREATOR)
        .await
        .expect("grant");
    let before = assoc_rows(&pool, RT_PLAIN, rid).await;
    // 软删该资源上的全部 association（模拟已撤权）
    sqlx::query(
        "UPDATE isahl_auth.ngac_association a SET deleted_at = NOW() \
         FROM isahl_auth.ngac_object_attribute oa \
         WHERE oa.id = a.fk_object_attribute AND oa.resource_type = $1 AND oa.fk_resource = $2 \
           AND a.deleted_at IS NULL",
    )
    .bind(RT_PLAIN)
    .bind(rid)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(assoc_rows(&pool, RT_PLAIN, rid).await, 0);

    grant_rights(&pool, RT_PLAIN, rid, ADMIN_UA_NAME, RIGHTS_CREATOR)
        .await
        .expect("re-grant");
    assert_eq!(
        assoc_rows(&pool, RT_PLAIN, rid).await,
        0,
        "软删占位行 MUST NOT 被复活（与原点位 NOT EXISTS(deleted_at IS NULL) 同效）"
    );
    let _ = before;
    cleanup(&pool).await;
}
