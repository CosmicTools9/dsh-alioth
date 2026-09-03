//! 节点类型定义补全回归测试（2026-09-02 fix-flow-node-type-definitions）：
//! - review + 签署模式：运行时按 human 建评审实例（不退化 gate）
//! - end outcome=rejected：到达终局不物化结论实例（complete 物化）
//! - parallel：advance 忽略 cond 全边扇出（两个分支审批实例并发创建）

use ::common::event_bus::{DomainEventBus, InMemoryEventBus};
use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
mod common;
use common::{ensure_role_member, grant_user_access, setup_test_schema};

const USER_ID: i64 = 424701;

macro_rules! build_app {
    ($pool:expr, $bus:expr) => {{
        let ctx = ::common::context::RequestContext::with_username(
            USER_ID,
            "ndef@test.local",
            "ndef-test",
        );
        test::init_service(
            App::new()
                .app_data(web::Data::new($pool.clone()))
                .app_data(web::Data::new($bus))
                .service(
                    web::scope("/test")
                        .wrap_fn(move |req, srv| {
                            req.extensions_mut().insert(ctx.clone());
                            srv.call(req)
                        })
                        .configure(handlers::publish::register)
                        .configure(handlers::initiate::register)
                        .configure(handlers::approve_reject::register),
                ),
        )
        .await
    }};
}

macro_rules! post_json {
    ($app:expr, $uri:expr, $body:expr) => {{
        let resp = test::call_service(
            $app,
            test::TestRequest::post()
                .uri($uri)
                .set_json($body)
                .to_request(),
        )
        .await;
        let status: u16 = resp.status().as_u16();
        let body: Value = test::read_body_json(resp).await;
        (status, body)
    }};
}

async fn seed_user_and_role(pool: &PgPool) {
    // 预建 UA（ensure_role_member 对不存在 UA 插入缺 id——helper 预存缺陷，仅成员插入可用）
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_attribute (id, o_name, fk_policy_class)
           SELECT isahl.gen_next_zuid(), $1, pc.id FROM isahl_auth.ngac_policy_class pc
           WHERE pc.o_name = 'default'
             AND NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_user_attribute ua
                             WHERE ua.o_name = $1)
           LIMIT 1"#,
    )
    .bind("ndef-role")
    .execute(pool)
    .await
    .unwrap();
    ensure_role_member(pool, "ndef-role", USER_ID)
        .await
        .unwrap();
    sqlx::query(r#"DELETE FROM isahl."zc_id_subj-position" WHERE notice = 'ndef-pos'"#)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-position"
           (id, code, notice, fk_user, created_by_id, created_at, updated_at)
           VALUES (isahl.gen_next_zuid(), 'ndef-pos', 'ndef-pos', $1, $1, NOW(), NOW())"#,
    )
    .bind(USER_ID)
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_task_commission(pool: &PgPool) -> (i64, i64) {
    let scope_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, _t_, created_by_id)
           VALUES (isahl.gen_next_zuid(), 'ndef-任务委派', 'scope-definition', 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let entity_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, code, created_by_id)
           VALUES (isahl.gen_next_zuid(), '委派实体', 'NDEF-VIP', 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    (scope_id, entity_id)
}

async fn create_flow(pool: &PgPool, name: &str, ctx_id: Option<i64>, graph: &Value) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve" (notice, meta, code, fk_context, created_by_id, _f_, _t_)
           VALUES ($1, $2::jsonb, $3, $4, 1, '实现', '范例') RETURNING id"#,
    )
    .bind(name)
    .bind(graph.to_string())
    .bind(format!("NDEF-{}", name))
    .bind(ctx_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn conclusion_instance_count(pool: &PgPool, flow_id: i64) -> i64 {
    sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_statement" st
           WHERE st.deleted_at IS NULL AND st.tpl_id IS NOT NULL
             AND st.tpl_id IN (
               SELECT rs.ref_right FROM isahl."zc_id_operation_rr_statement" rs
               JOIN isahl."zc_id_process_rr_operation" rro
                 ON rro.ref_right = rs.ref_left AND rro.deleted_at IS NULL
               WHERE rro.ref_left = $1 AND rs.deleted_at IS NULL
             )"#,
    )
    .bind(flow_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn review_with_sign_mode_creates_review_instances_not_gate() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    seed_user_and_role(&pool).await;
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    // review + 签署模式 or_sign（2026-09-02 修复前：cate=or_sign → 运行时误判 gate）
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "r", "type": "review", "label": "技术评审", "role": "ndef-role", "mode": "or_sign", "next": [{"to": 2}]},
            {"id": "e", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let flow_id = create_flow(&pool, "Review", Some(scope_id), &graph).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "review+or_sign publish 应 200: {b}");

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "initiate 应 200: {b}");
    let ids = b["data"]["instance_ids"].as_array().expect("instance_ids");
    assert_eq!(
        ids.len(),
        1,
        "review+签署模式应按 human 节点建评审实例（非 gate 空扇出）: {b}"
    );
}

#[tokio::test]
async fn end_rejected_outcome_skips_conclusion_materialization() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    seed_user_and_role(&pool).await;
    grant_user_access(&pool, USER_ID, "approval-instances", &["approve"])
        .await
        .unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "a", "type": "approval", "label": "审批", "mode": "or_sign", "next": [{"to": 2}]},
            {"id": "e", "type": "end", "label": "拒绝结束", "outcome": "rejected", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let flow_id = create_flow(&pool, "EndRejected", Some(scope_id), &graph).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200: {b}");

    // end_outcome 物化到 operation.meta
    let meta_outcome: Option<String> = sqlx::query_scalar(
        r#"SELECT o.meta->>'end_outcome' FROM isahl."zc_id_operation" o
           JOIN isahl."zc_id_process_rr_operation" rro
             ON rro.ref_right = o.id AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = 'e' AND o.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    assert_eq!(
        meta_outcome.as_deref(),
        Some("rejected"),
        "end_outcome 应物化 rejected"
    );

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "initiate 应 200: {b}");
    let inst: i64 = b["data"]["instance_ids"][0]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-instances/{inst}/approve"),
        json!({"opinion": "同意"})
    );
    assert_eq!(s, 200, "approve 应 200: {b}");
    assert_eq!(
        conclusion_instance_count(&pool, flow_id).await,
        0,
        "outcome=rejected 终局不得物化结论实例"
    );
}

#[tokio::test]
async fn parallel_forks_all_branches_ignoring_cond() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    seed_user_and_role(&pool).await;
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    // parallel 两条出边（其中一条带 cond——并行语义忽略之，仍并发扇出）
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "p", "type": "parallel", "label": "并行分支", "next": [{"to": 2, "cond": "false"}, {"to": 3}]},
            {"id": "a1", "type": "approval", "label": "分支 A", "mode": "or_sign"},
            {"id": "a2", "type": "approval", "label": "分支 B", "mode": "or_sign"}
        ]
    });
    let flow_id = create_flow(&pool, "Parallel", Some(scope_id), &graph).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200: {b}");

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "initiate 应 200: {b}");

    // 两个分支目标均建实例（cond=false 被忽略；若按 select_targets 只剩 1 分支）
    let branch_insts: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           JOIN isahl."zc_id_operation_rr_event" oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           WHERE oa.tpl_id IS NOT NULL AND oa.deleted_at IS NULL
             AND oe.ref_right IN (
               SELECT oe2.ref_right FROM isahl."zc_id_operation_rr_event" oe2
               JOIN isahl."zc_id_process_rr_operation" rro
                 ON rro.ref_right = oe2.ref_left AND rro.deleted_at IS NULL
               WHERE rro.ref_left = $1 AND rro.code IN ('a1', 'a2')
                 AND oe2.deleted_at IS NULL
             )"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        branch_insts, 2,
        "parallel 应忽略 cond 全边扇出：a1/a2 均建审批实例"
    );
}
