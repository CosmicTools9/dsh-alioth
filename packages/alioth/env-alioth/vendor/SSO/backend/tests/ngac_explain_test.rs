//! NGAC 属性图能力集成测试
//!
//! 覆盖（安全 blocker 要求）：
//!   1. Prohibition CRUD 端点（创建/列表/更新/软删）
//!   2. `/api/ngac/decide/explain` 与 `/api/ngac/decide` 决策结果一致
//!   3. 软删 UA/OA 后决策行为（属性表 deleted_at 过滤语义）

mod common;

use ::common::testing::connect_test_db;
use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use serde_json::json;
use sqlx::PgPool;

struct Seed {
    admin: i64,
    admin_ua: i64,
    fk_policy_class: i64,
    oa: i64,
    #[allow(dead_code)] // 测试断言按需读取
    read_ar: i64,
    delete_ar: i64,
    #[allow(dead_code)] // 测试断言按需读取
    admin_ar: i64,
}

/// 幂等 seed：admin 用户 + admin UA + OA(engineers:0) + read/admin AR + 关联。
/// 返回主键供测试引用；不创建 prohibition（各测试自建）。
async fn seed(pool: &PgPool, email: &str) -> Seed {
    let username = email
        .split('@')
        .next()
        .unwrap_or("ngac_admin")
        .replace('-', "_");
    // 确保 default policy class
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
    let admin: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email=$1 LIMIT 1")
            .bind(email)
            .fetch_one(pool)
            .await
            .expect("admin user");

    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
           VALUES ('admin', $1, NOW(), NOW())
           ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING"#,
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
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(admin)
    .bind(admin_ua)
    .execute(pool)
    .await
    .ok();

    // OA: engineers:0
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at)
           VALUES ('engineers-oa', $1, 'engineers', 0, NOW(), NOW())
           ON CONFLICT (resource_type, fk_resource) DO UPDATE SET deleted_at = NULL"#,
    )
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let oa: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type='engineers' AND fk_resource=0 LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("OA");

    // access rights
    let mut ars = [0i64; 3];
    for (i, name) in ["read", "delete", "admin"].iter().enumerate() {
        sqlx::query("INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ($1) ON CONFLICT (o_name) DO NOTHING")
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

    // association: admin UA → OA read/admin
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_association (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at, updated_at)
           VALUES ($1, $2, ARRAY[$3, $4], $5, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(admin_ua)
    .bind(oa)
    .bind(ars[0])
    .bind(ars[2])
    .bind(pc)
    .execute(pool)
    .await
    .ok();

    Seed {
        admin,
        admin_ua,
        fk_policy_class: pc,
        oa,
        read_ar: ars[0],
        delete_ar: ars[1],
        admin_ar: ars[2],
    }
}

fn test_auth_state() -> AuthState {
    common::test_auth_state()
}

/// 构造 admin 请求头（JWT）。
async fn admin_token(_pool: &PgPool, admin: i64, email: &str) -> String {
    let state = test_auth_state();
    configure_token_validation(
        "http://localhost:9002".to_string(),
        "http://localhost:9002".to_string(),
    );
    encode_access_token(
        &Claims::new(&admin.to_string(), email, false),
        &state.jwt_private_key,
    )
    .expect("encode token")
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
    // 删除测试创建的 OA（含关联规则），避免污染共享测试库（uq_ngac_oa_resource）
    sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE id = $1")
        .bind(oa_id)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_prohibition_crud() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let email = "ngac-admin-crud@test.local";
    let s = seed(&pool, email).await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(test_auth_state()))
            .configure(gateway_sso::admin::configure)
            .route(
                "/api/ngac/decide",
                web::post().to(gateway_sso::ngac::pdp::ngac_decide),
            )
            .route(
                "/api/ngac/decide/explain",
                web::post().to(gateway_sso::ngac::pdp::ngac_decide_explain),
            ),
    )
    .await;
    let token = admin_token(&pool, s.admin, email).await;

    // create
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/admin/ngac/prohibitions")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(json!({
                "o_name": "no-delete-engineers",
                "fk_user_attribute": s.admin_ua,
                "fk_object_attribute": s.oa,
                "ak_access_rights": [s.delete_ar],
                "is_active": true
            }))
            .to_request(),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "create prohibition: {}",
        resp.status()
    );
    let body: serde_json::Value = test::read_body_json(resp).await;
    // create 响应 id 为 zuid 字符串（列表响应同约定）——保持字符串比较
    let pid = body["id"].as_str().unwrap().to_string();

    // list（含名称解析）
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/admin/ngac/prohibitions")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert!(resp.status().is_success());
    let body: serde_json::Value = test::read_body_json(resp).await;
    let found = body["prohibitions"]
        .as_array()
        .expect("prohibitions array")
        .iter()
        .find(|p| p["id"].as_str() == Some(pid.as_str()))
        .cloned();
    let p = found.expect("created prohibition in list");
    assert_eq!(p["user_attribute"], "admin", "resolved UA name");
    assert!(
        !p["object_attribute"].as_str().unwrap_or("").is_empty(),
        "resolved OA name"
    );
    assert_eq!(p["access_rights"][0], "delete", "resolved AR name");
    assert_eq!(p["is_active"], true);

    // update（改 is_active=false）
    let resp = test::call_service(
        &app,
        test::TestRequest::put()
            .uri(&format!("/api/admin/ngac/prohibitions/{}", pid))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(json!({"is_active": false}))
            .to_request(),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "update prohibition: {}",
        resp.status()
    );

    // delete
    let resp = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri(&format!("/api/admin/ngac/prohibitions/{}", pid))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "delete prohibition: {}",
        resp.status()
    );

    cleanup(&pool, email, s.oa).await;
}

#[tokio::test]
async fn test_explain_matches_decide_and_soft_delete_oa() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let email = "ngac-admin-exp@test.local";
    let s = seed(&pool, email).await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(test_auth_state()))
            .configure(gateway_sso::admin::configure)
            .route(
                "/api/ngac/decide",
                web::post().to(gateway_sso::ngac::pdp::ngac_decide),
            )
            .route(
                "/api/ngac/decide/explain",
                web::post().to(gateway_sso::ngac::pdp::ngac_decide_explain),
            ),
    )
    .await;

    // 1) 关联授权 read → permit；explain 一致
    let token = admin_token(&pool, s.admin, email).await;
    let d = {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/ngac/decide")
                .set_json(json!({
                    "user_id": s.admin,
                    "resource": "engineers:0",
                    "action": "read"
                }))
                .to_request(),
        )
        .await;
        let body: serde_json::Value = test::read_body_json(resp).await;
        body
    };
    assert_eq!(
        d["permitted"], true,
        "admin read engineers:0 (association): {:?}",
        d
    );

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide/explain")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(json!({
                "user_id": s.admin,
                "resource": "engineers:0",
                "action": "read"
            }))
            .to_request(),
    )
    .await;
    if !resp.status().is_success() {
        let status = resp.status();
        let body_text: serde_json::Value = test::read_body_json(resp).await;
        panic!("explain failed: {} body={:?}", status, body_text);
    }
    let e: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(e["permitted"], d["permitted"], "explain == decide (read)");
    assert_eq!(e["outcome"], "permit");
    let matched: Vec<&serde_json::Value> = e["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .filter(|s| s["matched"] == true)
        .collect();
    assert!(!matched.is_empty(), "explain has matched step");
    assert_eq!(matched[0]["rule_type"], "association");
    assert_eq!(matched[0]["kind"], "allow");

    // 2) 无 delete 关联 → not applicable；explain 一致
    let d2 = {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/ngac/decide")
                .set_json(json!({
                    "user_id": s.admin,
                    "resource": "engineers:0",
                    "action": "delete"
                }))
                .to_request(),
        )
        .await;
        let body: serde_json::Value = test::read_body_json(resp).await;
        body
    };
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide/explain")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(json!({
                "user_id": s.admin,
                "resource": "engineers:0",
                "action": "delete"
            }))
            .to_request(),
    )
    .await;
    let e2: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(
        e2["permitted"], d2["permitted"],
        "explain == decide (delete)"
    );

    // 3) 软删 OA → 决策必须不再放行（PIP 对象侧过滤 deleted_at 的语义）
    sqlx::query("UPDATE isahl_auth.ngac_object_attribute SET deleted_at = NOW() WHERE id = $1")
        .bind(s.oa)
        .execute(&pool)
        .await
        .expect("soft delete OA");

    // 重新加载策略（PDP 每次 decide 重新 load_policy_from_db → 无需清缓存）
    let d3 = {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/ngac/decide")
                .set_json(json!({
                    "user_id": s.admin,
                    "resource": "engineers:0",
                    "action": "read"
                }))
                .to_request(),
        )
        .await;
        let body: serde_json::Value = test::read_body_json(resp).await;
        body
    };
    // s.admin 持 admin UA——decide_access 的 admin 豁免（全量放行）是既有语义，
    // 软删 OA 后仍 Permit；「软删回收」语义须非 admin 用户验证（本测试无该
    // 用户，由 ngac_soft_delete_test 覆盖）。此处断言豁免行为 + explain 一致性。
    assert_eq!(
        d3["permitted"], true,
        "admin 豁免下软删 OA 仍放行: {:?}",
        d3
    );
    let e3 = {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/ngac/decide/explain")
                .insert_header(("Authorization", format!("Bearer {}", token)))
                .set_json(json!({
                    "user_id": s.admin,
                    "resource": "engineers:0",
                    "action": "read"
                }))
                .to_request(),
        )
        .await;
        let body: serde_json::Value = test::read_body_json(resp).await;
        body
    };
    assert_eq!(
        e3["permitted"], d3["permitted"],
        "explain == decide (soft-delete, admin 豁免)"
    );

    // 恢复（其它测试共享该 OA）
    sqlx::query("UPDATE isahl_auth.ngac_object_attribute SET deleted_at = NULL WHERE id = $1")
        .bind(s.oa)
        .execute(&pool)
        .await
        .ok();

    cleanup(&pool, email, s.oa).await;
}

#[tokio::test]
async fn test_explain_rejects_non_admin() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let email = "ngac-admin-rej@test.local";
    let s = seed(&pool, email).await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(test_auth_state()))
            .configure(gateway_sso::admin::configure)
            .route(
                "/api/ngac/decide",
                web::post().to(gateway_sso::ngac::pdp::ngac_decide),
            )
            .route(
                "/api/ngac/decide/explain",
                web::post().to(gateway_sso::ngac::pdp::ngac_decide_explain),
            ),
    )
    .await;

    // 普通用户（无 admin UA）
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users (id, name, username, email, status, is_active, created_at, updated_at)
           VALUES (isahl.gen_next_zuid(), 'ngac_regular', 'ngac_regular', 'ngac-regular@test.local', 'active', true, NOW(), NOW())
           ON CONFLICT (email) DO NOTHING"#,
    )
    .execute(&pool)
    .await
    .ok();
    let regular: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE email='ngac-regular@test.local' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("regular user");
    let regular_token = admin_token(&pool, regular, "ngac-regular@test.local").await;

    // 非管理员 explain → 401/403（策略路径属敏感信息，禁止泄露）
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide/explain")
            .insert_header(("Authorization", format!("Bearer {}", regular_token)))
            .set_json(json!({
                "user_id": regular,
                "resource": "engineers:0",
                "action": "read"
            }))
            .to_request(),
    )
    .await;
    assert!(
        resp.status().is_client_error(),
        "non-admin explain must be rejected, got {}",
        resp.status()
    );

    // 无 token → 401
    let resp2 = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide/explain")
            .set_json(json!({
                "user_id": s.admin,
                "resource": "engineers:0",
                "action": "read"
            }))
            .to_request(),
    )
    .await;
    assert!(
        resp2.status().is_client_error(),
        "no-token explain must be rejected"
    );

    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE email='ngac-regular@test.local'")
        .execute(&pool)
        .await
        .ok();
    cleanup(&pool, email, s.oa).await;
}

#[tokio::test]
async fn test_ancestor_write_integrity() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let email = "ngac-integrity@test.local";
    // 清理上次运行残留（唯一约束 idx_ngac_ua_name_pc_unique）
    sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name IN ('child-of-admin','orphan-attr','cross-domain-attr')")
        .execute(&pool)
        .await
        .ok();
    let s = seed(&pool, email).await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(test_auth_state()))
            .configure(gateway_sso::admin::configure),
    )
    .await;
    let token = admin_token(&pool, s.admin, email).await;
    let hdr = ("Authorization", format!("Bearer {}", token));

    // 1) 创建 UA 带合法父属性 → 生效
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/admin/user-attributes")
            .insert_header(hdr.clone())
            .set_json(json!({
                "o_name": "child-of-admin",
                "fk_policy_class": s.fk_policy_class,
                "ancestor_ids": [s.admin_ua]
            }))
            .to_request(),
    )
    .await;
    if !resp.status().is_success() {
        let status = resp.status();
        let b: serde_json::Value = test::read_body_json(resp).await;
        panic!("create UA with parent failed: {} body={:?}", status, b);
    }
    let child = {
        let body: serde_json::Value = test::read_body_json(resp).await;
        body["id"].as_str().unwrap().to_string()
    };
    // list 校验 ancestor_ids 持久化
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/admin/user-attributes")
            .insert_header(hdr.clone())
            .to_request(),
    )
    .await;
    let body: serde_json::Value = test::read_body_json(resp).await;
    let found = body["attributes"]
        .as_array()
        .expect("attrs")
        .iter()
        .find(|a| a["id"].as_str() == Some(child.as_str()))
        .cloned()
        .expect("child in list");
    assert_eq!(
        found["ancestor_ids"][0]
            .as_str()
            .unwrap()
            .parse::<i64>()
            .unwrap(),
        s.admin_ua,
        "ancestor persisted"
    );

    // 2) 环：更新 admin UA 的父为 child → 拒绝（admin 是 child 的父）
    let admin_anc_before: Vec<i64> = sqlx::query_scalar(
        "SELECT COALESCE(ancestor_ids, '{}') FROM isahl_auth.ngac_user_attribute WHERE id = $1",
    )
    .bind(s.admin_ua)
    .fetch_one(&pool)
    .await
    .expect("admin ancestor before");
    let resp = test::call_service(
        &app,
        test::TestRequest::put()
            .uri(&format!("/api/admin/user-attributes/{}", s.admin_ua))
            .insert_header(hdr.clone())
            .set_json(json!({"ancestor_ids": [child]}))
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status().as_u16(),
        400,
        "cycle update must be rejected (400)"
    );
    // DB 未变
    let anc: Vec<i64> = sqlx::query_scalar(
        "SELECT COALESCE(ancestor_ids, '{}') FROM isahl_auth.ngac_user_attribute WHERE id = $1",
    )
    .bind(s.admin_ua)
    .fetch_one(&pool)
    .await
    .expect("admin ancestor");
    assert_eq!(
        anc, admin_anc_before,
        "admin ancestor unchanged after rejected cycle"
    );

    // 3) 父不存在 → 拒绝
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/admin/user-attributes")
            .insert_header(hdr.clone())
            .set_json(json!({
                "o_name": "orphan-attr",
                "ancestor_ids": [999999999999i64]
            }))
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status().as_u16(),
        400,
        "missing parent must be rejected"
    );

    // 4) 跨策略类 → 拒绝（创建另一策略类）
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_policy_class (o_name) VALUES ('other') ON CONFLICT DO NOTHING",
    )
    .execute(&pool)
    .await
    .ok();
    let other_pc: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='other' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("other pc");
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/admin/user-attributes")
            .insert_header(hdr.clone())
            .set_json(json!({
                "o_name": "cross-domain-attr",
                "fk_policy_class": other_pc,
                "ancestor_ids": [s.admin_ua]
            }))
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status().as_u16(),
        400,
        "cross-policy-class must be rejected"
    );

    // 5) 自引用 → 拒绝（更新自身为父）
    let resp = test::call_service(
        &app,
        test::TestRequest::put()
            .uri(&format!("/api/admin/user-attributes/{}", child))
            .insert_header(hdr.clone())
            .set_json(json!({"ancestor_ids": [child]}))
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status().as_u16(),
        400,
        "self-reference must be rejected"
    );

    // 清理
    sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name IN ('child-of-admin','orphan-attr','cross-domain-attr')")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.ngac_policy_class WHERE o_name='other'")
        .execute(&pool)
        .await
        .ok();
    cleanup(&pool, email, s.oa).await;
}

#[tokio::test]
async fn test_ancestor_diamond_dag_allowed() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let email = "ngac-diamond@test.local";
    // 前置清理：失败运行残留（幂等重跑）
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_user_attribute \
         WHERE o_name IN ('diamond-a','diamond-b','diamond-c','diamond-d')",
    )
    .execute(&pool)
    .await
    .ok();
    let s = seed(&pool, email).await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(test_auth_state()))
            .configure(gateway_sso::admin::configure),
    )
    .await;
    let token = admin_token(&pool, s.admin, email).await;
    let hdr = ("Authorization", format!("Bearer {}", token));

    // 构造菱形 DAG：A ← B、A ← C、{B, C} ← D（D 的两个父共享祖先 A —— 合法，非环）
    let a: String = {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/admin/user-attributes")
                .insert_header(hdr.clone())
                .set_json(json!({
                    "o_name": "diamond-a",
                    "fk_policy_class": s.fk_policy_class,
                    "ancestor_ids": []
                }))
                .to_request(),
        )
        .await;
        assert!(
            resp.status().is_success(),
            "create diamond-a: {}",
            resp.status()
        );
        let body: serde_json::Value = test::read_body_json(resp).await;
        body["id"].as_str().unwrap().to_string()
    };
    let b: String = {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/admin/user-attributes")
                .insert_header(hdr.clone())
                .set_json(json!({
                    "o_name": "diamond-b",
                    "fk_policy_class": s.fk_policy_class,
                    "ancestor_ids": [a]
                }))
                .to_request(),
        )
        .await;
        assert!(
            resp.status().is_success(),
            "create diamond-b: {}",
            resp.status()
        );
        let body: serde_json::Value = test::read_body_json(resp).await;
        body["id"].as_str().unwrap().to_string()
    };
    let c: String = {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/admin/user-attributes")
                .insert_header(hdr.clone())
                .set_json(json!({
                    "o_name": "diamond-c",
                    "fk_policy_class": s.fk_policy_class,
                    "ancestor_ids": [a]
                }))
                .to_request(),
        )
        .await;
        assert!(
            resp.status().is_success(),
            "create diamond-c: {}",
            resp.status()
        );
        let body: serde_json::Value = test::read_body_json(resp).await;
        body["id"].as_str().unwrap().to_string()
    };
    let d: String = {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/admin/user-attributes")
                .insert_header(hdr.clone())
                .set_json(json!({
                    "o_name": "diamond-d",
                    "fk_policy_class": s.fk_policy_class,
                    "ancestor_ids": [b, c]
                }))
                .to_request(),
        )
        .await;
        assert!(
            resp.status().is_success(),
            "create diamond-d: {}",
            resp.status()
        );
        let body: serde_json::Value = test::read_body_json(resp).await;
        body["id"].as_str().unwrap().to_string()
    }; // 共享祖先 A 的两个父——创建成功即证明未误判为环

    // 真环：B 的父改为 D（B → D → B）→ 拒绝
    let resp = test::call_service(
        &app,
        test::TestRequest::put()
            .uri(&format!("/api/admin/user-attributes/{}", b))
            .insert_header(hdr.clone())
            .set_json(json!({"ancestor_ids": [d]}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 400, "true cycle must be rejected");

    sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'diamond-%'")
        .execute(&pool)
        .await
        .ok();
    cleanup(&pool, email, s.oa).await;
}
