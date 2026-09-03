//! context-fields 端点测试（refactor-flow-node-context-fields）
//!
//! 编译期绑定（context_meta 静态索引）后本端点零 DB 依赖——纯 handler 契约测试：
//! - 在册三域叶表 → 200 + 字段数组（name/label/category/data_type），
//!   且不含系统/技术列（created_at、tk_batch_no、tpl_id 等）；t_color_ 例外——
//! - 非三域叶表 → 400；table 缺参/空串 → 400
//! - context_meta 索引本身：三域代表叶表在册、标签非空

use actix_web::{test, web, App};
use approval::context_meta::{context_fields_of, CONTEXT_FIELDS};
use approval::handlers;
use serde_json::Value;

async fn call(uri: &str) -> (u16, Value) {
    let app = test::init_service(
        App::new().service(web::scope("/test").configure(handlers::context_fields::register)),
    )
    .await;
    let resp = test::call_service(&app, test::TestRequest::get().uri(uri).to_request()).await;
    let status = resp.status().as_u16();
    (status, test::read_body_json(resp).await)
}

/// 取一个在册且字段非空的叶表作正例
fn sample_leaf() -> &'static str {
    CONTEXT_FIELDS
        .iter()
        .find(|(_, fields)| !fields.is_empty())
        .map(|(table, _)| *table)
        .expect("context_meta 应至少含一个字段非空的三域叶表")
}

#[tokio::test]
async fn context_fields_returns_business_fields_for_leaf() {
    let leaf = sample_leaf();
    let (status, body) = call(&format!("/test/approval-flows/context-fields?table={leaf}")).await;
    assert_eq!(status, 200, "context-fields 应 200");

    let fields = body["data"].as_array().expect("data 应为字段数组");
    assert!(!fields.is_empty(), "叶表 {leaf} 应识别出业务字段");
    for f in fields {
        assert!(f["name"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(f["label"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(f["category"].as_str().is_some());
        assert!(f["data_type"].as_str().is_some());
    }

    let names: Vec<&str> = fields.iter().filter_map(|f| f["name"].as_str()).collect();
    for tech in [
        "created_at",
        "updated_at",
        "tk_batch_no",
        "tpl_id",
        "projection",
    ] {
        assert!(
            !names.contains(&tech),
            "字段集 MUST NOT 含系统/技术列 {tech}，实际: {names:?}"
        );
    }
    // t_color_ 例外（spec t-color-color-badge-field）：入选时 MUST domain=color
    if names.contains(&"t_color_") {
        let f = fields
            .iter()
            .find(|f| f["name"] == "t_color_")
            .expect("t_color_ 在场");
        assert_eq!(f["domain"], "color", "t_color_ MUST domain=color");
    }
}

#[tokio::test]
async fn context_fields_rejects_non_context_table() {
    // zc_id_process 是流程表自身，不在三域叶表白名单内
    let (status, _) = call("/test/approval-flows/context-fields?table=zc_id_process").await;
    assert_eq!(status, 400, "非三域叶表 MUST 返回 400");
}

#[tokio::test]
async fn context_fields_rejects_empty_table() {
    let (status, _) = call("/test/approval-flows/context-fields?table=").await;
    assert_eq!(status, 400, "空 table MUST 返回 400");
}

#[tokio::test]
async fn context_meta_index_covers_three_domains() {
    // 三域代表：task / event（非审批）/ approve 各至少一叶在册
    assert!(
        context_fields_of("zc_id_task-commission").is_some(),
        "task 域叶表应在册"
    );
    assert!(
        CONTEXT_FIELDS
            .iter()
            .any(|(t, _)| t.starts_with("zc_id_even-")),
        "event 域叶表（zc_id_even-*）应在册"
    );
    assert!(
        CONTEXT_FIELDS
            .iter()
            .any(|(t, _)| t.starts_with("zc_id_appr-")),
        "approve 域叶表（zc_id_even-approve 子表 zc_id_appr-*）应在册"
    );
    // 白名单外
    assert!(context_fields_of("zc_id_process").is_none());
}
