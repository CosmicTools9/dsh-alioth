//! 权限申请闭环集成测试（add-ngac-access-request）
//!
//! 覆盖：
//! - 本人发起申请 → 重复 pending 409 → 管理列表可见
//! - approve：指派落库 + 决策生效 + 申请 approved（复用 assign_ua_with_audit_tx）
//! - reject：状态 rejected + 无新增权限
//! - 非 admin 审批 403；无 token 401
//! - 表 `isahl_auth.ngac_access_request` 由本文件幂等自建（测试基建模式，
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
    // NGAC 运行时自愈（lib 唯一实现：auth_users + 扩展表 + 030 触发器 +
    // 核心约束 + 审计分区 + 种子 AR；生产 PDP 路径同源自愈）
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
    ua_id: i64,
}

async fn seed(pool: &PgPool, suffix: &str) -> Fixture {
    ensure_table(pool).await;
    // 确保 default PC
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

    // 申请用户 U（无 admin）
    let email = format!("req_user_{}@alioth.test", suffix);
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), $2, $1, 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .bind(&email)
    .bind(format!("req_user_{}", suffix))
    .execute(pool)
    .await
    .expect("insert user");
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1")
            .bind(&email)
            .fetch_one(pool)
            .await
            .expect("fetch user");

    // 前置清理：历史失败运行的残留（申请 + 指派；幂等重跑）
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_access_request WHERE fk_user = $1")
        .bind(user_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
        .bind(user_id)
        .execute(pool)
        .await;

    // 管理员 A（admin UA 关联）
    let admin_email = format!("req_admin_{}@alioth.test", suffix);
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), $2, $1, 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .bind(&admin_email)
    .bind(format!("req_admin_{}", suffix))
    .execute(pool)
    .await
    .expect("insert admin");
    let admin_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1")
            .bind(&admin_email)
            .fetch_one(pool)
            .await
            .expect("fetch admin");
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

    // 目标 UA（审批指派目标）
    let ua_name = format!("req-target-{}", suffix);
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
         VALUES ($1, $2, NOW(), NOW())
         ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING",
    )
    .bind(&ua_name)
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let ua_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(&ua_name)
    .fetch_one(pool)
    .await
    .expect("target UA");

    // 关联：target UA → reqres/read（集合级 OA）
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at)
         VALUES ($1, $2, 'reqres', 0, NOW(), NOW())
         ON CONFLICT (resource_type, fk_resource) DO UPDATE SET deleted_at = NULL",
    )
    .bind(format!("reqres-oa-{}", suffix))
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let oa: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type='reqres' AND fk_resource=0 LIMIT 1",
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
        "INSERT INTO isahl_auth.ngac_association (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at, updated_at)
         VALUES ($1, $2, ARRAY[$3], $4, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(ua_id)
    .bind(oa)
    .bind(ar)
    .bind(pc)
    .execute(pool)
    .await
    .ok();

    Fixture {
        user_id,
        email,
        admin_id,
        admin_email,
        ua_id,
    }
}

async fn cleanup(pool: &PgPool, f: &Fixture) {
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_access_request WHERE fk_user = $1")
        .bind(f.user_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
        .bind(f.user_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE fk_user_attribute = $1")
        .bind(f.ua_id)
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type='reqres' AND fk_resource=0",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE id = $1")
        .bind(f.ua_id)
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
async fn access_request_full_loop_approve() {
    let pool = connect().await;
    let f = seed(&pool, "b1").await;
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

    // 1. 用户发起申请
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/access-request")
            .insert_header(("Authorization", format!("Bearer {}", user_token)))
            .set_json(json!({"resource_type": "reqres", "action": "read", "reason": "工作需要"}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 201, "发起应 201");
    let created: serde_json::Value = test::read_body_json(resp).await;
    let req_id: i64 = created["id"].as_str().unwrap().parse().expect("id");

    // 2. 重复 pending → 409
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/access-request")
            .insert_header(("Authorization", format!("Bearer {}", user_token)))
            .set_json(json!({"resource_type": "reqres", "action": "read"}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 409, "重复 pending 应 409");

    // 3. 无 token → 401
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/access-request")
            .set_json(json!({"resource_type": "reqres", "action": "read"}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 401, "无 token 应 401");

    // 4. 管理列表可见
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/admin/ngac/access-requests")
            .insert_header(("Authorization", format!("Bearer {}", admin_token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "管理列表应 200");
    let list: serde_json::Value = test::read_body_json(resp).await;
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"].as_str() == Some(&req_id.to_string())),
        "列表应含申请"
    );

    // 5. 非 admin 审批 → 403
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/api/admin/ngac/access-requests/{}/approve",
                req_id
            ))
            .insert_header(("Authorization", format!("Bearer {}", user_token)))
            .set_json(json!({"fk_user_attribute": f.ua_id}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 403, "非 admin 审批应 403");

    // 6. admin approve → 指派落库 + 决策生效
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/api/admin/ngac/access-requests/{}/approve",
                req_id
            ))
            .insert_header(("Authorization", format!("Bearer {}", admin_token)))
            .set_json(json!({"fk_user_attribute": f.ua_id}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "approve 应 200: {:?}", resp);
    let assigned: Option<i64> = sqlx::query_scalar(
        "SELECT fk_user_attribute FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user = $1 AND fk_user_attribute = $2 AND deleted_at IS NULL",
    )
    .bind(f.user_id)
    .bind(f.ua_id)
    .fetch_optional(&pool)
    .await
    .expect("read assignment");
    assert_eq!(assigned, Some(f.ua_id), "指派应落库");
    // 决策生效（PDP 全局图已含关联；版本触发器已 bump）
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide")
            .set_json(json!({"user_id": f.user_id, "resource": "reqres:0", "action": "read"}))
            .to_request(),
    )
    .await;
    let d: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(d["permitted"], true, "审批后应放行: {}", d);

    // 7. 已审结 → 409
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/api/admin/ngac/access-requests/{}/approve",
                req_id
            ))
            .insert_header(("Authorization", format!("Bearer {}", admin_token)))
            .set_json(json!({"fk_user_attribute": f.ua_id}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 409, "已审结应 409");

    // 8. 我的申请列表状态 approved
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/ngac/access-request/me")
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
async fn access_request_reject_path() {
    let pool = connect().await;
    let f = seed(&pool, "b2").await;
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
            .uri("/api/ngac/access-request")
            .insert_header(("Authorization", format!("Bearer {}", user_token)))
            .set_json(json!({"resource_type": "reqres", "action": "write"}))
            .to_request(),
    )
    .await;
    let created: serde_json::Value = test::read_body_json(resp).await;
    let req_id: i64 = created["id"].as_str().unwrap().parse().expect("id");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/api/admin/ngac/access-requests/{}/reject",
                req_id
            ))
            .insert_header(("Authorization", format!("Bearer {}", admin_token)))
            .set_json(json!({"reason": "无业务必要性"}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "reject 应 200");
    let row: (String, Option<String>) =
        sqlx::query_as("SELECT status, reason FROM isahl_auth.ngac_access_request WHERE id = $1")
            .bind(req_id)
            .fetch_one(&pool)
            .await
            .expect("read request");
    assert_eq!(row.0, "rejected");
    assert_eq!(row.1.as_deref(), Some("无业务必要性"));

    // 无新增权限
    let assigned: Option<i64> = sqlx::query_scalar(
        "SELECT fk_user_attribute FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(f.user_id)
    .fetch_optional(&pool)
    .await
    .expect("read assignment");
    assert!(assigned.is_none(), "reject 不应创建指派");

    cleanup(&pool, &f).await;
}
