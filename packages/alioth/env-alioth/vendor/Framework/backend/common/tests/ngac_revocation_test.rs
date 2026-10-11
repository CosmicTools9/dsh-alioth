//! 窗口失效类派生物撤销测试（revoke-expired-org-policy-derivations）。
//!
//! 判据：active 类派生（UA × 集合 OA × read association）→ 窗口到期（`effective_until`
//! 过去化）→ `revoke_expired_class_derivations` 软删该类派生 association + 每类一条
//! 审计行；重跑幂等（零新删零新审计）；窗口恢复 + 重派生 ⇒ association 幂等重建。
//!
//! 运行：`bash scripts/cargo-test.sh -p common --test ngac_revocation_test -- --ignored --test-threads=1`
//!（需 `aliothstudio_test`——org_policy_* 表由 SSO ensure 链供给）。

use common::ngac_policy::{derive_from_class, revoke_expired_class_derivations};
use common::testing::connect_test_db;
use sqlx::PgPool;
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};

static SERIAL_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn lock_serial() -> MutexGuard<'static, ()> {
    SERIAL_LOCK.lock().await
}

const CLASS_CODE: &str = "z-test-rev-class";
const RT: &str = "z-test-rev-rt";

async fn live_assoc_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl_auth.ngac_association a
           JOIN isahl_auth.ngac_user_attribute ua ON ua.id = a.fk_user_attribute
           JOIN isahl_auth.ngac_object_attribute oa ON oa.id = a.fk_object_attribute
           WHERE ua.property ->> 'source_kind' = 'org_policy_class'
             AND ua.property ->> 'source_code' = $1
             AND oa.resource_type = $2 AND oa.fk_resource = 0
             AND a.deleted_at IS NULL"#,
    )
    .bind(CLASS_CODE)
    .bind(RT)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn audit_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.ngac_policy_audit_log \
         WHERE action = 'revoke_expired' AND new_values ->> 'code' = $1",
    )
    .bind(CLASS_CODE)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn cleanup(pool: &PgPool) {
    // 软删行与派生物全清（测试自持类；审计行同清）
    sqlx::query(
        r#"DELETE FROM isahl_auth.ngac_association a
           USING isahl_auth.ngac_user_attribute ua
           WHERE ua.id = a.fk_user_attribute
             AND ua.property ->> 'source_code' = $1"#,
    )
    .bind(CLASS_CODE)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE property ->> 'source_code' = $1")
        .bind(CLASS_CODE)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type = $1")
        .bind(RT)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM isahl_auth.org_policy_rule WHERE subject_code = $1")
        .bind(CLASS_CODE)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM isahl_auth.org_policy_class WHERE code = $1")
        .bind(CLASS_CODE)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM isahl_auth.ngac_policy_audit_log WHERE new_values ->> 'code' = $1")
        .bind(CLASS_CODE)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs test DB"]
async fn expired_window_revokes_derived_associations_idempotently() {
    let _g = lock_serial().await;
    let pool = connect_test_db().await;
    cleanup(&pool).await;

    // ── 夹具：active 窗口内类 + read rule → 派生 ──
    let class_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.org_policy_class
             (code, notice, state, effective_from, effective_until)
           VALUES ($1, '撤销测试类', 'active', NOW() - INTERVAL '1 hour',
                   NOW() + INTERVAL '2 hours')
           RETURNING id"#,
    )
    .bind(CLASS_CODE)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl_auth.org_policy_rule
             (policy_class_id, subject_code, resource_type, actions, state)
           VALUES ($1, $2, $3, '["read"]'::jsonb, 'active')"#,
    )
    .bind(class_id)
    .bind(CLASS_CODE)
    .bind(RT)
    .execute(&pool)
    .await
    .unwrap();

    derive_from_class(&pool, class_id)
        .await
        .expect("窗口内派生");
    assert!(live_assoc_count(&pool).await >= 1, "派生 association 落库");

    // ── 窗口到期 → 撤销 ──
    sqlx::query("UPDATE isahl_auth.org_policy_class SET effective_until = NOW() - INTERVAL '1 hour' WHERE id = $1")
        .bind(class_id)
        .execute(&pool)
        .await
        .unwrap();

    let stats = revoke_expired_class_derivations(&pool)
        .await
        .expect("撤销扫帚");
    assert!(stats.associations_revoked >= 1, "本类至少撤一行：{stats:?}");
    assert_eq!(
        live_assoc_count(&pool).await,
        0,
        "窗口外类的派生 association 全部软删"
    );
    assert_eq!(audit_count(&pool).await, 1, "每类一条撤销审计行");

    // ── 幂等：重跑零新删零新审计 ──
    let stats2 = revoke_expired_class_derivations(&pool)
        .await
        .expect("撤销扫帚重跑");
    //（测试库可能存在他类失效行——只对本类断言）
    assert_eq!(live_assoc_count(&pool).await, 0);
    assert_eq!(audit_count(&pool).await, 1, "零撤销量不写新审计");
    let _ = stats2;

    // ── 窗口恢复 + 重派生 ⇒ association 幂等重建（撤销不破坏再入）──
    sqlx::query("UPDATE isahl_auth.org_policy_class SET effective_until = NOW() + INTERVAL '2 hours' WHERE id = $1")
        .bind(class_id)
        .execute(&pool)
        .await
        .unwrap();
    derive_from_class(&pool, class_id)
        .await
        .expect("重派生（恢复窗口）");
    assert!(
        live_assoc_count(&pool).await >= 1,
        "软删 association 后重派生幂等重建"
    );

    cleanup(&pool).await;
}
