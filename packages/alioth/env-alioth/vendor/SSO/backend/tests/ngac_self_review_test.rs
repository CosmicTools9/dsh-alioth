//! 本人作用域访问审查与决策解释测试（add-ngac-self-access-review）
//!
//! 覆盖：
//! - GET /api/ngac/review/me：本人 assignments/derived_ua/permissions；无 token → 401
//! - POST /api/ngac/decide/explain/me：outcome 与 /api/ngac/decide 一致；
//!   任职软删后解释随派生撤销；无 token → 401
#![allow(clippy::type_complexity)]

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use serde_json::json;
use sqlx::PgPool;

async fn connect() -> PgPool {
    ::common::testing::connect_test_db().await
}

fn mint_token(ast: &AuthState, user_id: i64, email: &str) -> String {
    configure_token_validation(
        "http://localhost:9002".to_string(),
        "http://localhost:9002".to_string(),
    );
    encode_access_token(
        &Claims::new(&user_id.to_string(), email, false),
        &ast.jwt_private_key,
    )
    .expect("mint token")
}

struct Fixture {
    user_id: i64,
    email: String,
    employee_id: i64,
    position_id: i64,
    tag_code: String,
}

/// 认知用户 + 派生 UA + 关联（view: UA → cogme/read）全链路
async fn seed_cognition_user_with_grant(pool: &PgPool, suffix: &str) -> Fixture {
    let email = format!("self_review_{}@alioth.test", suffix);
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), $2, $1, 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .bind(&email)
    .bind(format!("self_review_{}", suffix))
    .execute(pool)
    .await
    .expect("insert user");
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1")
            .bind(&email)
            .fetch_one(pool)
            .await
            .expect("fetch user");

    let employee_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_empl-natural\" (id, notice, code, fk_user, created_by_id)
         VALUES (isahl.gen_next_zuid(), '自审雇员', $1, $2, 1) RETURNING id",
    )
    .bind(format!("SR-EMP-{}", suffix))
    .bind(user_id)
    .fetch_one(pool)
    .await
    .expect("insert employee");
    let position_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_subj-position\" (id, notice, code, created_by_id)
         VALUES (isahl.gen_next_zuid(), '自审岗位', $1, 1) RETURNING id",
    )
    .bind(format!("SR-POS-{}", suffix))
    .fetch_one(pool)
    .await
    .expect("insert position");
    sqlx::query(
        "INSERT INTO isahl.\"zc_id_subj-post_rr_employee\" (id, notice, ref_left, ref_right, created_by_id)
         VALUES (isahl.gen_next_uid(312), '任职', $1, $2, 1)",
    )
    .bind(position_id)
    .bind(employee_id)
    .execute(pool)
    .await
    .expect("link employment");

    let tag_code = format!("VIEW-SR-{}", suffix);
    let tag_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_tags-post_view\" (id, notice, code, created_by_id)
         VALUES (isahl.gen_next_uid(130), '自审视角', $1, 1) RETURNING id",
    )
    .bind(&tag_code)
    .fetch_one(pool)
    .await
    .expect("insert view tag");
    let _ = sqlx::query(
        "INSERT INTO isahl.\"zc_id_relation-post_view_r_tags\" (id, notice, ref_left, ref_right, created_by_id)
         VALUES (isahl.gen_next_uid(180), '岗位视角', $1, $2, 1)",
    )
    .bind(position_id)
    .bind(tag_id)
    .execute(pool)
    .await
    .expect("link position tag");

    // 物化派生 UA 并建关联（view:UA → cogme/read）
    use gateway_sso::ngac::pip::PostgresPip;
    use gateway_sso::ngac::Pip;
    let pip = PostgresPip::new(pool.clone());
    pip.get_all_user_attributes_with_inheritance(user_id)
        .await
        .expect("ensure + effective set");
    let view_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(format!("view:{}", tag_code))
    .fetch_one(pool)
    .await
    .expect("view UA");
    let pc: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("policy class");
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at)
         VALUES ($1, $2, 'cogme', 0, NOW(), NOW())
         ON CONFLICT (resource_type, fk_resource) DO UPDATE SET deleted_at = NULL",
    )
    .bind(format!("cogme-oa-sr-{}", suffix))
    .bind(pc)
    .execute(pool)
    .await
    .expect("OA");
    let oa: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type='cogme' AND fk_resource=0 LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("OA id");
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ('read') ON CONFLICT (o_name) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("AR");
    let ar: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='read' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("AR id");
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_association (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at, updated_at)
         VALUES ($1, $2, ARRAY[$3], $4, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(view_ua)
    .bind(oa)
    .bind(ar)
    .bind(pc)
    .execute(pool)
    .await
    .expect("association");

    Fixture {
        user_id,
        email,
        employee_id,
        position_id,
        tag_code,
    }
}

async fn cleanup(pool: &PgPool, f: &Fixture) {
    let _ = sqlx::query(
        "UPDATE isahl.\"zc_id_subj-post_rr_employee\" SET deleted_at = NOW() WHERE ref_right = $1",
    )
    .bind(f.employee_id)
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "UPDATE isahl.\"zc_id_relation-post_view_r_tags\" SET deleted_at = NOW() WHERE ref_left = $1",
    )
    .bind(f.position_id)
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.ngac_association WHERE fk_user_attribute IN
         (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1)",
    )
    .bind(format!("view:{}", f.tag_code))
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name = $1")
        .bind(format!("view:{}", f.tag_code))
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type='cogme' AND fk_resource=0",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_subj-position\" WHERE id = $1")
        .bind(f.position_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_empl-natural\" WHERE id = $1")
        .bind(f.employee_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(f.user_id)
        .execute(pool)
        .await;
}

#[tokio::test]
async fn self_review_returns_derived_ua_and_permissions() {
    let pool = connect().await;
    let f = seed_cognition_user_with_grant(&pool, "a1").await;
    let ast = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/api/ngac").configure(gateway_sso::ngac::pdp::configure_routes)),
    )
    .await;
    let token = mint_token(&ast, f.user_id, &f.email);

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/ngac/review/me")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "review/me 应 200");
    let body: serde_json::Value = test::read_body_json(resp).await;
    let derived: Vec<&str> = body["derived_ua"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
        .unwrap_or_default();
    assert!(
        derived.contains(&format!("view:{}", f.tag_code).as_str()),
        "derived_ua 应含视角派生 UA: {:?}",
        derived
    );
    let perms = &body["permissions"];
    let cogme = perms
        .as_array()
        .and_then(|a| a.iter().find(|p| p["resource_type"] == "cogme"))
        .expect("cogme 行");
    let allowed: Vec<&str> = cogme["allowed"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
        .unwrap_or_default();
    assert!(
        allowed.contains(&"read"),
        "cogme allowed 应含 read: {:?}",
        allowed
    );
    // 派生 UA 不应出现在直接指派 assignments
    let assignments = body["assignments"].as_array().unwrap();
    for a in assignments {
        assert_ne!(
            a["o_name"].as_str(),
            Some(format!("view:{}", f.tag_code).as_str()),
            "派生 UA 不应出现在 assignments"
        );
    }

    cleanup(&pool, &f).await;
}

#[tokio::test]
async fn self_review_requires_token() {
    let pool = connect().await;
    let f = seed_cognition_user_with_grant(&pool, "a2").await;
    let ast = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast))
            .service(web::scope("/api/ngac").configure(gateway_sso::ngac::pdp::configure_routes)),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/ngac/review/me")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 401, "无 token review/me 应 401");

    cleanup(&pool, &f).await;
}

#[tokio::test]
async fn self_explain_matches_decide_and_follows_derivation() {
    let pool = connect().await;
    let f = seed_cognition_user_with_grant(&pool, "a3").await;
    let ast = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/api/ngac").configure(gateway_sso::ngac::pdp::configure_routes)),
    )
    .await;
    let token = mint_token(&ast, f.user_id, &f.email);
    let auth = ("Authorization", format!("Bearer {}", token));

    // explain/me 与 decide 一致（均有派生 read）
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide/explain/me")
            .insert_header(auth.clone())
            .set_json(json!({"resource": "cogme:0", "action": "read"}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["permitted"], true, "派生 read 应放行: {}", body);
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide")
            .set_json(json!({"user_id": f.user_id, "resource": "cogme:0", "action": "read"}))
            .to_request(),
    )
    .await;
    let d: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(d["permitted"], true, "decide 应一致放行");

    // 任职软删 → explain 随派生撤销
    let _ = sqlx::query(
        "UPDATE isahl.\"zc_id_subj-post_rr_employee\" SET deleted_at = NOW() WHERE ref_right = $1",
    )
    .bind(f.employee_id)
    .execute(&pool)
    .await
    .expect("soft-delete employment");
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide/explain/me")
            .insert_header(auth.clone())
            .set_json(json!({"resource": "cogme:0", "action": "read"}))
            .to_request(),
    )
    .await;
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["permitted"], false, "任职终止后应拒绝: {}", body);

    cleanup(&pool, &f).await;
}

#[tokio::test]
async fn self_explain_requires_token() {
    let pool = connect().await;
    let f = seed_cognition_user_with_grant(&pool, "a4").await;
    let ast = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast))
            .service(web::scope("/api/ngac").configure(gateway_sso::ngac::pdp::configure_routes)),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide/explain/me")
            .set_json(json!({"resource": "cogme:0", "action": "read"}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 401, "无 token explain/me 应 401");

    cleanup(&pool, &f).await;
}
