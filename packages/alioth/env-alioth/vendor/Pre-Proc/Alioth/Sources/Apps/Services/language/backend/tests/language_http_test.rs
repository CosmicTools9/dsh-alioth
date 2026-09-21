//! alioth-service-language HTTP handler 集成测试
//!
//! 通过 actix-web test server 验证 list / create / get / patch / delete 路由
//! 及 settings JSONB 元数据（locale/enabled/coverage）的读写。

use actix_web::{test, web, App};
use common::testing::{connect_test_db, setup_test_schema_light};
use serde_json::json;

/// 注入请求身份（handlers 走 require_auth；Gateway 集成测试同先例：
/// extensions_mut + RequestContext，绕 JWT 中间件只测 handler 层）
fn authed<R: actix_web::HttpMessage>(req: R) -> R {
    req.extensions_mut()
        .insert(common::context::RequestContext::new(
            1,
            "test@local".to_string(),
        ));
    req
}

#[tokio::test]
async fn language_http_lifecycle() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    // 前轮失败残留 lang:% 行会污染 list 计数——前置清场（本测试族独占该 code 域）
    sqlx::query(r#"DELETE FROM isahl."zc_id_prot-env_config" WHERE code LIKE 'lang:%'"#)
        .execute(&pool)
        .await
        .expect("clean lang residue");

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .configure(alioth_service_language::register_service_routes),
    )
    .await;

    // Create
    let req = test::TestRequest::post()
        .uri("/service/language/languages")
        .set_json(json!({
            "name": "English (US)",
            "locale": "en-US",
            "enabled": true,
            "coverage": 0.85
        }))
        .to_request();
    let resp = test::call_service(&app, authed(req)).await;
    assert!(resp.status().is_success(), "create should succeed");
    let created: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(created["success"], true);
    assert_eq!(created["data"]["name"], "English (US)");
    assert_eq!(created["data"]["code"], "lang:en-US");
    // ID_JSON_PRECISION：id 以字符串下发（JS 精度安全）
    let id: String = created["data"]["id"]
        .as_str()
        .expect("created id (string)")
        .to_string();
    // ID_JSON_PRECISION：id 以字符串下发（JS 精度安全）——同时断言可无损解析为 i64
    assert!(id.parse::<i64>().is_ok(), "id MUST 可解析为 i64: {id}");

    // List
    let req = test::TestRequest::get()
        .uri("/service/language/languages")
        .to_request();
    let resp = test::call_service(&app, authed(req)).await;
    assert!(resp.status().is_success());
    let list: serde_json::Value = test::read_body_json(resp).await;
    let items = list["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["code"], "lang:en-US");

    // Get by id
    let req = test::TestRequest::get()
        .uri(&format!("/service/language/languages/{}", id))
        .to_request();
    let resp = test::call_service(&app, authed(req)).await;
    assert!(resp.status().is_success());
    let fetched: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(fetched["id"], id);
    assert_eq!(fetched["name"], "English (US)");
    assert_eq!(fetched["code"], "lang:en-US");
    assert_eq!(fetched["locale"], "en-US");
    assert_eq!(fetched["enabled"], true);
    assert_eq!(fetched["coverage"], 0.85);

    // Update coverage
    let req = test::TestRequest::patch()
        .uri(&format!("/service/language/languages/{}", id))
        .set_json(json!({ "coverage": 0.95 }))
        .to_request();
    let resp = test::call_service(&app, authed(req)).await;
    assert!(resp.status().is_success());
    let updated: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(updated["success"], true);
    assert_eq!(updated["data"]["coverage"], 0.95);
    assert_eq!(
        updated["data"]["locale"], "en-US",
        "PATCH should preserve locale"
    );
    assert_eq!(
        updated["data"]["enabled"], true,
        "PATCH should preserve enabled"
    );

    // Get after update
    let req = test::TestRequest::get()
        .uri(&format!("/service/language/languages/{}", id))
        .to_request();
    let resp = test::call_service(&app, authed(req)).await;
    let fetched: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(fetched["coverage"], 0.95);
    assert_eq!(fetched["locale"], "en-US", "PATCH should preserve locale");
    assert_eq!(fetched["enabled"], true, "PATCH should preserve enabled");

    // Delete
    let req = test::TestRequest::delete()
        .uri(&format!("/service/language/languages/{}", id))
        .to_request();
    let resp = test::call_service(&app, authed(req)).await;
    assert_eq!(resp.status(), 204);

    // Get after delete should be 404
    let req = test::TestRequest::get()
        .uri(&format!("/service/language/languages/{}", id))
        .to_request();
    let resp = test::call_service(&app, authed(req)).await;
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn language_http_create_then_list() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    // 前轮失败残留 lang:% 行会污染 list 计数——前置清场（本测试族独占该 code 域）
    sqlx::query(r#"DELETE FROM isahl."zc_id_prot-env_config" WHERE code LIKE 'lang:%'"#)
        .execute(&pool)
        .await
        .expect("clean lang residue");

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .configure(alioth_service_language::register_service_routes),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/service/language/languages")
        .set_json(json!({
            "name": "简体中文",
            "locale": "zh-CN",
            "enabled": true,
            "coverage": 1.0
        }))
        .to_request();
    let resp = test::call_service(&app, authed(req)).await;
    assert!(resp.status().is_success());

    let req = test::TestRequest::get()
        .uri("/service/language/languages")
        .to_request();
    let resp = test::call_service(&app, authed(req)).await;
    let list: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(list["total"], 1);
    assert_eq!(list["items"][0]["code"], "lang:zh-CN");
    assert_eq!(list["items"][0]["name"], "简体中文");
    assert_eq!(list["items"][0]["locale"], "zh-CN");
    assert_eq!(list["items"][0]["enabled"], true);
    assert_eq!(list["items"][0]["coverage"], 1.0);
}
