//! 通用 NGAC 委托集成测试（add-ngac-delegation）
//!
//! 覆盖：
//! - 发起委托（直接持有 UA）→ 被委托人决策放行 → 撤销 → 即时失效
//! - 时间窗外不生效
//! - 链式委托（委托来源再委托）→ 400
//! - 重复 active 时间窗重叠 → 409
//! - me 列表双向隔离 + 服务端过滤
//! - 表 `isahl_auth.ngac_delegation` 由本文件幂等自建（测试基建模式；
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
    u1: i64,
    u1_email: String,
    u2: i64,
    u2_email: String,
    u3: i64,
    u3_email: String,
    ua_id: i64,
}

async fn seed(pool: &PgPool, suffix: &str) -> Fixture {
    ensure_table(pool).await;
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

    let mut ids = [0i64; 3];
    let mut emails = [String::new(), String::new(), String::new()];
    for (i, tag) in ["d1", "d2", "d3"].iter().enumerate() {
        let email = format!("deleg_{}_{}@alioth.test", tag, suffix);
        sqlx::query(
            "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
             VALUES (isahl.gen_next_zuid(), $2, $1, 'active', true, NOW(), NOW())
             ON CONFLICT (email) DO NOTHING",
        )
        .bind(&email)
        .bind(format!("deleg_{}_{}", tag, suffix))
        .execute(pool)
        .await
        .expect("insert user");
        ids[i] =
            sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1")
                .bind(&email)
                .fetch_one(pool)
                .await
                .expect("fetch user");
        emails[i] = email;
    }

    // UA X + 关联（X → deleggres/read）
    let ua_name = format!("deleg-target-{}", suffix);
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
    // U1 直接持有 X
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
         VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(ids[0])
    .bind(ua_id)
    .execute(pool)
    .await
    .ok();
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at)
         VALUES ($1, $2, 'deleggres', 0, NOW(), NOW())
         ON CONFLICT (resource_type, fk_resource) DO UPDATE SET deleted_at = NULL",
    )
    .bind(format!("deleggres-oa-{}", suffix))
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let oa: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type='deleggres' AND fk_resource=0 LIMIT 1",
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
        u1: ids[0],
        u1_email: emails[0].clone(),
        u2: ids[1],
        u2_email: emails[1].clone(),
        u3: ids[2],
        u3_email: emails[2].clone(),
        ua_id,
    }
}

async fn cleanup(pool: &PgPool, f: &Fixture) {
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.ngac_delegation WHERE fk_delegator = ANY($1) OR fk_delegatee = ANY($1)",
    )
    .bind(vec![f.u1, f.u2, f.u3])
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
        .bind(f.u1)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE fk_user_attribute = $1")
        .bind(f.ua_id)
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type='deleggres' AND fk_resource=0",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE id = $1")
        .bind(f.ua_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = ANY($1)")
        .bind(vec![f.u1, f.u2, f.u3])
        .execute(pool)
        .await;
}

#[tokio::test]
async fn delegation_grant_revoke_and_boundaries() {
    let pool = connect().await;
    let f = seed(&pool, "c1").await;
    let ast = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/api/ngac").configure(gateway_sso::ngac::pdp::configure_routes)),
    )
    .await;
    let t1 = mint_token(&ast, f.u1, &f.u1_email);
    let t2 = mint_token(&ast, f.u2, &f.u2_email);
    let t3 = mint_token(&ast, f.u3, &f.u3_email);

    // 委托前 U2 无权限
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide")
            .set_json(json!({"user_id": f.u2, "resource": "deleggres:0", "action": "read"}))
            .to_request(),
    )
    .await;
    let d: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(d["permitted"], false, "委托前 U2 应无权限");

    // U1 委托 X 给 U2（现在 → +1h）
    let now = chrono::Utc::now();
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/delegations")
            .insert_header(("Authorization", format!("Bearer {}", t1)))
            .set_json(json!({
                "fk_user_attribute": f.ua_id,
                "fk_delegatee": f.u2,
                "date_st": now,
                "date_ed": now + chrono::Duration::hours(1),
            }))
            .to_request(),
    )
    .await;
    let resp_status = resp.status();
    let resp_body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(resp_status, 201, "发起委托应 201: body={:?}", resp_body);
    let created = resp_body;
    let del_id: i64 = created["id"].as_str().unwrap().parse().expect("id");

    // 委托后 U2 放行（PIP 派生）
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide")
            .set_json(json!({"user_id": f.u2, "resource": "deleggres:0", "action": "read"}))
            .to_request(),
    )
    .await;
    let d: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(d["permitted"], true, "委托生效 U2 应放行: {}", d);

    // 链式委托：U2（委托来源）再委托给 U3 → 400
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/delegations")
            .insert_header(("Authorization", format!("Bearer {}", t2)))
            .set_json(json!({
                "fk_user_attribute": f.ua_id,
                "fk_delegatee": f.u3,
                "date_st": now,
                "date_ed": now + chrono::Duration::hours(1),
            }))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 400, "链式委托应 400");

    // 重复 active 重叠 → 409
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/delegations")
            .insert_header(("Authorization", format!("Bearer {}", t1)))
            .set_json(json!({
                "fk_user_attribute": f.ua_id,
                "fk_delegatee": f.u2,
                "date_st": now,
                "date_ed": now + chrono::Duration::hours(2),
            }))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 409, "重叠委托应 409");

    // me 列表双向隔离
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/ngac/delegations/me?direction=out")
            .insert_header(("Authorization", format!("Bearer {}", t1)))
            .to_request(),
    )
    .await;
    let out: serde_json::Value = test::read_body_json(resp).await;
    assert!(
        out.as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"].as_str() == Some(&del_id.to_string())),
        "U1 out 应含委托"
    );
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/ngac/delegations/me?direction=in")
            .insert_header(("Authorization", format!("Bearer {}", t2)))
            .to_request(),
    )
    .await;
    let in_: serde_json::Value = test::read_body_json(resp).await;
    assert!(
        in_.as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"].as_str() == Some(&del_id.to_string())),
        "U2 in 应含委托"
    );
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/ngac/delegations/me?direction=out")
            .insert_header(("Authorization", format!("Bearer {}", t3)))
            .to_request(),
    )
    .await;
    let out3: serde_json::Value = test::read_body_json(resp).await;
    assert!(
        !out3
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"].as_str() == Some(&del_id.to_string())),
        "U3 不应见该委托"
    );

    // 撤销 → 即时失效
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/api/ngac/delegations/{}/revoke", del_id))
            .insert_header(("Authorization", format!("Bearer {}", t1)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "撤销应 200");
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide")
            .set_json(json!({"user_id": f.u2, "resource": "deleggres:0", "action": "read"}))
            .to_request(),
    )
    .await;
    let d: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(d["permitted"], false, "撤销后 U2 应无权限: {}", d);

    // 二次撤销 → 409
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/api/ngac/delegations/{}/revoke", del_id))
            .insert_header(("Authorization", format!("Bearer {}", t1)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 409, "重复撤销应 409");

    cleanup(&pool, &f).await;
}

#[tokio::test]
async fn delegation_outside_window_inactive() {
    let pool = connect().await;
    let f = seed(&pool, "c2").await;
    let ast = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/api/ngac").configure(gateway_sso::ngac::pdp::configure_routes)),
    )
    .await;

    // 过去时间窗（已过期）
    let now = chrono::Utc::now();
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_delegation \
         (fk_delegator, fk_delegatee, fk_user_attribute, date_st, date_ed) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(f.u1)
    .bind(f.u2)
    .bind(f.ua_id)
    .bind(now - chrono::Duration::hours(2))
    .bind(now - chrono::Duration::hours(1))
    .execute(&pool)
    .await
    .expect("expired delegation");

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide")
            .set_json(json!({"user_id": f.u2, "resource": "deleggres:0", "action": "read"}))
            .to_request(),
    )
    .await;
    let d: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(d["permitted"], false, "过期委托不应生效: {}", d);

    cleanup(&pool, &f).await;
}
