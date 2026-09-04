//! alioth-service-authority HTTP 集成测试
//!
//! `alioth-service-authority` 是薄委托层（lib.rs 仅注册路由），不持有自有 model/repo。
//! 本测试通过 `register_service_routes` 启动 in-process actix 服务，覆盖
//! `Framework/backend/authority` 在 `/service/authority` 前缀下挂载的 4 类资源：
//! employees / skill_tags / approval_roles / approvers，CRUD 主路径 + 异常路径 + list。
//!
//! ## 测试约定
//! - `#[tokio::test]` + `PgPool::connect`（BACKEND_FRAMEWORK.md §5：禁 `#[sqlx::test]`）
//! - 测试库为 `aliothstudio_test`，OS 用户 = `whoami`
//! - 通过 `wrap_fn` 注入 `i64` 扩展以满足 `require_auth`
//! - 串行执行（Pre-Proc/Alioth/.cargo/config.toml test-threads=1），避免互相污染
//!
//! ## 软删除
//! `Employee/SkillTag/ApprovalRole/Approver` 均 `SOFT_DELETE = true`，delete 后 get 返回 None。
//!
//! ## Schema 限制
//! `zc_id_tags-skill` 与 `zc_id_category` 表在测试库中**缺少** `dk_scene/dk_factor/dk_function`
//! 列，但 `repositories.rs` 的 INSERT 硬编码引用了这些列 → 500。
//! 受影响资源（skill_tag / approval_role）的 CRUD 主路径测试改用 list + 404
//! 验证路由 + handler 装配正确；employee / approver 表包含 dk_* 列，完整 lifecycle 可跑通。

use actix_web::{dev::Service, test as actix_test, web, App, HttpMessage};
use common::testing::{connect_test_db, setup_test_schema_light};
use serde_json::{json, Value};

const TEST_USER_ID: i64 = 1;
const PREFIX: &str = "/service/authority";

macro_rules! make_app {
    ($pool:expr) => {
        actix_test::init_service(
            App::new()
                .app_data(web::Data::new($pool.clone()))
                .wrap_fn(|req, srv| {
                    req.extensions_mut().insert(TEST_USER_ID);
                    srv.call(req)
                })
                .configure(alioth_service_authority::register_service_routes),
        )
        .await
    };
}

// ── Employee CRUD（完整 lifecycle：表含 dk_* 列） ───────────────

#[tokio::test]
async fn employee_crud_full_lifecycle() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let app = make_app!(pool);

    let create_body = json!({
        "name": "测试工程师-create",
        "code": "E2E-EMP-CRUD",
        "role": 1,
        "team": 1,
    });
    let req = actix_test::TestRequest::post()
        .uri(&format!("{PREFIX}/employees"))
        .set_json(&create_body)
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(
        resp.status(),
        201,
        "create employee must return 201 Created"
    );
    let created: Value = actix_test::read_body_json(resp).await;
    let emp_id = created["id"].as_i64().expect("id present");
    assert_eq!(created["name"], "测试工程师-create");
    assert_eq!(created["code"], "E2E-EMP-CRUD");
    assert!(emp_id > 0);

    let req = actix_test::TestRequest::get()
        .uri(&format!("{PREFIX}/employees/{emp_id}"))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let fetched: Value = actix_test::read_body_json(resp).await;
    assert_eq!(fetched["id"], emp_id);
    assert_eq!(fetched["name"], "测试工程师-create");

    let req = actix_test::TestRequest::patch()
        .uri(&format!("{PREFIX}/employees/{emp_id}"))
        .set_json(json!({ "name": "测试工程师-updated" }))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let updated: Value = actix_test::read_body_json(resp).await;
    assert_eq!(updated["name"], "测试工程师-updated");

    let req = actix_test::TestRequest::delete()
        .uri(&format!("{PREFIX}/employees/{emp_id}"))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 204);

    let req = actix_test::TestRequest::get()
        .uri(&format!("{PREFIX}/employees/{emp_id}"))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 404, "soft-deleted employee must return 404");
}

#[tokio::test]
async fn employee_get_returns_404_for_missing_id() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let app = make_app!(pool);

    let req = actix_test::TestRequest::get()
        .uri(&format!("{PREFIX}/employees/999999999"))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
    let body: Value = actix_test::read_body_json(resp).await;
    assert_eq!(body["error"], "not_found");
}

#[tokio::test]
async fn employee_create_with_empty_name_returns_error() {
    // service 层 AliothError::Validation 由 handler 映射为 500
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let app = make_app!(pool);

    let req = actix_test::TestRequest::post()
        .uri(&format!("{PREFIX}/employees"))
        .set_json(json!({ "name": "" }))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert!(
        resp.status().is_client_error() || resp.status().is_server_error(),
        "empty name must produce an error status, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn employee_list_returns_paginated_envelope() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let app = make_app!(pool);

    for n in ["列表-A", "列表-B"] {
        let req = actix_test::TestRequest::post()
            .uri(&format!("{PREFIX}/employees"))
            .set_json(json!({ "name": n }))
            .to_request();
        let resp = actix_test::call_service(&app, req).await;
        assert_eq!(resp.status(), 201);
    }

    let req = actix_test::TestRequest::get()
        .uri(&format!("{PREFIX}/employees?page=1&page_size=50"))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = actix_test::read_body_json(resp).await;
    assert!(body["items"].is_array(), "items must be array");
    assert!(body["total"].as_i64().unwrap() >= 2);
    assert_eq!(body["page"], 1);
}

// ── SkillTag：list + get-404 路由装配验证（create 路径受表结构限制） ──

#[tokio::test]
async fn skill_tag_list_returns_paginated_envelope() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let app = make_app!(pool);

    let req = actix_test::TestRequest::get()
        .uri(&format!("{PREFIX}/skill-tags?page=1&page_size=10"))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = actix_test::read_body_json(resp).await;
    assert!(body["items"].is_array());
    assert!(body["total"].is_i64());
}

#[tokio::test]
async fn skill_tag_get_returns_404_for_missing_id() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let app = make_app!(pool);

    let req = actix_test::TestRequest::get()
        .uri(&format!("{PREFIX}/skill-tags/999999999"))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── ApprovalRole：list + get-404 路由装配验证 ────────────────────

#[tokio::test]
async fn approval_role_list_returns_paginated_envelope() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let app = make_app!(pool);

    let req = actix_test::TestRequest::get()
        .uri(&format!("{PREFIX}/approval-roles?page=1&page_size=10"))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = actix_test::read_body_json(resp).await;
    assert!(body["items"].is_array());
    assert!(body["total"].is_i64());
}

#[tokio::test]
async fn approval_role_get_returns_404_for_missing_id() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let app = make_app!(pool);

    let req = actix_test::TestRequest::get()
        .uri(&format!("{PREFIX}/approval-roles/999999999"))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── Approver CRUD（完整 lifecycle：表含 dk_* 列） ───────────────

#[tokio::test]
async fn approver_crud_full_lifecycle() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let app = make_app!(pool);

    let req = actix_test::TestRequest::post()
        .uri(&format!("{PREFIX}/approvers"))
        .set_json(json!({
            "name": "测试CCB成员-create",
            "role": 1,
            "description": "E2E approver test",
        }))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 201);
    let created: Value = actix_test::read_body_json(resp).await;
    let approver_id = created["id"].as_i64().expect("id present");

    let req = actix_test::TestRequest::get()
        .uri(&format!("{PREFIX}/approvers/{approver_id}"))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);

    let req = actix_test::TestRequest::patch()
        .uri(&format!("{PREFIX}/approvers/{approver_id}"))
        .set_json(json!({ "name": "测试CCB成员-updated" }))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let updated: Value = actix_test::read_body_json(resp).await;
    assert_eq!(updated["name"], "测试CCB成员-updated");

    let req = actix_test::TestRequest::delete()
        .uri(&format!("{PREFIX}/approvers/{approver_id}"))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 204);

    let req = actix_test::TestRequest::get()
        .uri(&format!("{PREFIX}/approvers/{approver_id}"))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn approver_get_returns_404_for_missing_id() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let app = make_app!(pool);

    let req = actix_test::TestRequest::get()
        .uri(&format!("{PREFIX}/approvers/999999999"))
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── List 端点全覆盖（4 类资源 list 都能正常返回） ──────────────

#[tokio::test]
async fn list_all_four_resources_return_paginated_envelopes() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let app = make_app!(pool);

    for resource in ["employees", "skill-tags", "approval-roles", "approvers"] {
        let req = actix_test::TestRequest::get()
            .uri(&format!("{PREFIX}/{resource}?page=1&page_size=5"))
            .to_request();
        let resp = actix_test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "{resource} list must return 200, got {}",
            resp.status()
        );
        let body: Value = actix_test::read_body_json(resp).await;
        assert!(body["items"].is_array(), "{resource} items must be array");
        assert!(body["total"].is_i64(), "{resource} total must be i64");
    }
}
