//! NGAC 策略矩阵端点（GET /api/admin/ngac/matrix）集成测试
//!
//! 覆盖（change `refactor-ngac-policy-matrix` tasks 1.5）：
//!   1. 三态单元格：直接（direct）/ 继承有效（effective）/ 禁止（denied）
//!   2. 缺省 `policy_class` 参数 → 400
//!   3. 与 `/api/ngac/decide/explain` 结论一致（同源 `evaluate_pair` 语义）
//!   4. 实例级 OA 折叠：每组上限 200 截断 + `instance_count` 真实总数
//!   5. 缓存失效：`(policy_class, version)` 键，写入触发版本 bump 后自动失效
//!
//! 数据约定：测试数据统一 `mmx-` 前缀，测试前后双清理（崩溃残留防御），
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
    #[allow(dead_code)] // 供 seed 完整性；断言按需读取
    admin_ua: i64,
    mmx_admin_ua: i64,
    mmx_operator_ua: i64,
    mmx_oa: i64,
    pc: i64,
    #[allow(dead_code)] // 供 seed 完整性；断言按需读取
    read_ar: i64,
    write_ar: i64,
    email: String,
}

fn test_auth_state() -> AuthState {
    common::test_auth_state()
}

/// 幂等创建测试用户（mmx-*@test.local）。
async fn ensure_user(pool: &PgPool, email: &str) -> i64 {
    let username = email.split('@').next().unwrap_or("mmx").replace('-', "_");
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

/// 清理全部 mmx-* 测试数据（起点 + 终点各调用一次）。
async fn cleanup(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user IN (SELECT id FROM isahl_auth.auth_users WHERE email LIKE 'mmx-%@test.local')",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE email LIKE 'mmx-%@test.local'")
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_prohibition \
         WHERE fk_user_attribute IN (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'mmx-%')",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_association \
         WHERE fk_user_attribute IN (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'mmx-%')",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_association \
         WHERE fk_object_attribute IN (SELECT id FROM isahl_auth.ngac_object_attribute \
                                       WHERE resource_type IN ('mmxmod','mmxmod2','mmxinst'))",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_object_attribute \
         WHERE resource_type IN ('mmxmod','mmxmod2','mmxinst')",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'mmx-%'")
        .execute(pool)
        .await
        .ok();
}

/// 幂等 seed：admin 用户（绑定真实 `admin` UA，供 require_admin + PEP 决策）、
/// mmx 测试 UA 层级（mmx-admin 继承 mmx-operator）、集合级 OA（mmxmod:0）、
/// operator→OA 的 read association + delete prohibition。
async fn seed(pool: &PgPool) -> Seed {
    // default policy class
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

    // admin 用户（管理员面访问）
    let email = "mmx-admin@test.local".to_string();
    let admin_user = ensure_user(pool, &email).await;
    // 成员用户（operator UA 的 member_count = 2）
    let _member1 = ensure_user(pool, "mmx-member@test.local").await;
    let _member2 = ensure_user(pool, "mmx-member2@test.local").await;

    // 真实 admin UA（require_admin 与 PEP 决策依赖）
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

    // 测试 UA：mmx-operator（无父） + mmx-admin（继承 mmx-operator）
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
           VALUES ('mmx-operator', $1, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let mmx_operator_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='mmx-operator' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("mmx-operator UA");
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, ancestor_ids, created_at, updated_at)
           VALUES ('mmx-admin', $1, ARRAY[$2], NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(pc)
    .bind(mmx_operator_ua)
    .execute(pool)
    .await
    .ok();
    let mmx_admin_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='mmx-admin' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("mmx-admin UA");

    // 绑定：admin 用户 → mmx-admin；两个成员用户 → mmx-operator
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(admin_user)
    .bind(mmx_admin_ua)
    .execute(pool)
    .await
    .ok();
    for member_email in ["mmx-member@test.local", "mmx-member2@test.local"] {
        let member = ensure_user(pool, member_email).await;
        sqlx::query(
            r#"INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
               VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
        )
        .bind(member)
        .bind(mmx_operator_ua)
        .execute(pool)
        .await
        .ok();
    }

    // 集合级 OA：mmxmod:0
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, resource_identifier, created_at, updated_at)
           VALUES ('mmx-modules', $1, 'mmxmod', 0, 'mmx 模块集合', NOW(), NOW())
           ON CONFLICT (resource_type, fk_resource) DO UPDATE SET deleted_at = NULL"#,
    )
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let mmx_oa: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type='mmxmod' AND fk_resource=0 LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("mmx OA");

    // access rights（read/write/delete 为种子 AR，幂等补齐）
    let read_ar: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='read' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("read AR");
    let write_ar: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='write' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("write AR");

    // 直接边：mmx-operator → mmx-oa [read]
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_association (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at, updated_at)
           VALUES ($1, $2, ARRAY[$3], $4, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(mmx_operator_ua)
    .bind(mmx_oa)
    .bind(read_ar)
    .bind(pc)
    .execute(pool)
    .await
    .ok();

    // 禁止边：mmx-operator → mmx-oa [delete]
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_prohibition (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active, created_at, updated_at)
           VALUES ('mmx-no-delete', $1, $2, ARRAY[$3], TRUE, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(mmx_operator_ua)
    .bind(mmx_oa)
    .bind(sqlx::query_scalar::<_, i64>(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='delete' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("delete AR"))
    .execute(pool)
    .await
    .ok();

    Seed {
        admin_user,
        admin_ua,
        mmx_admin_ua,
        mmx_operator_ua,
        mmx_oa,
        pc,
        read_ar,
        write_ar,
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

/// 从矩阵响应中取 (ua_id, oa_id) 单元格。
fn find_cell(cells: &[Value], ua_id: i64, oa_id: i64) -> Option<&Value> {
    cells
        .iter()
        .find(|c| c["ua_id"] == json!(ua_id.to_string()) && c["oa_id"] == json!(oa_id.to_string()))
}

#[tokio::test]
async fn matrix_three_state_cells_and_shape() {
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

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/api/admin/ngac/matrix?policy_class={}", s.pc))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "matrix status: {}",
        resp.status()
    );
    let body: Value = test::read_body_json(resp).await;

    // policy_class 元信息 + version
    assert_eq!(body["policy_class"]["id"], json!(s.pc.to_string()));
    assert_eq!(body["policy_class"]["o_name"], "default");
    assert!(body["version"].as_i64().is_some(), "version is a number");

    // UA 行：ancestor_ids（直接父边）+ member_count（未删绑定数）
    let uas = body["user_attributes"].as_array().expect("user_attributes");
    let mmx_admin_row = uas
        .iter()
        .find(|u| u["o_name"] == "mmx-admin")
        .expect("mmx-admin row");
    assert_eq!(mmx_admin_row["id"], json!(s.mmx_admin_ua.to_string()));
    assert_eq!(
        mmx_admin_row["fk_policy_class"],
        json!(s.pc.to_string()),
        "fk_policy_class preserved"
    );
    assert_eq!(
        mmx_admin_row["ancestor_ids"],
        json!(vec![s.mmx_operator_ua.to_string()]),
        "direct parent edge"
    );
    assert_eq!(mmx_admin_row["member_count"], 1);
    let mmx_operator_row = uas
        .iter()
        .find(|u| u["o_name"] == "mmx-operator")
        .expect("mmx-operator row");
    assert_eq!(
        mmx_operator_row["ancestor_ids"],
        json!(Vec::<String>::new())
    );
    assert_eq!(
        mmx_operator_row["member_count"], 2,
        "two member users bound"
    );

    // OA 分组：mmxmod 组首列为集合级 OA（fk_resource=0），无实例
    let groups = body["object_groups"].as_array().expect("object_groups");
    let g = groups
        .iter()
        .find(|g| g["resource_type"] == "mmxmod")
        .expect("mmxmod group");
    assert!(g["collection_oa"].is_object(), "collection OA present");
    assert_eq!(g["collection_oa"]["id"], json!(s.mmx_oa.to_string()));
    assert_eq!(g["collection_oa"]["fk_resource"], json!("0"));
    assert_eq!(
        g["collection_oa"]["resource_identifier"],
        json!("mmx 模块集合"),
        "resource_identifier exposed by matrix endpoint"
    );
    assert_eq!(g["instance_count"], 0);
    assert_eq!(g["instances"].as_array().expect("instances").len(), 0);

    // 单元格三态：
    //   (mmx-admin, oa)：无直接边，经继承 effective=read、经禁止 denied=delete
    //   (mmx-operator, oa)：直接 read + effective read + denied delete
    let cells = body["cells"].as_array().expect("cells");
    let cell_admin = find_cell(cells, s.mmx_admin_ua, s.mmx_oa).expect("cell (mmx-admin, oa)");
    assert_eq!(cell_admin["direct"], json!(Vec::<String>::new()));
    assert!(
        cell_admin["effective"]
            .as_array()
            .expect("effective")
            .contains(&json!("read")),
        "inherited read visible as effective"
    );
    assert!(
        cell_admin["denied"]
            .as_array()
            .expect("denied")
            .contains(&json!("delete")),
        "prohibition on ancestor pair visible as denied"
    );

    let cell_operator =
        find_cell(cells, s.mmx_operator_ua, s.mmx_oa).expect("cell (mmx-operator, oa)");
    assert_eq!(cell_operator["direct"], json!(vec!["read"]));
    assert!(cell_operator["effective"]
        .as_array()
        .expect("effective")
        .contains(&json!("read")));
    assert!(cell_operator["denied"]
        .as_array()
        .expect("denied")
        .contains(&json!("delete")));

    // access rights 词表含 read/write/delete
    let ars = body["access_rights"].as_array().expect("access_rights");
    let names: Vec<&str> = ars.iter().filter_map(|a| a["o_name"].as_str()).collect();
    for required in ["read", "write", "delete"] {
        assert!(
            names.contains(&required),
            "access right {} present",
            required
        );
    }

    cleanup(&pool).await;
}

#[tokio::test]
async fn matrix_missing_policy_class_returns_400() {
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
            .uri("/api/admin/ngac/matrix")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 400, "missing policy_class must 400");

    cleanup(&pool).await;
}

#[tokio::test]
async fn matrix_consistent_with_explain() {
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

    // 同一 (user, resource, action)：矩阵 effective/denied 与 explain 结论一致
    for (action, expected_outcome, expect_effective, expect_denied) in [
        ("read", "permit", true, false),
        ("delete", "deny", false, true),
    ] {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/ngac/decide/explain")
                .insert_header(("Authorization", format!("Bearer {}", token)))
                .set_json(json!({
                    "user_id": s.admin_user,
                    "resource": "mmxmod:0",
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
            "explain outcome for {}",
            action
        );

        let resp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!("/api/admin/ngac/matrix?policy_class={}", s.pc))
                .insert_header(("Authorization", format!("Bearer {}", token)))
                .to_request(),
        )
        .await;
        assert!(resp.status().is_success());
        let matrix: Value = test::read_body_json(resp).await;
        let cells = matrix["cells"].as_array().expect("cells");
        let cell = find_cell(cells, s.mmx_admin_ua, s.mmx_oa).expect("cell (mmx-admin, mmx-oa)");
        let effective = cell["effective"].as_array().expect("effective");
        let denied = cell["denied"].as_array().expect("denied");
        assert_eq!(
            effective.contains(&json!(action)),
            expect_effective,
            "matrix effective for {}",
            action
        );
        assert_eq!(
            denied.contains(&json!(action)),
            expect_denied,
            "matrix denied for {}",
            action
        );
    }

    cleanup(&pool).await;
}

#[tokio::test]
async fn matrix_instance_oa_folding_caps_at_200() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    cleanup(&pool).await;
    let s = seed(&pool).await;
    // 201 个实例级 OA（resource_type='mmxinst'，fk_resource=1..201）
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, ancestor_ids, created_at, updated_at)
           SELECT 'mmx-inst-' || g, $1, 'mmxinst', g, '{}'::bigint[], NOW(), NOW()
           FROM generate_series(1, 201) AS g
           ON CONFLICT (resource_type, fk_resource) DO NOTHING"#,
    )
    .bind(s.pc)
    .execute(&pool)
    .await
    .expect("seed 201 instance OAs");

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
            .uri(&format!("/api/admin/ngac/matrix?policy_class={}", s.pc))
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "matrix status: {}",
        resp.status()
    );
    let body: Value = test::read_body_json(resp).await;
    let groups = body["object_groups"].as_array().expect("object_groups");
    let g = groups
        .iter()
        .find(|g| g["resource_type"] == "mmxinst")
        .expect("mmxinst group");
    assert!(
        g["collection_oa"].is_null(),
        "no collection OA in instance group"
    );
    assert_eq!(g["instance_count"], 201, "true total reported");
    assert_eq!(
        g["instances"].as_array().expect("instances").len(),
        200,
        "instances truncated to 200"
    );

    cleanup(&pool).await;
}

#[tokio::test]
async fn matrix_cache_invalidated_on_policy_version_bump() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    cleanup(&pool).await;
    let s = seed(&pool).await;
    // 第二组 OA（mmxmod2:0），初始无关联
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at)
           VALUES ('mmx-modules-2', $1, 'mmxmod2', 0, NOW(), NOW())
           ON CONFLICT (resource_type, fk_resource) DO UPDATE SET deleted_at = NULL"#,
    )
    .bind(s.pc)
    .execute(&pool)
    .await
    .ok();
    let oa2: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type='mmxmod2' AND fk_resource=0 LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("mmx OA2");

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(test_auth_state()))
            .configure(gateway_sso::admin::configure),
    )
    .await;
    let token = admin_token(&pool, s.admin_user, &s.email).await;
    let uri = format!("/api/admin/ngac/matrix?policy_class={}", s.pc);

    // 第一次请求：无 (operator, oa2) 单元格
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&uri)
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert!(resp.status().is_success());
    let body1: Value = test::read_body_json(resp).await;
    let cells1 = body1["cells"].as_array().expect("cells");
    assert!(
        find_cell(cells1, s.mmx_operator_ua, oa2).is_none(),
        "no cell before association"
    );

    // 新增 association：operator → oa2 [write]（030 触发器自动 bump 版本）
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_association (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at, updated_at)
           VALUES ($1, $2, ARRAY[$3], $4, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(s.mmx_operator_ua)
    .bind(oa2)
    .bind(s.write_ar)
    .bind(s.pc)
    .execute(&pool)
    .await
    .expect("seed write association");

    // 第二次请求：版本键变化 → 缓存失效 → 新单元格可见
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&uri)
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert!(resp.status().is_success());
    let body2: Value = test::read_body_json(resp).await;
    let cells2 = body2["cells"].as_array().expect("cells");
    let cell = find_cell(cells2, s.mmx_operator_ua, oa2)
        .expect("cell after association (cache must invalidate on version bump)");
    assert_eq!(cell["direct"], json!(vec!["write"]));
    assert!(cell["effective"]
        .as_array()
        .expect("effective")
        .contains(&json!("write")));

    cleanup(&pool).await;
}
