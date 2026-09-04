//! 认知链推导 UA 集成测试（add-ngac-cognition-derived-ua）
//!
//! 覆盖：
//! - PIP 有效 UA 集并入 position:/view: 派生 UA（含祖先闭包查询）
//! - ensure 幂等（二次调用零新行）
//! - decide 决策：association 绑 view: UA → 持有者 Permit；任职软删 → 撤销
//! - get_accessible_resource_ids（RLS 列表）含派生可见
//! - /auth/me permissions 矩阵含派生授权
//! - 指派 expires_at 写路径（add-ngac-assignment-expires-at）
#![allow(clippy::type_complexity)]

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use ::common::ngac_org::ensure_cognition_uas;
use gateway_sso::auth::AuthState;
use gateway_sso::ngac::pip::PostgresPip;
use gateway_sso::ngac::Pip;
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
    position_code: String,
    tag_id: i64,
    tag_code: String,
}

/// 建「任职岗位 + 岗位视角标签」链路的用户（同 me_subject_perspective 同构；
/// 主体绑定非本 change 测试面，不建 subject）。
async fn seed_cognition_user(pool: &PgPool, suffix: &str) -> Fixture {
    let email = format!("cog_derived_{}@alioth.test", suffix);
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), $2, $1, 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .bind(&email)
    .bind(format!("cog_derived_{}", suffix))
    .execute(pool)
    .await
    .expect("insert user");
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1")
            .bind(&email)
            .fetch_one(pool)
            .await
            .expect("fetch user");

    // 前置清理：重跑残留（旧任职/标签链，code 不唯一导致累积）
    let _ = sqlx::query(
        "UPDATE isahl.\"zc_id_subj-post_rr_employee\" SET deleted_at = NOW() \
         WHERE ref_right IN (SELECT id FROM isahl.\"zc_id_empl-natural\" WHERE fk_user = $1)",
    )
    .bind(user_id)
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "UPDATE isahl.\"zc_id_relation-post_view_r_tags\" SET deleted_at = NOW() \
         WHERE ref_left IN (SELECT id FROM isahl.\"zc_id_subj-position\" p \
            JOIN isahl.\"zc_id_subj-post_rr_employee\" spre ON spre.ref_left = p.id \
            WHERE spre.ref_right IN (SELECT id FROM isahl.\"zc_id_empl-natural\" WHERE fk_user = $1))",
    )
    .bind(user_id)
    .execute(pool)
    .await;

    let employee_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_empl-natural\" (id, notice, code, fk_user, created_by_id)
         VALUES (isahl.gen_next_zuid(), '认知测试雇员', $1, $2, 1) RETURNING id",
    )
    .bind(format!("COG-EMP-{}", suffix))
    .bind(user_id)
    .fetch_one(pool)
    .await
    .expect("insert employee");

    let position_code = format!("COG-POS-{}", suffix);
    let position_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_subj-position\" (id, notice, code, created_by_id)
         VALUES (isahl.gen_next_zuid(), '认知测试岗位', $1, 1) RETURNING id",
    )
    .bind(&position_code)
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

    let tag_code = format!("VIEW-COG-{}", suffix);
    let tag_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_tags-post_view\" (id, notice, code, created_by_id)
         VALUES (isahl.gen_next_uid(130), '认知测试视角', $1, 1) RETURNING id",
    )
    .bind(&tag_code)
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
        employee_id,
        position_id,
        position_code,
        tag_id,
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
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(f.user_id)
        .execute(pool)
        .await;
}

/// 清理认知派生 UA 行（按 o_name 硬删，含关联规则先删）
async fn cleanup_cognition_uas(pool: &PgPool, names: &[String]) {
    for name in names {
        let _ = sqlx::query(
            "DELETE FROM isahl_auth.ngac_association WHERE fk_user_attribute IN
             (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1)",
        )
        .bind(name)
        .execute(pool)
        .await;
        let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name = $1")
            .bind(name)
            .execute(pool)
            .await;
    }
}

#[tokio::test]
async fn pip_effective_set_includes_derived_uas_and_ensure_is_idempotent() {
    let pool = connect().await;
    let f = seed_cognition_user(&pool, "t1").await;
    let pos_name = format!("position:{}", f.position_code);
    let view_name = format!("view:{}", f.tag_code);

    let pip = PostgresPip::new(pool.clone());
    let attrs = pip
        .get_all_user_attributes_with_inheritance(f.user_id)
        .await
        .expect("effective UA set");
    let names: Vec<&str> = attrs.iter().map(|a| a.o_name.as_str()).collect();
    assert!(
        names.contains(&pos_name.as_str()),
        "有效 UA 集应含岗位派生 UA: {:?}",
        names
    );
    assert!(
        names.contains(&view_name.as_str()),
        "有效 UA 集应含视角派生 UA: {:?}",
        names
    );

    // ensure 幂等：两次调用后 UA 行数不变（各 1）
    ensure_cognition_uas(&pool, f.user_id).await;
    let cnt: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.ngac_user_attribute WHERE o_name = ANY($1)",
    )
    .bind(&[&pos_name, &view_name][..])
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(cnt, 2, "首次 ensure 后应恰有 2 个派生 UA 行");
    ensure_cognition_uas(&pool, f.user_id).await;
    let cnt2: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.ngac_user_attribute WHERE o_name = ANY($1)",
    )
    .bind(&[&pos_name, &view_name][..])
    .fetch_one(&pool)
    .await
    .expect("count2");
    assert_eq!(cnt2, 2, "二次 ensure 零新行");

    cleanup_cognition_uas(&pool, &[pos_name, view_name]).await;
    cleanup(&pool, &f).await;
}

/// decide + RLS 列表：view: UA 绑 association → 持有者放行；任职软删 → 撤销
#[tokio::test]
async fn derived_view_ua_grants_decide_and_list_until_employment_ends() {
    let pool = connect().await;
    let f = seed_cognition_user(&pool, "t2").await;
    let view_name = format!("view:{}", f.tag_code);

    let pip = PostgresPip::new(pool.clone());
    pip.get_all_user_attributes_with_inheritance(f.user_id)
        .await
        .expect("ensure + effective set");
    let view_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(&view_name)
    .fetch_one(&pool)
    .await
    .expect("view UA");

    // 资源设施：PC + OA + AR + association
    let pc: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("policy class");
    let oa_name = format!("cogtest-oa-{}", f.tag_code);
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at)
         VALUES ($1, $2, 'cogtest', 424242, NOW(), NOW())
         ON CONFLICT (resource_type, fk_resource) DO UPDATE SET deleted_at = NULL",
    )
    .bind(&oa_name)
    .bind(pc)
    .execute(&pool)
    .await
    .expect("OA");
    let oa: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type='cogtest' AND fk_resource=424242 LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("OA id");
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ('cog-read') ON CONFLICT (o_name) DO NOTHING",
    )
    .execute(&pool)
    .await
    .expect("AR");
    let ar: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='cog-read' LIMIT 1",
    )
    .fetch_one(&pool)
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
    .execute(&pool)
    .await
    .expect("association");

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

    let decide = |uid: i64| {
        let app = &app;
        async move {
            let resp = test::call_service(
                app,
                test::TestRequest::post()
                    .uri("/api/ngac/decide")
                    .set_json(
                        json!({"user_id": uid, "resource": "cogtest:424242", "action": "cog-read"}),
                    )
                    .to_request(),
            )
            .await;
            let body: serde_json::Value = test::read_body_json(resp).await;
            body["permitted"].as_bool().unwrap_or(false)
        }
    };

    assert!(decide(f.user_id).await, "持有 view: UA 的用户应被放行");

    // RLS 列表：get_accessible_resource_ids 应含资源行
    let ids = pip
        .get_accessible_resource_ids(f.user_id, "cogtest", "cog-read")
        .await
        .expect("visible ids");
    assert!(
        ids.contains(&424242i64),
        "RLS 可见 id 应含 424242: {:?}",
        ids
    );

    // 任职软删 → 派生撤销（decide NotApplicable → permitted=false）
    let _ = sqlx::query(
        "UPDATE isahl.\"zc_id_subj-post_rr_employee\" SET deleted_at = NOW() WHERE ref_right = $1",
    )
    .bind(f.employee_id)
    .execute(&pool)
    .await
    .expect("soft-delete employment");
    assert!(!decide(f.user_id).await, "任职终止后派生授权应撤销");

    // 清理
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE fk_object_attribute = $1")
        .bind(oa)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE id = $1")
        .bind(oa)
        .execute(&pool)
        .await;
    cleanup_cognition_uas(&pool, &[view_name]).await;
    cleanup(&pool, &f).await;
}

/// /auth/me permissions 矩阵并入派生授权
#[tokio::test]
async fn me_permissions_matrix_includes_derived_grant() {
    let pool = connect().await;
    let f = seed_cognition_user(&pool, "t3").await;
    let view_name = format!("view:{}", f.tag_code);

    let pip = PostgresPip::new(pool.clone());
    pip.get_all_user_attributes_with_inheritance(f.user_id)
        .await
        .expect("ensure + effective set");
    let view_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(&view_name)
    .fetch_one(&pool)
    .await
    .expect("view UA");

    let pc: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("policy class");
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at)
         VALUES ($1, $2, 'cogme', 0, NOW(), NOW())
         ON CONFLICT (resource_type, fk_resource) DO UPDATE SET deleted_at = NULL",
    )
    .bind(format!("cogme-oa-{}", f.tag_code))
    .bind(pc)
    .execute(&pool)
    .await
    .expect("OA");
    let oa: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type='cogme' AND fk_resource=0 LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("OA id");
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ('read') ON CONFLICT (o_name) DO NOTHING",
    )
    .execute(&pool)
    .await
    .expect("AR");
    let ar: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='read' LIMIT 1",
    )
    .fetch_one(&pool)
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
    .execute(&pool)
    .await
    .expect("association");

    let ast = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::auth::login::configure),
    )
    .await;
    let token = mint_token(&ast, f.user_id, &f.email);
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/me")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "me 应 200");
    let body: serde_json::Value = test::read_body_json(resp).await;
    let perms = &body["user"]["permissions"];
    let actions: Vec<&str> = perms["cogme"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
        .unwrap_or_default();
    assert!(
        actions.contains(&"read"),
        "me 权限矩阵应含派生授权 cogme/read: {}",
        perms
    );

    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE fk_object_attribute = $1")
        .bind(oa)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE id = $1")
        .bind(oa)
        .execute(&pool)
        .await;
    cleanup_cognition_uas(&pool, &[view_name]).await;
    cleanup(&pool, &f).await;
}

/// expires_at 写路径（add-ngac-assignment-expires-at）：API 落库，缺省保持 NULL
#[tokio::test]
async fn assignment_expires_at_write_path() {
    let pool = connect().await;
    let f = seed_cognition_user(&pool, "t4").await;
    let pos_name = format!("position:{}", f.position_code);

    let pip = PostgresPip::new(pool.clone());
    pip.get_all_user_attributes_with_inheritance(f.user_id)
        .await
        .expect("ensure + effective set");
    let pos_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(&pos_name)
    .fetch_one(&pool)
    .await
    .expect("position UA");
    let view_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(format!("view:{}", f.tag_code))
    .fetch_one(&pool)
    .await
    .expect("view UA");

    // 非认知用户 B 作为指派对象（避免与派生混淆）
    let email_b = format!("cog_exp_{}@alioth.test", f.tag_code);
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), $2, $1, 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .bind(&email_b)
    .bind(format!("cog_exp_{}", f.tag_code))
    .execute(&pool)
    .await
    .expect("insert user b");
    let user_b: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1")
            .bind(&email_b)
            .fetch_one(&pool)
            .await
            .expect("fetch user b");
    // 前置清理：历史运行残留指派（幂等重跑）
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
        .bind(user_b)
        .execute(&pool)
        .await;

    let ast = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(
                web::scope("/api/admin/ngac/pip")
                    .configure(gateway_sso::ngac::pip::configure_routes),
            ),
    )
    .await;
    let token = mint_token(&ast, f.user_id, &f.email);

    // 带 expires_at → 落库
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/api/admin/ngac/pip/users/{}/attributes", user_b))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(json!({
                "fk_user_attribute": pos_ua,
                "expires_at": "2030-01-01T00:00:00Z",
            }))
            .to_request(),
    )
    .await;
    let resp_status = resp.status();
    let resp_body = test::read_body_json::<serde_json::Value, _>(resp).await;
    assert_eq!(resp_status, 201, "指派应 201: body={:?}", resp_body);
    let stored: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT expires_at FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1 AND fk_user_attribute = $2 AND deleted_at IS NULL",
    )
    .bind(user_b)
    .bind(pos_ua)
    .fetch_one(&pool)
    .await
    .expect("read expires_at");
    assert!(stored.is_some(), "expires_at 应落库非空");

    // 缺省 → NULL（既有行为不变；用另一 UA 避免撞 UNIQUE(fk_user, fk_user_attribute)）
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/api/admin/ngac/pip/users/{}/attributes", user_b))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(json!({ "fk_user_attribute": view_ua }))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 201, "缺省指派应 201");
    let stored2: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT expires_at FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1 AND fk_user_attribute = $2 AND deleted_at IS NULL ORDER BY id DESC LIMIT 1",
    )
    .bind(user_b)
    .bind(view_ua)
    .fetch_one(&pool)
    .await
    .expect("read expires_at2");
    assert!(stored2.is_none(), "缺省 expires_at 应为 NULL");

    let _ = sqlx::query(
        "DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1 AND fk_user_attribute = ANY($2)",
    )
    .bind(user_b)
    .bind(vec![pos_ua, view_ua])
    .execute(&pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(user_b)
        .execute(&pool)
        .await;
    cleanup_cognition_uas(&pool, &[pos_name]).await;
    cleanup(&pool, &f).await;
}
