//! End-to-end integration test for the ontology handler.

use ::common::testing::connect_test_db;
use actix_web::dev::Service;
use actix_web::{test, web, App, HttpMessage};
use crud::ontology_handler::ontology_routes;
use sqlx::AssertSqlSafe;

const TEST_BIZ_LEAF: &str = "zc_id_agre-pricing";

fn lookup_fn(table: &str) -> (Option<i64>, Option<i64>, Option<i64>) {
    let h = table.len() as i64;
    (
        Some(1_000_000_000_000_000 + h),
        Some(2_000_000_000_000_000 + h),
        Some(3_000_000_000_000_000 + h),
    )
}

#[actix_web::test]
async fn full_lifecycle_create_list_get_delete() {
    let pool = connect_test_db().await;
    sqlx::query(AssertSqlSafe(format!(
        r#"DELETE FROM isahl."{}" WHERE notice = $1"#,
        TEST_BIZ_LEAF
    )))
    .bind("__test_ontology_handler__")
    .execute(&pool)
    .await
    .unwrap();

    let ctx = common::context::RequestContext::with_username(1, "test@test", "tester");
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(|cfg: &mut web::ServiceConfig| {
                    ontology_routes(cfg, "/ontology", lookup_fn);
                }),
        ),
    )
    .await;

    // CREATE
    let body = serde_json::json!({
        "notice": "__test_ontology_handler__",
        "code": "OH-001",
        "public": true,
    });
    let req = test::TestRequest::post()
        .uri(&format!("/test/ontology/leaf/{}", TEST_BIZ_LEAF))
        .set_json(&body)
        .to_request();
    let resp: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    let new_id = match resp["data"]["id"].as_i64() {
        Some(v) => v,
        None => panic!("create failed: {}", resp),
    };
    assert!(new_id > 0);

    // GET
    let req = test::TestRequest::get()
        .uri(&format!("/test/ontology/leaf/{}/{}", TEST_BIZ_LEAF, new_id))
        .to_request();
    let resp: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(resp["data"]["code"].as_str(), Some("OH-001"));

    // Verify binding was written
    let row: (Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(AssertSqlSafe(format!(
        r#"SELECT dk_scene, dk_factor, dk_function FROM isahl."{}" WHERE id = $1"#,
        TEST_BIZ_LEAF
    )))
    .bind(new_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let h = TEST_BIZ_LEAF.len() as i64;
    assert_eq!(row.0, Some(1_000_000_000_000_000 + h));
    assert_eq!(row.1, Some(2_000_000_000_000_000 + h));
    assert_eq!(row.2, Some(3_000_000_000_000_000 + h));

    // LIST
    let req = test::TestRequest::get()
        .uri(&format!(
            "/test/ontology/leaf/{}?page=1&page_size=5",
            TEST_BIZ_LEAF
        ))
        .to_request();
    let resp: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    let data = resp["data"]["data"].as_array().unwrap();
    assert!(!data.is_empty());

    // DELETE
    let req = test::TestRequest::delete()
        .uri(&format!("/test/ontology/leaf/{}/{}", TEST_BIZ_LEAF, new_id))
        .to_request();
    let resp: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(resp["data"]["deleted"], serde_json::Value::Bool(true));

    // Verify soft delete
    let row: (Option<chrono::DateTime<chrono::Utc>>,) = sqlx::query_as(AssertSqlSafe(format!(
        r#"SELECT deleted_at FROM isahl."{}" WHERE id = $1"#,
        TEST_BIZ_LEAF
    )))
    .bind(new_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(row.0.is_some(), "deleted_at should be set");
}

#[actix_web::test]
async fn reference_routes_coexist_with_leaf_in_same_scope() {
    use actix_web::dev::Service;
    use crud::ontology_handler::ontology_routes_with_reference;
    use std::collections::HashSet;
    use std::sync::Arc;
    let pool = connect_test_db().await;
    let ctx = common::context::RequestContext::with_username(1, "test@test", "tester");
    let allowlist: Arc<HashSet<String>> =
        Arc::new(["zc_id_leve-health".to_string()].into_iter().collect());
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(|cfg: &mut web::ServiceConfig| {
                    ontology_routes_with_reference(cfg, "/ontology", lookup_fn, allowlist.clone());
                }),
        ),
    )
    .await;

    // reference 路由（非叶 FK 目标 zc_id_process，fk_index 覆盖）必须 success
    let req = test::TestRequest::get()
        .uri("/test/ontology/reference/zc_id_process")
        .to_request();
    let resp = app.call(req).await.unwrap();
    assert!(
        resp.status().is_success(),
        "fk-covered reference should succeed (got {})",
        resp.status()
    );

    // allowlist 命中的独立叶表必须 success（确定断言）
    let req = test::TestRequest::get()
        .uri("/test/ontology/reference/zc_id_leve-health")
        .to_request();
    let resp = app.call(req).await.unwrap();
    assert!(
        resp.status().is_success(),
        "allowlisted reference should succeed (got {})",
        resp.status()
    );

    // 未授权表必须严格 400（路由存在但授权拒绝）
    let req = test::TestRequest::get()
        .uri("/test/ontology/reference/not_a_managed_table_xyz")
        .to_request();
    let resp = app.call(req).await.unwrap();
    assert_eq!(resp.status(), actix_web::http::StatusCode::BAD_REQUEST);

    // leaf 路由仍应工作
    let req = test::TestRequest::get()
        .uri(&format!("/test/ontology/leaf/{}", TEST_BIZ_LEAF))
        .to_request();
    let resp = app.call(req).await.unwrap();
    assert!(
        resp.status().is_success() || resp.status() == actix_web::http::StatusCode::BAD_REQUEST,
        "leaf route should respond (got {})",
        resp.status()
    );
}

/// 回归（fix-avic-reference-field-401 语义修正）：leaf 读取不要求 leaf 表判定。
/// 非叶表 GET list 应成功（leaf 端点可读任意业务表），写路径仍拒绝，非法表拒绝。
#[actix_web::test]
async fn leaf_read_non_leaf_get_succeeds() {
    let pool = connect_test_db().await;
    let ctx = common::context::RequestContext::with_username(1, "test@test", "tester");
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(|cfg: &mut web::ServiceConfig| {
                    ontology_routes(cfg, "/ontology", lookup_fn);
                }),
        ),
    )
    .await;

    // ① 非叶表（zc_id_subjects）GET list 必须成功——读取不要求 leaf 判定
    let req = test::TestRequest::get()
        .uri("/test/ontology/leaf/zc_id_subjects")
        .to_request();
    let resp = app.call(req).await.unwrap();
    assert!(
        resp.status().is_success(),
        "non-leaf GET should succeed after removing read-side leaf check (got {})",
        resp.status()
    );

    // ② 非叶表 POST（insert）仍必须 400——leaf 判定仅约束写路径
    let req = test::TestRequest::post()
        .uri("/test/ontology/leaf/zc_id_subjects")
        .set_json(serde_json::json!({ "notice": "__test_non_leaf_post__" }))
        .to_request();
    let resp = app.call(req).await.unwrap();
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::BAD_REQUEST,
        "non-leaf POST must still be rejected by insert-side leaf check"
    );

    // ③ 非法/不存在表 GET 必须 400——表存在性校验兜底（防 SQL 标识符注入）
    let req = test::TestRequest::get()
        .uri("/test/ontology/leaf/zc_id_nonexistent_leaf")
        .to_request();
    let resp = app.call(req).await.unwrap();
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::BAD_REQUEST,
        "nonexistent table GET must be rejected by existence check"
    );
}
