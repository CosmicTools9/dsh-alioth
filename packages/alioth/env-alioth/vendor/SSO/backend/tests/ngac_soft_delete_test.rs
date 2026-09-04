//! NGAC 策略边软删语义集成测试（fix-ngac-pdp-soft-delete-leak）
//!
//! 覆盖：`load_policy_from_db` 对 association / prohibition 的 `deleted_at IS NULL`
//! 过滤——软删的策略边 MUST 不再参与 PDP 决策（此前 SELECT 无过滤，软删边仍生效）。
//!
//! 验证路径：创建边 → decide 生效 → 软删边 → bump policy version 强制 reload →
//! decide 断言 NotApplicable（不再 permit / 不再 deny）。

mod common;

use ::common::testing::connect_test_db;
use actix_web::{test, web, App};
use gateway_sso::auth::AuthState;
use serde_json::json;
use sqlx::PgPool;

struct Seed {
    user: i64,
    reader_ua: i64,
    oa: i64,
    delete_ar: i64,
    assoc_id: i64,
}

/// 幂等 seed：普通用户（无 admin UA，避免 admin 兜底 Permit 干扰断言）+
/// reader UA + OA(engineers:0) + read/delete AR + association(reader→OA read)。
async fn seed(pool: &PgPool, email: &str) -> Seed {
    let username = email
        .split('@')
        .next()
        .unwrap_or("ngac_user")
        .replace('-', "_");
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_policy_class (o_name) VALUES ('default')
           ON CONFLICT DO NOTHING"#,
    )
    .execute(pool)
    .await
    .ok();
    let pc: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("policy class");

    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users (id, name, username, email, status, is_active, created_at, updated_at)
           VALUES (isahl.gen_next_zuid(), $2, $2, $1, 'active', true, NOW(), NOW())
           ON CONFLICT (email) DO NOTHING"#,
    )
    .bind(email)
    .bind(&username)
    .execute(pool)
    .await
    .ok();
    let user: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email=$1 LIMIT 1")
            .bind(email)
            .fetch_one(pool)
            .await
            .expect("user");

    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
           VALUES ('reader_ngac_sd', $1, NOW(), NOW())
           ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING"#,
    )
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let reader_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='reader_ngac_sd' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("reader UA");
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(user)
    .bind(reader_ua)
    .execute(pool)
    .await
    .ok();

    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at)
           VALUES ('sd-engineers-oa', $1, 'sd-engineers', 0, NOW(), NOW())
           ON CONFLICT (resource_type, fk_resource) DO UPDATE SET deleted_at = NULL"#,
    )
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let oa: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type='sd-engineers' AND fk_resource=0 LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("OA");

    let mut ars = [0i64; 2];
    for (i, name) in ["read", "delete"].iter().enumerate() {
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ($1) ON CONFLICT (o_name) DO NOTHING",
        )
        .bind(name)
        .execute(pool)
        .await
        .ok();
        ars[i] = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name=$1 LIMIT 1",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("AR");
    }

    let assoc_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.ngac_association (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at, updated_at)
           VALUES ($1, $2, ARRAY[$3], $4, NOW(), NOW())
           ON CONFLICT DO NOTHING
           RETURNING id"#,
    )
    .bind(reader_ua)
    .bind(oa)
    .bind(ars[0])
    .bind(pc)
    .fetch_one(pool)
    .await
    .expect("association");

    Seed {
        user,
        reader_ua,
        oa,
        delete_ar: ars[1],
        assoc_id,
    }
}

fn test_auth_state() -> AuthState {
    common::test_auth_state()
}

/// 策略版本 +1，强制下一次 decide 走 `ensure_policy_loaded` 全量 reload。
async fn bump_policy_version(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_policy_version (version) VALUES (1) ON CONFLICT DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("policy version seed");
    sqlx::query(
        "UPDATE isahl_auth.ngac_policy_version SET version = version + 1, updated_at = NOW()",
    )
    .execute(pool)
    .await
    .expect("bump policy version");
}

async fn decide(pool: &PgPool, user: i64, action: &str) -> serde_json::Value {
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(test_auth_state()))
            .route(
                "/api/ngac/decide",
                web::post().to(gateway_sso::ngac::pdp::ngac_decide),
            ),
    )
    .await;
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide")
            .set_json(json!({
                "user_id": user,
                "resource": "sd-engineers:0",
                "action": action
            }))
            .to_request(),
    )
    .await;
    test::read_body_json(resp).await
}

async fn cleanup(pool: &PgPool, email: &str, oa_id: i64) {
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user IN (SELECT id FROM isahl_auth.auth_users WHERE email=$1)",
    )
    .bind(email)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE email=$1")
        .bind(email)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.ngac_prohibition WHERE fk_object_attribute = $1")
        .bind(oa_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE fk_object_attribute = $1")
        .bind(oa_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE id = $1")
        .bind(oa_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name = 'reader_ngac_sd'")
        .execute(pool)
        .await
        .ok();
}

/// 软删 association 后（bump 版本强制 reload），该边 MUST 不再参与决策：
/// read 从 permit 变为 not-applicable（permitted=false）。
#[tokio::test]
async fn soft_deleted_association_not_in_policy_graph() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let email = "ngac-sd-assoc@test.local";
    let s = seed(&pool, email).await;

    // 1) 软删前：association 生效 → permit
    let d1 = decide(&pool, s.user, "read").await;
    assert_eq!(d1["permitted"], true, "association active: {:?}", d1);

    // 2) 软删 association → bump 版本强制 reload
    sqlx::query("UPDATE isahl_auth.ngac_association SET deleted_at = NOW() WHERE id = $1")
        .bind(s.assoc_id)
        .execute(&pool)
        .await
        .expect("soft delete association");
    bump_policy_version(&pool).await;

    // 3) 软删后：决策不再放行（fail-closed，不残留幽灵授权）
    let d2 = decide(&pool, s.user, "read").await;
    assert_eq!(
        d2["permitted"], false,
        "soft-deleted association must not grant: {:?}",
        d2
    );
    assert_eq!(
        d2["reason"].as_str().unwrap_or(""),
        "No matching access right found",
        "soft-deleted association must be not-applicable: {:?}",
        d2
    );

    cleanup(&pool, email, s.oa).await;
}

/// 软删 prohibition 后（bump 版本强制 reload），该边 MUST 不再拒绝：
/// delete 从 deny 变为 not-applicable。
#[tokio::test]
async fn soft_deleted_prohibition_not_in_policy_graph() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let email = "ngac-sd-proh@test.local";
    let s = seed(&pool, email).await;

    // 1) 建 prohibition：reader UA → OA 禁止 delete
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_prohibition (fk_user_attribute, fk_object_attribute, ak_access_rights, created_at, updated_at)
           VALUES ($1, $2, ARRAY[$3], NOW(), NOW())"#,
    )
    .bind(s.reader_ua)
    .bind(s.oa)
    .bind(s.delete_ar)
    .execute(&pool)
    .await
    .expect("create prohibition");
    bump_policy_version(&pool).await;

    // 2) 软删前：prohibition 生效 → deny（reason 标识显式拒绝）
    let d1 = decide(&pool, s.user, "delete").await;
    assert_eq!(d1["permitted"], false, "prohibition active: {:?}", d1);
    assert!(
        d1["reason"]
            .as_str()
            .unwrap_or("")
            .contains("denied by prohibition"),
        "deny must come from prohibition, got: {:?}",
        d1
    );

    // 3) 软删 prohibition → bump 版本强制 reload
    sqlx::query("UPDATE isahl_auth.ngac_prohibition SET deleted_at = NOW()")
        .execute(&pool)
        .await
        .expect("soft delete prohibition");
    bump_policy_version(&pool).await;

    // 4) 软删后：不再拒绝 → not-applicable（reason 不再含 prohibition 拒绝）
    let d2 = decide(&pool, s.user, "delete").await;
    assert_eq!(
        d2["permitted"], false,
        "soft-deleted prohibition must not grant: {:?}",
        d2
    );
    assert!(
        !d2["reason"].as_str().unwrap_or("").contains("denied"),
        "soft-deleted prohibition must not deny (not-applicable), got: {:?}",
        d2
    );

    cleanup(&pool, email, s.oa).await;
}
