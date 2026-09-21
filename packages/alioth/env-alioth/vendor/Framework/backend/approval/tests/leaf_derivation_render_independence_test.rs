//! 叶表名派生 MUST 与连接 `search_path` 无关（change: fix-leaf-derivation-render-dependence）
//!
//! 根因：`tableoid::regclass::text` 随会话渲染为 `zc_id_proc-approve` 或
//! `isahl."zc_id_proc-approve"` ⇒ `branch` / `context_leaf` 与白名单、前端字面量失配
//! （同 P6 缺陷族）。单连接池 + `SET search_path = public` 固化「无 isahl 前缀」会话状态。

use approval::models::ApprovalFlow;
use approval::repositories::ApprovalFlowRepository;
use common::data::ListQuery;
use crud::AliothRepository as _;

#[tokio::test]
async fn flow_branch_and_context_leaf_are_bare_under_public_search_path() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://isahl@localhost:5432/aliothstudio_test".to_string());
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect test db");
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("current_database");
    assert!(db.contains("_test"), "REFUSED: non-test db {db}");
    sqlx::query("SET search_path = public")
        .execute(&pool)
        .await
        .expect("set search_path=public");

    let repo = ApprovalFlowRepository::new(pool.clone());
    let page = repo
        .list(&ListQuery {
            page: 1,
            page_size: 50,
            filter_field: None,
            filter_op: None,
            filter_value: None,
            sort_field: None,
            sort_order: None,
        })
        .await
        .expect("list flows");

    let flows: Vec<&ApprovalFlow> = page.items.iter().filter(|f| f.branch.is_some()).collect();
    assert!(!flows.is_empty(), "测试库应有带 branch 的流程行");
    for f in flows {
        let branch = f.branch.as_deref().unwrap();
        assert!(
            !branch.contains("isahl") && !branch.contains('"'),
            "branch MUST 为裸叶表名（旧实现落 isahl.\"…\"）: {branch}"
        );
        assert!(branch.starts_with("zc_id_"), "branch 应为叶表名: {branch}");
    }

    // 单条口径：context_leaf 同族
    let one = repo
        .get(page.items[0].id)
        .await
        .expect("get flow")
        .expect("流程存在");
    assert!(
        !one.branch.as_deref().unwrap_or("").contains("isahl"),
        "get 的 branch 亦不得带 schema 限定"
    );
    if let Some(leaf) = one.context_leaf.as_deref() {
        assert!(
            !leaf.contains("isahl") && !leaf.contains('"'),
            "context_leaf MUST 为裸叶表名: {leaf}"
        );
    }
}
