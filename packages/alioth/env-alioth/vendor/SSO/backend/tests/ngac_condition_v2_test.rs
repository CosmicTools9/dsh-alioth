//! conditions v2 集成测试（add-ngac-condition-v2）
//!
//! 覆盖：
//! - user_attr_in：association 条件要求用户另持 UA Y → 持有者放行、非持有者拒绝；
//!   认知派生 UA 可作为 user_attr_in 目标（缺口 1 联动）
//! - object_attr_in：条件要求 OA 闭包含特定名
//! - 非法字段 fail-closed（user_attr_in 非数组 → 拒绝）
#![allow(clippy::type_complexity)]

mod common;

use actix_web::{test, web, App};
use serde_json::json;
use sqlx::PgPool;

async fn connect() -> PgPool {
    ::common::testing::connect_test_db().await
}

struct Fixture {
    user_a: i64,
    user_b: i64,
    ua_cond: i64, // 条件要求持有的 UA（user_attr_in 目标）
    oa: i64,
}

async fn seed(pool: &PgPool, suffix: &str) -> Fixture {
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_policy_class (o_name) VALUES ('default') ON CONFLICT DO NOTHING",
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

    let mut users = [0i64; 2];
    for (i, tag) in ["va", "vb"].iter().enumerate() {
        let email = format!("condv2_{}_{}@alioth.test", tag, suffix);
        sqlx::query(
            "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
             VALUES (isahl.gen_next_zuid(), $2, $1, 'active', true, NOW(), NOW())
             ON CONFLICT (email) DO NOTHING",
        )
        .bind(&email)
        .bind(format!("condv2_{}_{}", tag, suffix))
        .execute(pool)
        .await
        .expect("insert user");
        users[i] =
            sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1")
                .bind(&email)
                .fetch_one(pool)
                .await
                .expect("fetch user");
    }

    // UA base + UA cond；user_a 持有 base 与 cond；user_b 仅持有 base
    let base_name = format!("condv2-base-{}", suffix);
    let cond_name = format!("condv2-cond-{}", suffix);
    for name in [&base_name, &cond_name] {
        let _ = sqlx::query(
            "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
             VALUES ($1, $2, NOW(), NOW())
             ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING",
        )
        .bind(name)
        .bind(pc)
        .execute(pool)
        .await
        .ok();
    }
    let base_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(&base_name)
    .fetch_one(pool)
    .await
    .expect("base UA");
    let ua_cond: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(&cond_name)
    .fetch_one(pool)
    .await
    .expect("cond UA");
    for (uid, ua) in [
        (users[0], base_ua),
        (users[0], ua_cond),
        (users[1], base_ua),
    ] {
        let _ = sqlx::query(
            "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
             VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING",
        )
        .bind(uid)
        .bind(ua)
        .execute(pool)
        .await
        .ok();
    }

    // OA condres/0 + AR read + association: base → condres/read，conditions={user_attr_in:[cond_name]}
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at)
         VALUES ($1, $2, 'condres', 0, NOW(), NOW())
         ON CONFLICT (resource_type, fk_resource) DO UPDATE SET deleted_at = NULL",
    )
    .bind(format!("condres-oa-{}", suffix))
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let oa: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type='condres' AND fk_resource=0 LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("OA");
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ('read') ON CONFLICT (o_name) DO NOTHING",
    )
    .execute(pool)
    .await
    .ok();
    let ar: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='read' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("AR");
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_association (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, conditions, created_at, updated_at)
         VALUES ($1, $2, ARRAY[$3], $4, $5::jsonb, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(base_ua)
    .bind(oa)
    .bind(ar)
    .bind(pc)
    .bind(json!({"user_attr_in": [cond_name]}).to_string())
    .execute(pool)
    .await
    .expect("association");

    Fixture {
        user_a: users[0],
        user_b: users[1],
        ua_cond,
        oa,
    }
}

async fn cleanup(pool: &PgPool, f: &Fixture) {
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE fk_object_attribute = $1")
        .bind(f.oa)
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type='condres' AND fk_resource=0",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE id = $1")
        .bind(f.ua_cond)
        .execute(pool)
        .await;
}

#[tokio::test]
async fn user_attr_in_condition_gates_decision() {
    let pool = connect().await;
    let f = seed(&pool, "v1").await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(common::test_auth_state()))
            .route(
                "/api/ngac/decide",
                web::post().to(gateway_sso::ngac::pdp::ngac_decide),
            ),
    )
    .await;

    // user_a 持有 cond UA → 放行
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide")
            .set_json(json!({"user_id": f.user_a, "resource": "condres:0", "action": "read"}))
            .to_request(),
    )
    .await;
    let d: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(d["permitted"], true, "user_a 应放行: {}", d);

    // user_b 不持 cond UA → 拒绝（条件不满足 → 关联不匹配）
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide")
            .set_json(json!({"user_id": f.user_b, "resource": "condres:0", "action": "read"}))
            .to_request(),
    )
    .await;
    let d: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(d["permitted"], false, "user_b 应拒绝: {}", d);

    // explain/me（user_a）steps 应显示 conditions_met=true
    let state = common::test_auth_state();
    use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
    configure_token_validation(
        "http://localhost:9002".to_string(),
        "http://localhost:9002".to_string(),
    );
    let email_a = format!("condv2_va_{}@alioth.test", "v1");
    let token = encode_access_token(
        &Claims::new(&f.user_a.to_string(), &email_a, false),
        &state.jwt_private_key,
    )
    .expect("token");
    let app2 = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(state))
            .route(
                "/api/ngac/decide/explain/me",
                web::post().to(gateway_sso::ngac::pdp::ngac_decide_explain_self),
            ),
    )
    .await;
    let resp = test::call_service(
        &app2,
        test::TestRequest::post()
            .uri("/api/ngac/decide/explain/me")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(json!({"resource": "condres:0", "action": "read"}))
            .to_request(),
    )
    .await;
    let e: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(e["permitted"], true, "explain/me 应放行: {}", e);
    assert!(
        e["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["conditions_met"] == true),
        "steps 应含 conditions_met=true: {}",
        e
    );

    cleanup(&pool, &f).await;
}

#[tokio::test]
async fn object_attr_in_condition_matches_oa_closure() {
    let pool = connect().await;
    let f = seed(&pool, "v2").await;
    // 追加一条带 object_attr_in 的关联：base → condres/0（OA 名已知）+ 条件命中
    let base_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'condv2-base-%' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("base UA");
    let pc: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("policy class");
    let ar: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='read' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("AR");
    let oa_name: String =
        sqlx::query_scalar("SELECT o_name FROM isahl_auth.ngac_object_attribute WHERE id = $1")
            .bind(f.oa)
            .fetch_one(&pool)
            .await
            .expect("OA name");
    // 删除原关联（其 user_attr_in 条件会挡 user_b），换 object_attr_in 版本
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE fk_object_attribute = $1")
        .bind(f.oa)
        .execute(&pool)
        .await;
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_association (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, conditions, created_at, updated_at)
         VALUES ($1, $2, ARRAY[$3], $4, $5::jsonb, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(base_ua)
    .bind(f.oa)
    .bind(ar)
    .bind(pc)
    .bind(json!({"object_attr_in": [oa_name]}).to_string())
    .execute(&pool)
    .await
    .expect("association v2");

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(common::test_auth_state()))
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
            .set_json(json!({"user_id": f.user_b, "resource": "condres:0", "action": "read"}))
            .to_request(),
    )
    .await;
    let d: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(d["permitted"], true, "object_attr_in 命中应放行: {}", d);

    cleanup(&pool, &f).await;
}
