//! 主体/岗位绑定自助申请集成测试（add-ngac-binding-request）
//!
//! 覆盖：
//! - entity 申请 → approve 落 auth_users 绑定；重复 pending 409；非 admin 403
//! - position 申请 → approve 建任职链 → 认知派生 UA 生效（缺口 1 联动）；reject 路径
//! - 表 `isahl_auth.ngac_binding_request` 由本文件幂等自建（测试基建模式；
//!   生产走运行时幂等 ensure（lib `ngac/ensure.rs`））
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

async fn ensure_table(pool: &PgPool) {
    // NGAC 运行时自愈（lib 唯一实现）
    gateway_sso::ngac::ensure::ensure_ngac_extension_tables(pool).await;
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
    admin_id: i64,
    admin_email: String,
    org_id: i64,
    position_id: i64,
}

async fn seed(pool: &PgPool, suffix: &str) -> Fixture {
    ensure_table(pool).await;
    // 用户 U + 管理员 A
    let email = format!("bind_user_{}@alioth.test", suffix);
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), $2, $1, 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .bind(&email)
    .bind(format!("bind_user_{}", suffix))
    .execute(pool)
    .await
    .expect("insert user");
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1")
            .bind(&email)
            .fetch_one(pool)
            .await
            .expect("fetch user");
    let admin_email = format!("bind_admin_{}@alioth.test", suffix);
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), $2, $1, 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .bind(&admin_email)
    .bind(format!("bind_admin_{}", suffix))
    .execute(pool)
    .await
    .expect("insert admin");
    let admin_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1")
            .bind(&admin_email)
            .fetch_one(pool)
            .await
            .expect("fetch admin");
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
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
         VALUES ('admin', $1, NOW(), NOW())
         ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING",
    )
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let admin_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='admin' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("admin UA");
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
         VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(admin_id)
    .bind(admin_ua)
    .execute(pool)
    .await
    .ok();

    // 组织主体（组织类白名单内）
    let org_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_orga-non-banking-legal\" (id, notice, code, created_by_id)
         VALUES (isahl.gen_next_zuid(), '绑定测试公司', $1, 1) RETURNING id",
    )
    .bind(format!("BIND-ORG-{}", suffix))
    .fetch_one(pool)
    .await
    .expect("insert org");

    // 岗位
    let position_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_subj-position\" (id, notice, code, created_by_id)
         VALUES (isahl.gen_next_zuid(), '绑定测试岗位', $1, 1) RETURNING id",
    )
    .bind(format!("BIND-POS-{}", suffix))
    .fetch_one(pool)
    .await
    .expect("insert position");

    Fixture {
        user_id,
        email,
        admin_id,
        admin_email,
        org_id,
        position_id,
    }
}

async fn cleanup(pool: &PgPool, f: &Fixture) {
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_binding_request WHERE fk_user = $1")
        .bind(f.user_id)
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "UPDATE isahl.\"zc_id_subj-post_rr_employee\" SET deleted_at = NOW() \
         WHERE ref_right IN (SELECT id FROM isahl.\"zc_id_empl-natural\" WHERE fk_user = $1)",
    )
    .bind(f.user_id)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_empl-natural\" WHERE fk_user = $1")
        .bind(f.user_id)
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "UPDATE isahl_auth.auth_users SET entity_id = NULL, entity_table = NULL WHERE id = $1",
    )
    .bind(f.user_id)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_subj-position\" WHERE id = $1")
        .bind(f.position_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_orga-non-banking-legal\" WHERE id = $1")
        .bind(f.org_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
        .bind(f.admin_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = ANY($1)")
        .bind(vec![f.user_id, f.admin_id])
        .execute(pool)
        .await;
}

#[tokio::test]
async fn entity_binding_request_full_loop() {
    let pool = connect().await;
    let f = seed(&pool, "e1").await;
    let ast = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/api/ngac").configure(gateway_sso::ngac::pdp::configure_routes))
            .service(web::scope("/api/admin").configure(gateway_sso::admin::configure)),
    )
    .await;
    let user_token = mint_token(&ast, f.user_id, &f.email);
    let admin_token = mint_token(&ast, f.admin_id, &f.admin_email);

    // 发起 entity 申请
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/binding-request")
            .insert_header(("Authorization", format!("Bearer {}", user_token)))
            .set_json(json!({"kind": "entity", "target_id": f.org_id, "reason": "入职绑定"}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 201, "发起应 201");
    let created: serde_json::Value = test::read_body_json(resp).await;
    let req_id: i64 = created["id"].as_str().unwrap().parse().expect("id");

    // 重复 pending → 409
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/binding-request")
            .insert_header(("Authorization", format!("Bearer {}", user_token)))
            .set_json(json!({"kind": "entity", "target_id": f.org_id}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 409, "重复 pending 应 409");

    // 非 admin 审批 → 403
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/api/admin/ngac/binding-requests/{}/approve",
                req_id
            ))
            .insert_header(("Authorization", format!("Bearer {}", user_token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 403, "非 admin 应 403");

    // admin approve → auth_users 绑定
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/api/admin/ngac/binding-requests/{}/approve",
                req_id
            ))
            .insert_header(("Authorization", format!("Bearer {}", admin_token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "approve 应 200: {:?}", resp);
    let bound: (Option<String>, Option<i64>) =
        sqlx::query_as("SELECT entity_table, entity_id FROM isahl_auth.auth_users WHERE id = $1")
            .bind(f.user_id)
            .fetch_one(&pool)
            .await
            .expect("read binding");
    assert_eq!(bound.1, Some(f.org_id), "entity_id 应落库");
    assert!(
        bound
            .0
            .as_deref()
            .unwrap_or("")
            .ends_with("zc_id_orga-non-banking-legal"),
        "entity_table 应为组织类: {:?}",
        bound.0
    );

    // 已审结 → 409
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/api/admin/ngac/binding-requests/{}/approve",
                req_id
            ))
            .insert_header(("Authorization", format!("Bearer {}", admin_token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 409, "已审结应 409");

    cleanup(&pool, &f).await;
}

#[tokio::test]
async fn position_binding_approve_links_cognition_derivation() {
    let pool = connect().await;
    let f = seed(&pool, "e2").await;
    let ast = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/api/ngac").configure(gateway_sso::ngac::pdp::configure_routes))
            .service(web::scope("/api/admin").configure(gateway_sso::admin::configure)),
    )
    .await;
    let user_token = mint_token(&ast, f.user_id, &f.email);
    let admin_token = mint_token(&ast, f.admin_id, &f.admin_email);

    // 发起 position 申请
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/binding-request")
            .insert_header(("Authorization", format!("Bearer {}", user_token)))
            .set_json(json!({"kind": "position", "target_id": f.position_id}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 201, "发起应 201");
    let created: serde_json::Value = test::read_body_json(resp).await;
    let req_id: i64 = created["id"].as_str().unwrap().parse().expect("id");

    // approve → 任职链 + 认知派生 UA
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/api/admin/ngac/binding-requests/{}/approve",
                req_id
            ))
            .insert_header(("Authorization", format!("Bearer {}", admin_token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "approve 应 200: {:?}", resp);

    let pos_name = format!("position:BIND-POS-{}", "e2");
    use gateway_sso::ngac::pip::PostgresPip;
    use gateway_sso::ngac::Pip;
    let pip = PostgresPip::new(pool.clone());
    let attrs = pip
        .get_all_user_attributes_with_inheritance(f.user_id)
        .await
        .expect("effective");
    let names: Vec<&str> = attrs.iter().map(|a| a.o_name.as_str()).collect();
    assert!(
        names.contains(&pos_name.as_str()),
        "绑定岗位后应派生 position UA: {:?}",
        names
    );

    // 我的申请列表含 approved
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/ngac/binding-request/me")
            .insert_header(("Authorization", format!("Bearer {}", user_token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let mine: serde_json::Value = test::read_body_json(resp).await;
    let row = mine
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"].as_str() == Some(&req_id.to_string()))
        .expect("我的申请含该条");
    assert_eq!(row["status"], "approved");

    cleanup(&pool, &f).await;
}

#[tokio::test]
async fn binding_request_reject_path() {
    let pool = connect().await;
    let f = seed(&pool, "e3").await;
    let ast = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/api/ngac").configure(gateway_sso::ngac::pdp::configure_routes))
            .service(web::scope("/api/admin").configure(gateway_sso::admin::configure)),
    )
    .await;
    let user_token = mint_token(&ast, f.user_id, &f.email);
    let admin_token = mint_token(&ast, f.admin_id, &f.admin_email);

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/binding-request")
            .insert_header(("Authorization", format!("Bearer {}", user_token)))
            .set_json(json!({"kind": "entity", "target_id": f.org_id}))
            .to_request(),
    )
    .await;
    let created: serde_json::Value = test::read_body_json(resp).await;
    let req_id: i64 = created["id"].as_str().unwrap().parse().expect("id");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/api/admin/ngac/binding-requests/{}/reject",
                req_id
            ))
            .insert_header(("Authorization", format!("Bearer {}", admin_token)))
            .set_json(json!({"reason": "主体不符"}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "reject 应 200");
    let row: (String, Option<String>) =
        sqlx::query_as("SELECT status, reason FROM isahl_auth.ngac_binding_request WHERE id = $1")
            .bind(req_id)
            .fetch_one(&pool)
            .await
            .expect("read request");
    assert_eq!(row.0, "rejected");
    assert_eq!(row.1.as_deref(), Some("主体不符"));
    // 绑定未被写
    let bound: (Option<String>, Option<i64>) =
        sqlx::query_as("SELECT entity_table, entity_id FROM isahl_auth.auth_users WHERE id = $1")
            .bind(f.user_id)
            .fetch_one(&pool)
            .await
            .expect("read binding");
    assert!(bound.1.is_none(), "reject 不应写绑定");

    cleanup(&pool, &f).await;
}
