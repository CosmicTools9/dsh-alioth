//! 图快照端点集成测试（refactor-ngac-admin-nl-graph）
//!
//! 覆盖：
//! - `graph_snapshot` 聚合结构完整性（版本/UA 持有者/OA 展示名/边名解析/认知标记）
//! - `GET /api/admin/ngac/graph` admin 门控（非 admin 403 / admin 200）
#![allow(clippy::type_complexity)]

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use gateway_sso::ngac::graph;
use sqlx::PgPool;

async fn connect() -> PgPool {
    ::common::testing::connect_test_db().await
}

async fn ensure_table(pool: &PgPool) {
    common::ensure_auth_users(pool)
        .await
        .expect("ensure auth_users");
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
    admin_ua_id: i64,
    ua_id: i64,
    oa_id: i64,
    assoc_id: i64,
}

/// admin（UA 指派持有者）+ operator UA + collection OA + read association。
async fn seed(pool: &PgPool, suffix: &str) -> Fixture {
    ensure_table(pool).await;
    let email = format!("graph_user_{}@alioth.test", suffix);
    let admin_email = format!("graph_admin_{}@alioth.test", suffix);
    for (mail, name) in [
        (&email, format!("graph_user_{}", suffix)),
        (&admin_email, format!("graph_admin_{}", suffix)),
    ] {
        sqlx::query(
            "INSERT INTO isahl_auth.auth_users (id, username, name, email, status, is_active, created_at, updated_at)
             VALUES (isahl.gen_next_zuid(), $2, $2, $1, 'active', true, NOW(), NOW())
             ON CONFLICT (email) DO UPDATE SET username = EXCLUDED.username",
        )
        .bind(mail)
        .bind(&name)
        .execute(pool)
        .await
        .expect("insert user");
    }
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1")
            .bind(&email)
            .fetch_one(pool)
            .await
            .expect("fetch user");
    let admin_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1")
            .bind(&admin_email)
            .fetch_one(pool)
            .await
            .expect("fetch admin");

    sqlx::query("INSERT INTO isahl_auth.ngac_policy_class (o_name) VALUES ('default') ON CONFLICT DO NOTHING")
        .execute(pool)
        .await
        .ok();
    let pc: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("policy class");

    // UA：admin（给 fixture admin 持有）+ operator（本测试断言对象）
    for ua_name in ["admin", "operator"] {
        let _ = sqlx::query(
            "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
             VALUES ($1, $2, NOW(), NOW())
             ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING",
        )
        .bind(ua_name)
        .bind(pc)
        .execute(pool)
        .await;
    }
    let admin_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='admin' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("admin UA");
    let ua_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='operator' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("operator UA");
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
         VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(admin_id)
    .bind(admin_ua)
    .execute(pool)
    .await
    .ok();

    // access right：read（幂等）
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ('read') ON CONFLICT (o_name) DO NOTHING",
    )
    .execute(pool)
    .await
    .ok();
    let read_right_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='read' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("read right");

    // collection OA + association
    let oa_name = format!("graph-snap-{}-collection", suffix);
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.ngac_association WHERE fk_object_attribute IN
           (SELECT id FROM isahl_auth.ngac_object_attribute WHERE o_name = $1)",
    )
    .bind(&oa_name)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE o_name = $1")
        .bind(&oa_name)
        .execute(pool)
        .await;
    let oa_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_object_attribute
             (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at)
         VALUES ($1, $2, $3, 0, NOW(), NOW()) RETURNING id",
    )
    .bind(&oa_name)
    .bind(pc)
    .bind(format!("graph_snap_{}", suffix))
    .fetch_one(pool)
    .await
    .expect("insert OA");
    let assoc_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_association
             (fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights, created_at, updated_at)
         VALUES ($1, $2, $3, ARRAY[$4]::BIGINT[], NOW(), NOW()) RETURNING id",
    )
    .bind(ua_id)
    .bind(oa_id)
    .bind(pc)
    .bind(read_right_id)
    .fetch_one(pool)
    .await
    .expect("insert association");

    Fixture {
        user_id,
        email,
        admin_id,
        admin_email,
        admin_ua_id: admin_ua,
        ua_id,
        oa_id,
        assoc_id,
    }
}

async fn cleanup(pool: &PgPool, f: &Fixture, suffix: &str) {
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE id = $1")
        .bind(f.assoc_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE id = $1")
        .bind(f.oa_id)
        .execute(pool)
        .await;
    // 认知 UA（第二用例 ensure 物化的 position:*）
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'position:GRAPH-SNAP-%'",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = ANY($1)")
        .bind(vec![f.user_id, f.admin_id])
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = ANY($1)")
        .bind(vec![f.user_id, f.admin_id])
        .execute(pool)
        .await;
    let _ = format!("_{}", suffix); // suffix 保留供扩展
}

#[tokio::test]
async fn graph_snapshot_structure_complete() {
    let pool = connect().await;
    let f = seed(&pool, "s1").await;

    let snap = graph::graph_snapshot(&pool).await.expect("snapshot");

    // UA：operator 在列，access_rights 目录非空
    let ua = snap
        .user_attributes
        .iter()
        .find(|u| u.id == f.ua_id)
        .expect("operator UA in snapshot");
    assert_eq!(ua.o_name, "operator");

    // admin UA 持有者：直接指派计数 ≥ 1 且名单含 fixture admin 用户名
    let admin_ua = snap
        .user_attributes
        .iter()
        .find(|u| u.id == f.admin_ua_id)
        .expect("admin UA in snapshot");
    assert_eq!(admin_ua.o_name, "admin");
    assert!(admin_ua.holder_count >= 1, "admin holder_count ≥ 1");
    assert!(
        admin_ua
            .holders
            .iter()
            .any(|h| h.starts_with("graph_admin_s1")),
        "holders 含 fixture admin（got {:?}）",
        admin_ua.holders
    );

    // OA：display_name 非空（解析链兜底 o_name）
    let oa = snap
        .object_attributes
        .iter()
        .find(|o| o.id == f.oa_id)
        .expect("OA in snapshot");
    assert!(!oa.display_name.is_empty(), "display_name 非空");

    // 边：association 名解析齐全
    let assoc = snap
        .associations
        .iter()
        .find(|a| a.id == f.assoc_id)
        .expect("association in snapshot");
    assert_eq!(assoc.user_attribute.as_deref(), Some("operator"));
    assert_eq!(assoc.object_attribute.as_deref(), Some(oa.o_name.as_str()));
    assert!(assoc.access_rights.iter().any(|r| r == "read"));

    // 认知标记：ensure 一个 cognition UA 后快照暴露 derived_from
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, property, created_at, updated_at)
         VALUES ($1, (SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default' LIMIT 1),
                 '{\"derived_from\":\"cognition\"}'::JSONB, NOW(), NOW())
         ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING",
    )
    .bind("position:GRAPH-SNAP-S1")
    .execute(&pool)
    .await
    .expect("insert cognition UA");
    let snap2 = graph::graph_snapshot(&pool).await.expect("snapshot 2");
    let cog = snap2
        .user_attributes
        .iter()
        .find(|u| u.o_name == "position:GRAPH-SNAP-S1")
        .expect("cognition UA in snapshot 2");
    assert_eq!(cog.derived_from.as_deref(), Some("cognition"));

    cleanup(&pool, &f, "s1").await;
}

#[tokio::test]
async fn graph_endpoint_admin_gated() {
    let pool = connect().await;
    let f = seed(&pool, "s2").await;
    let ast = common::test_auth_state();
    let user_token = mint_token(&ast, f.user_id, &f.email);
    let admin_token = mint_token(&ast, f.admin_id, &f.admin_email);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::admin::configure),
    )
    .await;
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/admin/ngac/graph")
            .insert_header(("Authorization", format!("Bearer {}", user_token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 403, "非 admin 应 403");

    // admin → 200 + 结构键齐全
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/admin/ngac/graph")
            .insert_header(("Authorization", format!("Bearer {}", admin_token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "admin 应 200");
    let body: serde_json::Value = test::read_body_json(resp).await;
    for key in [
        "version",
        "policy_classes",
        "user_attributes",
        "object_attributes",
        "associations",
        "prohibitions",
        "access_rights",
    ] {
        assert!(body.get(key).is_some(), "响应缺 key: {}", key);
    }
    assert!(
        body["user_attributes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["o_name"] == "operator"),
        "快照应含 operator UA"
    );

    cleanup(&pool, &f, "s2").await;
}
