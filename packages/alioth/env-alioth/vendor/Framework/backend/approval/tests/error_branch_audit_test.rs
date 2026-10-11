//! 错误分支审计集成测试（fix-approval-error-branch-integrity）：
//! 运行期无法继续推进的分支 MUST 留可查询审计痕迹（此前仅 warn，DB 无标记）——
//! ① decision 节点输出无匹配 label 边且无兜底 → `approval.decision_stall`；
//! ② rejectAction=back 但 backTo 越界（发布后载体被篡改）→ `approval.reject_route_degraded`。
//! 两例同时断言流程按现状停驻/降级（不推进、不报错）。

use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;

mod common;
use common::{grant_user_access, setup_test_schema};

const USER_ID: i64 = 447701;

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
        let s: u16 = resp.status().as_u16();
        let b: Value = test::read_body_json(resp).await;
        (s, b)
    }};
}

macro_rules! build_app {
    ($pool:expr, $bus:expr) => {{
        let ctx = ::common::context::RequestContext::with_username(
            USER_ID,
            "err-branch@test.local",
            "err-branch-test",
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

async fn test_user(pool: &PgPool) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, username, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'err-branch', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(USER_ID)
    .execute(pool)
    .await
    .unwrap();
}

/// 种子 task 域范畴定义 + 实体（code=C1 供 decision 规则求值）
async fn seed_task_commission(pool: &PgPool) -> (i64, i64) {
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("JE", "FMA", "↓_CH"))
        .await
        .unwrap();
    let scope_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, _t_, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), '错误分支-任务委派', 'scope-definition', 1, $1, $2, $3)
           RETURNING id"#,
    )
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .unwrap();
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("JE", "FMA", "↓_CH"))
        .await
        .unwrap();
    let entity_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, code, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), '委派实体', 'C1', 1, '实现', '范例', $1, $2, $3)
           RETURNING id"#,
    )
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .unwrap();
    (scope_id, entity_id)
}

async fn create_flow(pool: &PgPool, name: &str, ctx_id: Option<i64>, graph: &Value) -> i64 {
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool.clone(), ("JC", "FTA", "↑_NA"))
            .await
            .unwrap();
    let flow_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve" (notice, meta, code, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2::jsonb, $3, 1, '实现', '范例', $4, $5, $6) RETURNING id"#,
    )
    .bind(name)
    .bind(graph.to_string())
    .bind(format!("ERRB-{}", name))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .unwrap();
    if let Some(ctx) = ctx_id {
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_process_rr_context"
               (ref_left, ref_right, code, notice, created_by_id)
               VALUES ($1, $2, 'bind-context', '流程上下文绑定', 1)"#,
        )
        .bind(flow_id)
        .bind(ctx)
        .execute(pool)
        .await
        .unwrap();
    }
    flow_id
}

/// 审计行计数（operation + metadata.flow_id 双锚）
async fn audit_count(pool: &PgPool, operation: &str, flow_id: i64) -> i64 {
    sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl_audit.audit_events
           WHERE operation = $1 AND metadata->>'flow_id' = $2"#,
    )
    .bind(operation)
    .bind(flow_id.to_string())
    .fetch_one(pool)
    .await
    .unwrap()
}

/// ① decision 输出无匹配 label 边且无兜底 → 停驻 + `approval.decision_stall`
#[tokio::test]
async fn decision_unmatched_output_leaves_audit_and_stalls() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    // 发布期门禁会拦「输出无匹配 label 边且无兜底」的设计图（node[1] … 发布后必然停滞），
    // 故此处以「发布后载体 timeline.dmn 被改写」制造运行期漂移——运行期 decision 读载体
    // timeline.dmn（`advance_auto_node`），与图内 label 不再匹配且无兜底 → 空扇出停滞。
    let rules = json!([
        {"match": ["code == \"C1\""], "output": "go-a"},
        {"match": [null], "output": "go-b"}
    ]);
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "drive": "event",
             "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "n-decision", "type": "decision", "label": "分级决策",
             "dmn": {"hitPolicy": "FIRST", "inputs": [{"name": "code"}], "rules": rules},
             "next": [{"to": 2, "label": "go-a"}, {"to": 3, "label": "go-b"}]},
            {"id": "n-high", "type": "approve", "label": "高位审批", "next": [{"to": 4}]},
            {"id": "n-low", "type": "approve", "label": "常规审批", "next": [{"to": 4}]},
            {"id": "n-end", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let flow_id = create_flow(&pool, "decision-stall", Some(scope_id), &graph).await;
    let bus: Arc<dyn ::common::event_bus::DomainEventBus> =
        Arc::new(::common::event_bus::InMemoryEventBus::new());
    let app = build_app!(&pool, bus);

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200: {b}");

    // 载体 dmn 改写：规则输出 go-zzz（无匹配 label 边，且无兜底边）
    let tampered: u64 = sqlx::query(
        r#"UPDATE isahl."zc_id_even-approve" ea
           SET timeline = jsonb_set(
                 COALESCE(ea.timeline, '{}'::jsonb),
                 '{dmn,rules}',
                 $2::jsonb)
           FROM isahl.zc_id_operation_rr_event oe
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.deleted_at IS NULL
           WHERE oe.ref_right = ea.id AND oe.deleted_at IS NULL
             AND rro.ref_left = $1 AND rro.code = 'n-decision' AND ea.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(json!([{"match": ["code == \"C1\""], "output": "go-zzz"}]))
    .execute(&pool)
    .await
    .unwrap()
    .rows_affected();
    assert_eq!(tampered, 1, "decision 载体 dmn 应被改写为无匹配输出");

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "initiate 应 200（停驻不报错）: {b}");

    assert_eq!(
        audit_count(&pool, "approval.decision_stall", flow_id).await,
        1,
        "decision 无匹配边 MUST 留审计（approval.decision_stall）"
    );
    // 停驻：两分支均无实例，流程无推进
    let branch_instances: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oa.tpl_id AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code IN ('n-high', 'n-low')
             AND oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(branch_instances, 0, "空扇出不得创建任何分支实例");
}

/// ② backTo 越界（发布后载体篡改）→ 按 stop 降级 + `approval.reject_route_degraded`
#[tokio::test]
async fn reject_back_to_out_of_range_leaves_audit_and_stops() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    grant_user_access(&pool, USER_ID, "approval-instances", &["approve"])
        .await
        .unwrap();
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event",
             "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "a", "type": "approval", "label": "初审", "mode": "or_sign", "next": [{"to": 2}]},
            {"id": "b", "type": "approval", "label": "复审", "mode": "or_sign",
             "rejectAction": "back", "backTo": 1, "next": [{"to": 3}]},
            {"id": "e", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let flow_id = create_flow(&pool, "reject-degrade", Some(scope_id), &graph).await;
    let bus: Arc<dyn ::common::event_bus::DomainEventBus> =
        Arc::new(::common::event_bus::InMemoryEventBus::new());
    let app = build_app!(&pool, bus);

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

    // A 通过 → 生成 B（驳回降级面需 B 实例在场）
    let a1: i64 = sqlx::query_scalar(
        r#"SELECT oa.id FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_operation_rr_event oe2 ON oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe2.ref_left AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = 'a'
             AND oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL
           ORDER BY oa.id LIMIT 1"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-instances/{a1}/approve"),
        json!({"opinion": "同意"})
    );
    assert_eq!(s, 200, "A 通过应 200: {b}");

    // 发布后篡改载体 backTo → 越界（模拟载体与图漂移的降级面）
    let tampered: u64 = sqlx::query(
        r#"UPDATE isahl."zc_id_even-approve" ea
           SET timeline = jsonb_set(COALESCE(ea.timeline, '{}'::jsonb), '{backTo}', '999'::jsonb)
           FROM isahl.zc_id_operation_rr_event oe
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.deleted_at IS NULL
           WHERE oe.ref_right = ea.id AND oe.deleted_at IS NULL
             AND rro.ref_left = $1 AND rro.code = 'b' AND ea.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .execute(&pool)
    .await
    .unwrap()
    .rows_affected();
    assert_eq!(tampered, 1, "节点 b 载体 backTo 应被篡改为越界值");

    // 取节点 b 的在途实例并驳回
    let instance: i64 = sqlx::query_scalar(
        r#"SELECT oa.id FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_operation_rr_event oe2 ON oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe2.ref_left AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = 'b'
             AND oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL
           ORDER BY oa.id LIMIT 1"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-instances/{instance}/reject"),
        json!({"opinion": "不通过"})
    );
    assert_eq!(s, 200, "reject 应 200（降级不报错）: {b}");

    assert_eq!(
        audit_count(&pool, "approval.reject_route_degraded", flow_id).await,
        1,
        "backTo 越界 MUST 留审计（approval.reject_route_degraded）"
    );
    // 降级按 stop：目标节点不得重开新实例（本流程仅原初审实例仍在途）
    let rework_instances: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oa.tpl_id AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = 'a'
             AND oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rework_instances, 1, "降级为 stop：打回目标不得重开实例");
}
