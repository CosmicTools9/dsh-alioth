//! NGAC 访问审查端点（GET /api/admin/ngac/review/user/{id} 与
//! GET /api/admin/ngac/review/resource）集成测试
//!
//! 覆盖（change `add-ngac-access-review` tasks 1.5）：
//!   1. 用户视图三态：直接指派链 + allowed（含继承）/ denied（prohibition）
//!   2. 404（用户不存在）/ 400（缺 resource_type）
//!   3. 与 `/api/ngac/decide/explain` 逐 action 结论一致（同源 evaluate_pair）
//!   4. 资源视图 holders 稀疏性 + 成员用户解析
//!
//! 数据约定：测试数据统一 `rvx-` 前缀，测试前后双清理（崩溃残留防御），
//! 不残留任何行（与 tests/common 的"测试数据绝不残留"原则一致）。

mod common;

use ::common::testing::connect_test_db;
use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use serde_json::{json, Value};
use sqlx::PgPool;

struct Seed {
    admin_user: i64,
    rvx_user: i64,
    rvx_member: i64,
    rvx_admin_ua: i64,
    rvx_operator_ua: i64,
    rvx_oa: i64,
    pc: i64,
    email: String,
}

fn test_auth_state() -> AuthState {
    common::test_auth_state()
}

/// 幂等创建测试用户（rvx-*@test.local）。
async fn ensure_user(pool: &PgPool, email: &str) -> i64 {
    let username = email.split('@').next().unwrap_or("rvx").replace('-', "_");
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

/// 清理全部 rvx-* 测试数据（起点 + 终点各调用一次）。
async fn cleanup(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user IN (SELECT id FROM isahl_auth.auth_users WHERE email LIKE 'rvx-%@test.local')",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE email LIKE 'rvx-%@test.local'")
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_prohibition \
         WHERE fk_user_attribute IN (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'rvx-%')",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_association \
         WHERE fk_user_attribute IN (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'rvx-%')",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type = 'rvxmod'")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'rvx-%'")
        .execute(pool)
        .await
        .ok();
}

/// 幂等 seed：admin 用户（绑定真实 `admin` UA，供 require_admin + PEP 决策）、
/// rvx 测试 UA 层级（rvx-admin 继承 rvx-operator）、集合级 OA（rvxmod:0）、
/// rvx-operator→OA 的 read association + delete prohibition；
/// rvx-user → rvx-admin（继承授权），rvx-member → rvx-operator（直接授权）。
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

    let email = "rvx-admin@test.local".to_string();
    let admin_user = ensure_user(pool, &email).await;
    let rvx_user = ensure_user(pool, "rvx-user@test.local").await;
    let rvx_member = ensure_user(pool, "rvx-member@test.local").await;

    // 真实 admin UA（require_admin 依赖）
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

    // 测试 UA：rvx-operator（无父）+ rvx-admin（继承 rvx-operator）
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
           VALUES ('rvx-operator', $1, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let rvx_operator_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='rvx-operator' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("rvx-operator UA");
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, ancestor_ids, created_at, updated_at)
           VALUES ('rvx-admin', $1, ARRAY[$2], NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(pc)
    .bind(rvx_operator_ua)
    .execute(pool)
    .await
    .ok();
    let rvx_admin_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='rvx-admin' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("rvx-admin UA");

    // 绑定：rvx-user → rvx-admin；rvx-member → rvx-operator
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(rvx_user)
    .bind(rvx_admin_ua)
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(rvx_member)
    .bind(rvx_operator_ua)
    .execute(pool)
    .await
    .ok();

    // 集合级 OA：rvxmod:0
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, resource_identifier, created_at, updated_at)
           VALUES ('rvx-modules', $1, 'rvxmod', 0, 'rvx 模块集合', NOW(), NOW())
           ON CONFLICT (resource_type, fk_resource) DO UPDATE SET deleted_at = NULL"#,
    )
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let rvx_oa: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type='rvxmod' AND fk_resource=0 LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("rvx OA");

    // 直接边：rvx-operator → OA [read]
    let read_ar: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='read' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("read AR");
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_association (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at, updated_at)
           VALUES ($1, $2, ARRAY[$3], $4, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(rvx_operator_ua)
    .bind(rvx_oa)
    .bind(read_ar)
    .bind(pc)
    .execute(pool)
    .await
    .ok();

    // 禁止边：rvx-operator → OA [delete]
    let delete_ar: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='delete' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("delete AR");
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_prohibition (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active, created_at, updated_at)
           VALUES ('rvx-no-delete', $1, $2, ARRAY[$3], TRUE, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(rvx_operator_ua)
    .bind(rvx_oa)
    .bind(delete_ar)
    .execute(pool)
    .await
    .ok();

    Seed {
        admin_user,
        rvx_user,
        rvx_member,
        rvx_admin_ua,
        rvx_operator_ua,
        rvx_oa,
        pc,
        email,
    }
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

#[tokio::test]
async fn review_user_three_state_and_assignments() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
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

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/api/admin/ngac/review/user/{}", s.rvx_user))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "review status: {}",
        resp.status()
    );
    let body: Value = test::read_body_json(resp).await;

    // user 元信息（ZUID 字符串序列化）
    assert_eq!(body["user"]["id"], json!(s.rvx_user.to_string()));
    assert_eq!(body["user"]["email"], json!("rvx-user@test.local"));
    assert!(body["version"].as_i64().is_some());

    // assignments：直接指派 rvx-admin，链含自身 + 祖先 rvx-operator
    let assignments = body["assignments"].as_array().expect("assignments");
    let a = assignments
        .iter()
        .find(|a| a["ua_id"] == json!(s.rvx_admin_ua.to_string()))
        .expect("rvx-admin assignment");
    assert_eq!(a["o_name"], "rvx-admin");
    let chain = a["ancestor_chain"].as_array().expect("ancestor_chain");
    assert!(chain.contains(&json!("rvx-admin")));
    assert!(chain.contains(&json!("rvx-operator")));
    // rvx-operator 非直接指派，不出现在 assignments
    assert!(
        !assignments
            .iter()
            .any(|a| a["ua_id"] == json!(s.rvx_operator_ua.to_string())),
        "inherited UA must not appear in assignments"
    );

    // permissions：rvxmod 行 allowed 含 read（经 rvx-admin→rvx-operator 继承）、
    // denied 含 delete（prohibition 同样经继承命中）
    let perms = body["permissions"].as_array().expect("permissions");
    let row = perms
        .iter()
        .find(|p| p["resource_type"] == "rvxmod")
        .expect("rvxmod permission row");
    assert_eq!(row["fk_policy_class"], json!(s.pc.to_string()));
    assert!(
        row["allowed"]
            .as_array()
            .expect("allowed")
            .contains(&json!("read")),
        "inherited read visible as allowed"
    );
    assert!(
        row["denied"]
            .as_array()
            .expect("denied")
            .contains(&json!("delete")),
        "prohibition visible as denied"
    );

    cleanup(&pool).await;
}

#[tokio::test]
async fn review_user_not_found_returns_404() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
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

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/admin/ngac/review/user/999999999999999999")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 404, "unknown user must 404");

    cleanup(&pool).await;
}

#[tokio::test]
async fn review_resource_missing_type_returns_400() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
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

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/admin/ngac/review/resource")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 400, "missing resource_type must 400");

    cleanup(&pool).await;
}

#[tokio::test]
async fn review_user_consistent_with_explain() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    cleanup(&pool).await;
    let s = seed(&pool).await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(test_auth_state()))
            .configure(gateway_sso::admin::configure)
            .route(
                "/api/ngac/decide/explain",
                web::post().to(gateway_sso::ngac::pdp::ngac_decide_explain),
            ),
    )
    .await;
    let token = admin_token(&pool, s.admin_user, &s.email).await;

    // 拉一次用户视图，取 rvxmod 行的 allowed/denied
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/api/admin/ngac/review/user/{}", s.rvx_user))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert!(resp.status().is_success());
    let review: Value = test::read_body_json(resp).await;
    let perms = review["permissions"].as_array().expect("permissions");
    let row = perms
        .iter()
        .find(|p| p["resource_type"] == "rvxmod")
        .expect("rvxmod permission row");

    // 逐 action 与 explain 对照：allowed→permit、denied→deny
    for (action, expected_outcome) in [("read", "permit"), ("delete", "deny")] {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/ngac/decide/explain")
                .insert_header(("Authorization", format!("Bearer {}", token)))
                .set_json(json!({
                    "user_id": s.rvx_user,
                    "resource": "rvxmod:0",
                    "action": action,
                }))
                .to_request(),
        )
        .await;
        assert!(
            resp.status().is_success(),
            "explain {}: {}",
            action,
            resp.status()
        );
        let explain: Value = test::read_body_json(resp).await;
        assert_eq!(
            explain["outcome"], expected_outcome,
            "explain outcome {}",
            action
        );

        let allowed = row["allowed"].as_array().expect("allowed");
        let denied = row["denied"].as_array().expect("denied");
        assert_eq!(
            allowed.contains(&json!(action)),
            expected_outcome == "permit",
            "review allowed for {}",
            action
        );
        assert_eq!(
            denied.contains(&json!(action)),
            expected_outcome == "deny",
            "review denied for {}",
            action
        );
    }

    cleanup(&pool).await;
}

#[tokio::test]
async fn review_resource_holders_sparse_with_users() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
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

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/admin/ngac/review/resource?resource_type=rvxmod")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "review status: {}",
        resp.status()
    );
    let body: Value = test::read_body_json(resp).await;

    // resource 元信息：fk_resource 缺省 0、oa_ids 指向集合级 OA、resource_identifier 带出
    assert_eq!(body["resource"]["resource_type"], "rvxmod");
    assert_eq!(body["resource"]["fk_resource"], json!("0"));
    assert_eq!(
        body["resource"]["resource_identifier"],
        json!("rvx 模块集合"),
        "resource_identifier exposed by review/resource endpoint"
    );
    assert_eq!(
        body["resource"]["oa_ids"],
        json!(vec![s.rvx_oa.to_string()])
    );
    assert!(body["version"].as_i64().is_some());

    let holders = body["holders"].as_array().expect("holders");

    // rvx-operator 持有：allowed=read、denied=delete，成员含 rvx-member
    let op = holders
        .iter()
        .find(|h| h["ua_id"] == json!(s.rvx_operator_ua.to_string()))
        .expect("rvx-operator holder");
    assert!(op["allowed"]
        .as_array()
        .expect("allowed")
        .contains(&json!("read")));
    assert!(op["denied"]
        .as_array()
        .expect("denied")
        .contains(&json!("delete")));
    let users = op["users"].as_array().expect("users");
    assert!(
        users
            .iter()
            .any(|u| u["id"] == json!(s.rvx_member.to_string())),
        "rvx-member resolved as holder user"
    );

    // rvx-admin 经继承同样持有（稀疏：两 UA 均非空，均在 holders）
    let adm = holders
        .iter()
        .find(|h| h["ua_id"] == json!(s.rvx_admin_ua.to_string()))
        .expect("rvx-admin holder (inherited)");
    assert!(adm["allowed"]
        .as_array()
        .expect("allowed")
        .contains(&json!("read")));
    let adm_users = adm["users"].as_array().expect("users");
    assert!(
        adm_users
            .iter()
            .any(|u| u["id"] == json!(s.rvx_user.to_string())),
        "rvx-user resolved via rvx-admin"
    );

    // 稀疏性：真实 admin UA 对 rvxmod 无任何授权，不得出现在 holders
    assert!(
        !holders.iter().any(|h| h["o_name"] == "admin"),
        "sparse: UA without any rights must be absent"
    );

    cleanup(&pool).await;
}
