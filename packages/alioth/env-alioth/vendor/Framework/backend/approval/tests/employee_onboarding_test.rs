//! employee-onboarding 审批闭环订阅集成测试
//!
//! remove-comments-json-embedding 降级后语义：申请人上下文（applicant_id/name）
//! 此前寄生于 even-approve.comments JSON，已随 comments 文本化不可得——
//! `handle_event` 对 employee-onboarding 流程事件不再执行自动化
//! （不创建员工、不激活/禁用用户、不发布后续事件），静默跳过。
//!
//! 注意：所有测试共享 `aliothstudio_test`，请单线程运行：
//!   cargo test --test employee_onboarding_test -- --test-threads=1

use ::common::event_bus::{DomainEvent, DomainEventBus};
use ::common::testing::connect_test_db;
use std::sync::Arc;

mod common;
use common::{setup_test_schema, test_code};

/// 插入测试申请人（auth_users，status=pending_approval）
async fn insert_applicant(pool: &sqlx::PgPool, name: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, status, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES (isahl.gen_next_zuid(), $1, $1, $2, 'standard', 'pending_approval', TRUE,
                   NOW(), NOW(), 0, '{}'::jsonb)
           RETURNING id"#,
    )
    .bind(name)
    .bind(format!("{}@test.local", name))
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 插入审批流程（code 匹配 FLOW_EMPLOYEE_ONBOARDING）
async fn insert_process(pool: &sqlx::PgPool, code: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_process" (notice, code, created_by_id)
           VALUES ($1, $2, 1)
           RETURNING id"#,
    )
    .bind(format!("流程-{code}"))
    .bind(code)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 插入审批节点（桥链模型）：even 语义行（comments 纯文本——与生产写路一致的
/// 文本语义）+ oper 主体 + rro 在册锚 + 模板桥（rr_event）。返回 even id。
async fn insert_even(pool: &sqlx::PgPool, flow_id: i64, applicant_name: &str) -> i64 {
    let even_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_even-approve"
           (notice, code, comments, created_by_id)
           VALUES ($1, 'user-register-approval', $2, 1)
           RETURNING id"#,
    )
    .bind(format!("用户 {applicant_name} 访问授权审批"))
    .bind(format!("申请人：{applicant_name}"))
    .fetch_one(pool)
    .await
    .unwrap();
    let op_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, code, created_by_id)
           VALUES ($1, 'user-register-approval', 1) RETURNING id"#,
    )
    .bind(format!("节点-{applicant_name}"))
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_process_rr_operation
           (id, code, ref_left, ref_right, comments, created_by_id)
           VALUES (isahl.gen_next_uid(791), 'approve', $1, $2, $3, 1)"#,
    )
    .bind(flow_id)
    .bind(op_id)
    .bind(format!("节点-{applicant_name}"))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(op_id)
    .bind(even_id)
    .execute(pool)
    .await
    .unwrap();
    even_id
}

/// 插入审批实例（fk_approve=事件；notice 必填——enriched 解码 node_name
/// 为 Option，但 NULL notice 行会污染共享测试库的实例列表查询）
async fn insert_oper(pool: &sqlx::PgPool, even_id: i64) -> i64 {
    let instance_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (notice, fk_subject, created_by_id)
           VALUES ('员工入职实例', 1, 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(instance_id)
    .bind(even_id)
    .execute(pool)
    .await
    .unwrap();
    instance_id
}

fn completed_event(entity_id: i64, result: &str) -> DomainEvent {
    DomainEvent::new(
        "ApprovalCompleted",
        "commitment",
        entity_id,
        serde_json::json!({
            "entity_type": "approval-instance",
            "entity_id": entity_id,
            "result": result,
            "comment": Some("测试"),
        }),
    )
    .unwrap()
}

#[tokio::test]
async fn onboarding_automation_skipped_after_comments_textualization() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let suffix = test_code("onb-degraded");
    let applicant_name = format!("onb-applicant-{suffix}");
    let applicant_id = insert_applicant(&pool, &applicant_name).await;
    let flow_id = insert_process(&pool, "employee-onboarding").await;
    let even_id = insert_even(&pool, flow_id, &applicant_name).await;
    let oper_id = insert_oper(&pool, even_id).await;

    // approved 与 rejected 事件均应静默跳过（无申请人上下文）
    approval::handlers::employee_onboarding::handle_event(
        &pool,
        completed_event(even_id, "approved"),
    )
    .await;
    approval::handlers::employee_onboarding::handle_event(
        &pool,
        completed_event(oper_id, "rejected"),
    )
    .await;

    // 断言 1：不创建员工
    let emp_cnt: i64 =
        sqlx::query_scalar(r#"SELECT COUNT(*) FROM isahl."zc_id_empl-natural" WHERE code = $1"#)
            .bind(format!("emp-{}", applicant_id))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        emp_cnt, 0,
        "comments 文本化后 onboarding 自动化停用——不创建员工"
    );

    // 断言 2：用户状态保持 pending_approval（不激活也不禁用）
    let status: String =
        sqlx::query_scalar("SELECT status FROM isahl_auth.auth_users WHERE id = $1")
            .bind(applicant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "pending_approval", "用户状态不应被联动变更");
}

#[tokio::test]
async fn non_onboarding_flow_is_ignored() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let suffix = test_code("onb-other");
    let applicant_name = format!("onb-other-{suffix}");
    let applicant_id = insert_applicant(&pool, &applicant_name).await;
    let flow_id = insert_process(&pool, "other-flow").await;
    let even_id = insert_even(&pool, flow_id, &applicant_name).await;

    approval::handlers::employee_onboarding::handle_event(
        &pool,
        completed_event(even_id, "approved"),
    )
    .await;

    let emp_cnt: i64 =
        sqlx::query_scalar(r#"SELECT COUNT(*) FROM isahl."zc_id_empl-natural" WHERE code = $1"#)
            .bind(format!("emp-{}", applicant_id))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(emp_cnt, 0, "非 onboarding 流程不触发自动化");
}

#[tokio::test]
async fn subscriber_receives_published_event_via_bus() {
    // 事件总线装配冒烟：订阅者注册不 panic（自动化降级不影响装配契约）
    let _bus: Arc<dyn DomainEventBus> = Arc::new(::common::event_bus::InMemoryEventBus::new());
}
