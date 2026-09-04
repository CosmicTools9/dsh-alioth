//! NGAC 策略变更审计 + 删除影响预览集成测试
//! （change `add-ngac-audit-trail-view` tasks 2.5）
//!
//! 覆盖：
//!   1. 审计行写入：association create → `ngac_policy_audit_log` 同事务落行
//!      （action/entity_type/entity_id/new_values/fk_user 断言）
//!   2. 无变更不留痕：重复 bind（ON CONFLICT DO NOTHING 命中）无新审计行
//!   3. audit-log 端点：entity_type 过滤 + 分页形状
//!   4. impact-preview：association 删除 → lost_allow；prohibition 删除 →
//!      lost_deny；无关联实体删除 → 空 affected
//!
//! 数据约定：`audx-` 前缀，测试前后双清理（含审计行），不残留。

mod common;

use ::common::testing::connect_test_db;
use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use serde_json::{json, Value};
use sqlx::PgPool;

struct Seed {
    admin_user: i64,
    audx_user: i64,
    operator_ua: i64,
    oa: i64,
    pc: i64,
    read_ar: i64,
    delete_ar: i64,
    email: String,
}

/// IdResponse.id 为 i64 原始序列化（无 serde_zuid），兼容 number/string 两种形态
fn id_of(body: &Value) -> i64 {
    body["id"]
        .as_i64()
        .or_else(|| body["id"].as_str()?.parse::<i64>().ok())
        .expect("id as i64 or string")
}

/// 文件内串行化：4 个测试共享 `audx-` 前缀数据与同一测试库，
/// cleanup 会互删——持锁保证 seed→断言→cleanup 原子完成。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_auth_state() -> AuthState {
    common::test_auth_state()
}

async fn ensure_user(pool: &PgPool, email: &str) -> i64 {
    let username = email.split('@').next().unwrap_or("audx").replace('-', "_");
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
    sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email=$1 LIMIT 1")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("test user")
}

/// 清理 audx-* 测试数据（含审计行——entity_id 引用测试实体或 actor 为测试用户）。
async fn cleanup(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_policy_audit_log \
         WHERE fk_user IN (SELECT id FROM isahl_auth.auth_users WHERE email LIKE 'audx-%@test.local') \
            OR new_values::text LIKE '%audx-%' OR old_values::text LIKE '%audx-%'",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user IN (SELECT id FROM isahl_auth.auth_users WHERE email LIKE 'audx-%@test.local')",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE email LIKE 'audx-%@test.local'")
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_prohibition \
         WHERE fk_user_attribute IN (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'audx-%')",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_association \
         WHERE fk_user_attribute IN (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'audx-%')",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type = 'audxmod'")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'audx-%'")
        .execute(pool)
        .await
        .ok();
}

/// seed：admin 用户（真实 admin UA）+ audx-operator UA + audx-user 绑定 +
/// 集合级 OA（audxmod:0）。association/prohibition 由各测试按需经 API 创建
/// （顺带覆盖审计写入路径）。
async fn seed(pool: &PgPool) -> Seed {
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

    let email = "audx-admin@test.local".to_string();
    let admin_user = ensure_user(pool, &email).await;
    let audx_user = ensure_user(pool, "audx-user@test.local").await;

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
    .bind(admin_user)
    .bind(admin_ua)
    .execute(pool)
    .await
    .ok();

    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
           VALUES ('audx-operator', $1, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let operator_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='audx-operator' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("audx-operator UA");
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(audx_user)
    .bind(operator_ua)
    .execute(pool)
    .await
    .ok();

    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at)
           VALUES ('audx-modules', $1, 'audxmod', 0, NOW(), NOW())
           ON CONFLICT (resource_type, fk_resource) DO UPDATE SET deleted_at = NULL"#,
    )
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let oa: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type='audxmod' AND fk_resource=0 LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("audx OA");

    let read_ar: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='read' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("read AR");
    let delete_ar: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='delete' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("delete AR");

    Seed {
        admin_user,
        audx_user,
        operator_ua,
        oa,
        pc,
        read_ar,
        delete_ar,
        email,
    }
}

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

/// 构造 association 创建请求体（各测试内联 call_service，规避 actix_http
/// 未作为测试 crate 直接依赖导致的 Service 泛型签名问题）。
fn create_assoc_body(s: &Seed) -> Value {
    json!({
        "o_name": "audx-assoc",
        "fk_user_attribute": s.operator_ua.to_string(),
        "fk_object_attribute": s.oa.to_string(),
        "ak_access_rights": [s.read_ar.to_string()],
        "fk_policy_class": s.pc.to_string(),
    })
}

#[tokio::test]
async fn audit_row_written_on_association_create() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let _serial = SERIAL.lock().await;
    cleanup(&pool).await;
    let s = seed(&pool).await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(test_auth_state()))
            .configure(gateway_sso::admin::configure),
    )
    .await;
    let token = admin_token(&pool, s.admin_user, &s.email).await;

    let assoc_id = {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/admin/ngac/associations")
                .insert_header(("Authorization", format!("Bearer {}", token)))
                .set_json(create_assoc_body(&s))
                .to_request(),
        )
        .await;
        assert!(
            resp.status().is_success(),
            "create association: {}",
            resp.status()
        );
        let body: Value = test::read_body_json(resp).await;
        id_of(&body)
    };

    // 审计行同事务落库
    let row: Option<(String, String, i64, Option<i64>, serde_json::Value)> = sqlx::query_as(
        "SELECT action, entity_type, entity_id, fk_user, new_values \
         FROM isahl_auth.ngac_policy_audit_log WHERE entity_type='association' AND entity_id=$1",
    )
    .bind(assoc_id)
    .fetch_optional(&pool)
    .await
    .expect("audit query");
    let (action, entity_type, entity_id, fk_user, new_values) =
        row.expect("audit row for created association");
    assert_eq!(action, "insert");
    assert_eq!(entity_type, "association");
    assert_eq!(entity_id, assoc_id);
    assert_eq!(fk_user, Some(s.admin_user), "actor = operating admin");
    assert_eq!(
        new_values["fk_user_attribute"]
            .as_i64()
            .map(|v| v.to_string()),
        Some(s.operator_ua.to_string()),
        "new_values mirrors row"
    );

    cleanup(&pool).await;
}

#[tokio::test]
async fn audit_noop_bind_leaves_no_row() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let _serial = SERIAL.lock().await;
    cleanup(&pool).await;
    let s = seed(&pool).await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(test_auth_state()))
            .configure(gateway_sso::admin::configure),
    )
    .await;
    let token = admin_token(&pool, s.admin_user, &s.email).await;

    // 先解绑（seed 直插未留痕），再经 API 绑两次
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user=$1 AND fk_user_attribute=$2",
    )
    .bind(s.audx_user)
    .bind(s.operator_ua)
    .execute(&pool)
    .await
    .ok();

    let do_bind = |token: &str| {
        test::TestRequest::post()
            .uri(&format!("/api/admin/users/{}/attributes/bind", s.audx_user))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(json!({"fk_user_attribute": s.operator_ua.to_string()}))
            .to_request()
    };

    let resp = test::call_service(&app, do_bind(&token)).await;
    assert!(resp.status().is_success(), "first bind: {}", resp.status());
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl_auth.ngac_policy_audit_log \
         WHERE entity_type='user_assignment' AND fk_user=$1",
    )
    .bind(s.admin_user)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(count, 1, "first bind writes audit row");

    let resp = test::call_service(&app, do_bind(&token)).await;
    assert!(resp.status().is_success(), "second bind: {}", resp.status());
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl_auth.ngac_policy_audit_log \
         WHERE entity_type='user_assignment' AND fk_user=$1",
    )
    .bind(s.admin_user)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(count, 1, "ON CONFLICT no-op must not write audit row");

    cleanup(&pool).await;
}

#[tokio::test]
async fn audit_log_endpoint_filters_and_paginates() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let _serial = SERIAL.lock().await;
    cleanup(&pool).await;
    let s = seed(&pool).await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(test_auth_state()))
            .configure(gateway_sso::admin::configure),
    )
    .await;
    let token = admin_token(&pool, s.admin_user, &s.email).await;

    let assoc_id = {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/admin/ngac/associations")
                .insert_header(("Authorization", format!("Bearer {}", token)))
                .set_json(create_assoc_body(&s))
                .to_request(),
        )
        .await;
        assert!(
            resp.status().is_success(),
            "create association: {}",
            resp.status()
        );
        let body: Value = test::read_body_json(resp).await;
        id_of(&body)
    };

    // entity_type 过滤：association 行可见
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/admin/ngac/audit-log?entity_type=association&limit=10&offset=0")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert!(resp.status().is_success(), "audit-log: {}", resp.status());
    let body: Value = test::read_body_json(resp).await;
    let rows = body["rows"].as_array().expect("rows");
    assert!(body["total"].as_i64().expect("total") >= 1);
    let row = rows
        .iter()
        .find(|r| r["entity_id"] == json!(assoc_id.to_string()))
        .expect("created association in audit rows");
    assert_eq!(row["action"], "insert");
    assert_eq!(row["entity_type"], "association");
    assert_eq!(row["fk_user"], json!(s.admin_user.to_string()));
    assert!(row["actor_username"].is_string(), "actor resolved via JOIN");

    // 过滤排除：entity_type=prohibition 不含该行
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/admin/ngac/audit-log?entity_type=prohibition")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    let body: Value = test::read_body_json(resp).await;
    let rows = body["rows"].as_array().expect("rows");
    assert!(
        !rows
            .iter()
            .any(|r| r["entity_id"] == json!(assoc_id.to_string())),
        "filter excludes other entity types"
    );

    cleanup(&pool).await;
}

#[tokio::test]
async fn impact_preview_association_and_prohibition() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let _serial = SERIAL.lock().await;
    cleanup(&pool).await;
    let s = seed(&pool).await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(test_auth_state()))
            .configure(gateway_sso::admin::configure),
    )
    .await;
    let token = admin_token(&pool, s.admin_user, &s.email).await;

    // association [read] + prohibition [delete]
    let assoc_id = {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/admin/ngac/associations")
                .insert_header(("Authorization", format!("Bearer {}", token)))
                .set_json(create_assoc_body(&s))
                .to_request(),
        )
        .await;
        assert!(
            resp.status().is_success(),
            "create association: {}",
            resp.status()
        );
        let body: Value = test::read_body_json(resp).await;
        id_of(&body)
    };
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/admin/ngac/prohibitions")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(json!({
                "o_name": "audx-no-delete",
                "fk_user_attribute": s.operator_ua.to_string(),
                "fk_object_attribute": s.oa.to_string(),
                "ak_access_rights": [s.delete_ar.to_string()],
            }))
            .to_request(),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "create prohibition: {}",
        resp.status()
    );
    let proh: Value = test::read_body_json(resp).await;
    let proh_id: i64 = id_of(&proh);

    // 删除 association → lost_allow=[read]，users 含 audx-user
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!(
                "/api/admin/ngac/impact-preview?entity_type=association&id={}",
                assoc_id
            ))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "preview assoc: {}",
        resp.status()
    );
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["entity"]["entity_type"], "association");
    assert_eq!(body["truncated"], false);
    let affected = body["affected"].as_array().expect("affected");
    let entry = affected
        .iter()
        .find(|a| a["ua_id"] == json!(s.operator_ua.to_string()))
        .expect("operator affected");
    assert_eq!(entry["resource_type"], "audxmod");
    assert!(entry["lost_allow"]
        .as_array()
        .expect("lost_allow")
        .contains(&json!("read")));
    assert_eq!(entry["lost_deny"], json!(Vec::<String>::new()));
    assert!(entry["users"]
        .as_array()
        .expect("users")
        .iter()
        .any(|u| u["id"] == json!(s.audx_user.to_string())));

    // 删除 prohibition → lost_deny=[delete]
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!(
                "/api/admin/ngac/impact-preview?entity_type=prohibition&id={}",
                proh_id
            ))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "preview proh: {}",
        resp.status()
    );
    let body: Value = test::read_body_json(resp).await;
    let affected = body["affected"].as_array().expect("affected");
    let entry = affected
        .iter()
        .find(|a| a["ua_id"] == json!(s.operator_ua.to_string()))
        .expect("operator affected");
    assert!(entry["lost_deny"]
        .as_array()
        .expect("lost_deny")
        .contains(&json!("delete")));

    // 无影响：删除一个无关联 UA（audx-lonely）→ affected 空
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at) \
         VALUES ('audx-lonely', $1, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(s.pc)
    .execute(&pool)
    .await
    .ok();
    let lonely: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='audx-lonely' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("lonely UA");
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!(
                "/api/admin/ngac/impact-preview?entity_type=user_attribute&id={}",
                lonely
            ))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert!(resp.status().is_success());
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        body["affected"].as_array().expect("affected").len(),
        0,
        "no edges → no affected entries"
    );

    // 非法 entity_type → 400；不存在实体 → 404
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/admin/ngac/impact-preview?entity_type=bogus&id=1")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 400);
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/admin/ngac/impact-preview?entity_type=association&id=999999999999999999")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 404);

    cleanup(&pool).await;
}
