//! /auth/me 主体认知集成测试（refactor-subject-perspective-chain）
//!
//! 验证 `me` handler 新增的 `subject` / `perspectives` 字段：
//! - 已绑定用户 → subject 含 id/code/name/entity_table（zuid 字符串化）
//! - 岗位带视角标签用户 → perspectives 按岗位聚合 view_tags
//! - 未绑定用户 → subject=null、perspectives=[]，响应仍 200
#![allow(clippy::type_complexity)]

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
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
    subject_id: i64,
    employee_id: i64,
    position_id: i64,
    tag_id: i64,
}

/// 建「已绑定主体 + 任职岗位 + 岗位视角标签」完整链路的用户。
async fn seed_bound_user(pool: &PgPool, suffix: &str) -> Fixture {
    let email = format!("me_cognition_{}@alioth.test", suffix);
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), 'me_cognition', $1, 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .bind(&email)
    .execute(pool)
    .await
    .expect("insert user");
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1")
            .bind(&email)
            .fetch_one(pool)
            .await
            .expect("fetch user");

    // 主体（企业法人叶表——生产规则：主体一律落叶表）
    let subject_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_orga-non-banking-legal\" (id, notice, code, created_by_id)
         VALUES (isahl.gen_next_zuid(), '认知测试公司', $1, 1) RETURNING id",
    )
    .bind(format!("ME-COG-ORG-{}", suffix))
    .fetch_one(pool)
    .await
    .expect("insert subject");

    // 绑定（entity_table/entity_id）
    sqlx::query(
        "UPDATE isahl_auth.auth_users SET entity_table = 'zc_id_orga-non-banking-legal', entity_id = $1
         WHERE id = $2",
    )
    .bind(subject_id)
    .bind(user_id)
    .execute(pool)
    .await
    .expect("bind subject");

    // 任职链：empl-natural(fk_user) → post_rr_employee → 岗位
    let employee_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_empl-natural\" (id, notice, code, fk_user, created_by_id)
         VALUES (isahl.gen_next_zuid(), '认知测试雇员', $1, $2, 1) RETURNING id",
    )
    .bind(format!("ME-COG-EMP-{}", suffix))
    .bind(user_id)
    .fetch_one(pool)
    .await
    .expect("insert employee");
    let position_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_subj-position\" (id, notice, code, created_by_id)
         VALUES (isahl.gen_next_zuid(), '认知测试岗位', $1, 1) RETURNING id",
    )
    .bind(format!("ME-COG-POS-{}", suffix))
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

    // 视角链：岗位 → relation-post_view_r_tags → tags-post_view
    let tag_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_tags-post_view\" (id, notice, code, created_by_id)
         VALUES (isahl.gen_next_uid(130), '认知测试视角', $1, 1) RETURNING id",
    )
    .bind(format!("VIEW-ME-COG-{}", suffix))
    .fetch_one(pool)
    .await
    .expect("insert view tag");
    sqlx::query(
        "INSERT INTO isahl.\"zc_id_relation-post_view_r_tags\" (id, notice, ref_left, ref_right, created_by_id)
         VALUES (isahl.gen_next_uid(180), '岗位视角', $1, $2, 1)",
    )
    .bind(position_id)
    .bind(tag_id)
    .execute(pool)
    .await
    .expect("link position tag");

    Fixture {
        user_id,
        email,
        subject_id,
        employee_id,
        position_id,
        tag_id,
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
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_tags-post_view\" WHERE id = $1")
        .bind(f.tag_id)
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
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_orga-non-banking-legal\" WHERE id = $1")
        .bind(f.subject_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(f.user_id)
        .execute(pool)
        .await;
}

async fn call_me(pool: &PgPool, user_id: i64, email: &str) -> serde_json::Value {
    let ast = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::auth::login::configure),
    )
    .await;
    let token = mint_token(&ast, user_id, email);
    let req = test::TestRequest::get()
        .uri("/me")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200, "me 应 200: {:?}", resp);
    test::read_body_json(resp).await
}

#[tokio::test]
async fn me_returns_subject_and_perspectives_for_bound_user() {
    let pool = connect().await;
    let f = seed_bound_user(&pool, "s1").await;

    let body = call_me(&pool, f.user_id, &f.email).await;

    let subject = &body["user"]["subject"];
    assert!(
        subject.is_object(),
        "已绑定用户 subject 应为对象: {}",
        body["user"]
    );
    assert_eq!(subject["id"].as_str().unwrap(), f.subject_id.to_string());
    assert_eq!(subject["name"].as_str().unwrap(), "认知测试公司");
    assert_eq!(
        subject["entity_table"].as_str().unwrap(),
        "zc_id_orga-non-banking-legal"
    );

    let perspectives = body["user"]["perspectives"]
        .as_array()
        .expect("perspectives 数组");
    let entry = perspectives
        .iter()
        .find(|p| p["position_id"].as_str() == Some(&f.position_id.to_string()))
        .expect("应含任职岗位条目");
    let codes: Vec<&str> = entry["view_tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["code"].as_str())
        .collect();
    assert!(
        codes.contains(&"VIEW-ME-COG-s1"),
        "视角标签应含岗位标签: {:?}",
        codes
    );

    cleanup(&pool, &f).await;
}

#[tokio::test]
async fn me_returns_null_subject_and_empty_perspectives_for_unbound_user() {
    let pool = connect().await;
    let email = "me_cognition_unbound@alioth.test";
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), 'me_unbound', $1, 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .bind(email)
    .execute(&pool)
    .await
    .expect("insert user");
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1")
            .bind(email)
            .fetch_one(&pool)
            .await
            .expect("fetch user");

    let body = call_me(&pool, user_id, email).await;
    assert!(
        body["user"]["subject"].is_null(),
        "未绑定用户 subject 应为 null: {}",
        body["user"]
    );
    assert_eq!(
        body["user"]["perspectives"].as_array().unwrap().len(),
        0,
        "无任职用户 perspectives 应为空数组"
    );

    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await;
}
