//! scope-options 端点集成测试（close-approval-designer-gaps）
//!
//! 断言 GET /approval-flows/scope-options 结构契约：
//! - branches：zc_id_process 全部 7 个 proc 叶表（业务概念编译期绑定于 context_meta）
//! - domains：task / event / approve 三域，各域 items 非空且带 concept
//! - scopeId：字符串或 null（范畴定义种子缺失时为 null——不强制具体值）

mod common;

use ::common::testing::connect_test_db;
use actix_web::{test, web, App};
use approval::handlers;
use serde_json::Value;
use std::collections::HashSet;

use common::setup_test_schema;

async fn fetch_scope_options() -> Value {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .service(web::scope("/test").configure(handlers::scope_options::register)),
    )
    .await;
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/test/approval-flows/scope-options")
            .to_request(),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "scope-options 应 200，实际: {}",
        resp.status()
    );
    test::read_body_json(resp).await
}

#[tokio::test]
async fn scope_options_lists_all_proc_branches() {
    let body = fetch_scope_options().await;
    let data = &body["data"];
    let branches = data["branches"].as_array().expect("branches 数组");
    let tables: HashSet<&str> = branches
        .iter()
        .filter_map(|b| b["table"].as_str())
        .collect();
    let expected: HashSet<&str> = [
        "zc_id_proc-approve",
        "zc_id_proc-cicd",
        "zc_id_proc-loading",
        "zc_id_proc-make",
        "zc_id_proc-project",
        "zc_id_proc-purchase",
        "zc_id_proc-service",
    ]
    .into_iter()
    .collect();
    assert_eq!(tables, expected, "branches 必须覆盖全部 7 个 proc 叶表");
    // 业务概念解析（isahl_meta.meta_collections）
    let approve = branches
        .iter()
        .find(|b| b["table"] == "zc_id_proc-approve")
        .expect("proc-approve 分支存在");
    assert!(
        approve["concept"].as_str().is_some(),
        "branch 必须带业务概念，实际: {}",
        approve
    );
}

#[tokio::test]
async fn scope_options_has_three_domains_with_items() {
    let body = fetch_scope_options().await;
    let data = &body["data"];
    let domains = data["domains"].as_array().expect("domains 数组");
    let keys: HashSet<&str> = domains.iter().filter_map(|d| d["key"].as_str()).collect();
    assert_eq!(
        keys,
        ["task", "event", "approve"].into_iter().collect(),
        "必须恰好三域"
    );
    for d in domains {
        let items = d["items"].as_array().expect("items 数组");
        assert!(!items.is_empty(), "域 {} items 不得为空", d["key"]);
        for it in items {
            assert!(
                it["table"].as_str().is_some(),
                "item 必须带物理表名（值用）"
            );
            assert!(
                it["concept"].as_str().is_some(),
                "item 必须带业务概念（展示用），实际: {}",
                it
            );
            // scopeId：字符串（zuid）或 null——种子缺失不强制
            let sid = &it["scope_id"];
            assert!(
                sid.is_null() || sid.as_str().is_some(),
                "scope_id 必须为字符串或 null，实际: {}",
                sid
            );
        }
    }
}
