//! 授权撤销 + 到期（change `add-ngac-grant-revocation-and-expiry`）回归。
//!
//! 守护：① `revoke_rights` 差异收缩（剔除 right 后该动作拒、其余动作仍放行）；
//! ② 撤销至空集 ⇒ association 软删（不再有活跃行）；③ association `expires_at` 到期后
//! 决策即时翻转（无任何写入也要重载快照）。
//!
//! 口径：`TEST_DATABASE_URL="postgres://<user>@localhost:5432/aliothstudio_test"` + `--ignored`。

use common::ngac_grants::{
    ensure_resource_oa, grant_rights, grant_rights_until, revoke_rights, ResourceOaSpec,
};
use common::permissions::require_resource_access;
use sqlx::PgPool;

const RT: &str = "zz-rev";
const UA: &str = "zz-rev-ua";

async fn open_test_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    if url.is_none() {
        eprintln!("skipped: DATABASE_URL 未设置");
        return None;
    }
    Some(common::testing::connect_test_db().await)
}

/// 测试库自持 `expires_at` 列（生产由 `SSO ngac ensure` 幂等自愈；集成测试不依赖服务启动）。
async fn ensure_expiry_column(pool: &PgPool) {
    sqlx::query(
        "ALTER TABLE isahl_auth.ngac_association ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ",
    )
    .execute(pool)
    .await
    .expect("ensure expires_at column");
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
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE o_name = $1")
        .bind(UA)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name = $1")
        .bind(UA)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username LIKE 'zz-rev%'")
        .execute(pool)
        .await;
}

/// 夹具：用户 + 具名 UA + 行 OA；返回 (user_id, row_id)。
async fn setup(pool: &PgPool) -> (i64, i64) {
    common::permissions::ensure_ngac_predicate_fns(pool)
        .await
        .expect("ensure ngac predicate fns");
    ensure_expiry_column(pool).await;
    let user: i64 =
        sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id")
            .bind("zz-rev-user")
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
    .bind(UA)
    .bind(pc)
    .fetch_one(pool)
    .await
    .expect("insert UA");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, o_name) \
         VALUES ($1, $2, $3)",
    )
    .bind(user)
    .bind(ua_id)
    .bind(UA)
    .execute(pool)
    .await
    .expect("assign UA");

    let row_id: i64 = 9_900_991;
    ensure_resource_oa(
        pool,
        &ResourceOaSpec {
            o_name: format!("{RT}-{row_id}"),
            resource_type: RT,
            resource_id: row_id,
            resource_identifier: Some("ZZ-REV"),
            created_by: Some(user),
            policy_class_name: Some("default"),
        },
    )
    .await
    .expect("ensure row OA");
    grant_rights(pool, RT, row_id, UA, &["read", "write"])
        .await
        .expect("grant read+write");
    (user, row_id)
}

async fn active_assoc_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.ngac_association a \
           JOIN isahl_auth.ngac_user_attribute ua ON ua.id = a.fk_user_attribute \
           JOIN isahl_auth.ngac_object_attribute oa ON oa.id = a.fk_object_attribute \
          WHERE ua.o_name = $1 AND oa.resource_type = $2 AND a.deleted_at IS NULL",
    )
    .bind(UA)
    .bind(RT)
    .fetch_one(pool)
    .await
    .expect("count assoc")
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn revoke_shrinks_rights_only() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;
    let (user, row_id) = setup(&pool).await;
    assert!(require_resource_access(&pool, user, RT, row_id, "read")
        .await
        .is_ok());
    assert!(require_resource_access(&pool, user, RT, row_id, "write")
        .await
        .is_ok());

    let n = revoke_rights(&pool, RT, row_id, UA, &["read"])
        .await
        .expect("revoke read");
    assert!(n >= 1, "撤销 MUST 命中该 association（got {n}）");
    assert!(
        require_resource_access(&pool, user, RT, row_id, "read")
            .await
            .is_err(),
        "撤销后 read MUST 拒绝"
    );
    assert!(
        require_resource_access(&pool, user, RT, row_id, "write")
            .await
            .is_ok(),
        "撤销 read MUST NOT 影响 write（差异收缩）"
    );
    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn revoke_all_soft_deletes_association() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;
    let (user, row_id) = setup(&pool).await;
    revoke_rights(&pool, RT, row_id, UA, &["read", "write"])
        .await
        .expect("revoke all");
    assert_eq!(
        active_assoc_count(&pool).await,
        0,
        "撤销至空集 MUST 软删 association"
    );
    assert!(require_resource_access(&pool, user, RT, row_id, "read")
        .await
        .is_err());
    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn association_expires_at_denies_after_expiry() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;
    common::permissions::ensure_ngac_predicate_fns(&pool)
        .await
        .expect("ensure ngac predicate fns");
    ensure_expiry_column(&pool).await;
    let user: i64 =
        sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id")
            .bind("zz-rev-exp")
            .fetch_one(&pool)
            .await
            .expect("insert user");
    let pc: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default'")
            .fetch_one(&pool)
            .await
            .expect("default policy class");
    let ua_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class) \
         VALUES ($1, $2) RETURNING id",
    )
    .bind(UA)
    .bind(pc)
    .fetch_one(&pool)
    .await
    .expect("insert UA");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, o_name) \
         VALUES ($1, $2, $3)",
    )
    .bind(user)
    .bind(ua_id)
    .bind(UA)
    .execute(&pool)
    .await
    .expect("assign UA");
    let row_id: i64 = 9_900_992;
    ensure_resource_oa(
        &pool,
        &ResourceOaSpec {
            o_name: format!("{RT}-{row_id}"),
            resource_type: RT,
            resource_id: row_id,
            resource_identifier: Some("ZZ-REV-EXP"),
            created_by: Some(user),
            policy_class_name: Some("default"),
        },
    )
    .await
    .expect("ensure row OA");
    // 到期 = +2s
    grant_rights_until(
        &pool,
        RT,
        row_id,
        UA,
        &["read"],
        Some(chrono::Utc::now() + chrono::Duration::seconds(2)),
    )
    .await
    .expect("grant with expiry");

    assert!(
        require_resource_access(&pool, user, RT, row_id, "read")
            .await
            .is_ok(),
        "未到期 MUST 放行"
    );
    tokio::time::sleep(std::time::Duration::from_millis(2600)).await;
    assert!(
        require_resource_access(&pool, user, RT, row_id, "read")
            .await
            .is_err(),
        "到期后 MUST 即时拒绝（无写入也重载快照）"
    );
    cleanup(&pool).await;
}
