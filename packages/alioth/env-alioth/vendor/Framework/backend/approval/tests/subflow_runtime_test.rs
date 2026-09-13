//! subflow 运行语义补充测试（test-subflow-runtime-semantics）：
//! - 非 wait 触发：父流程不等待子流程终局，沿后续节点继续推进
//! - target 降级：目标流程不存在 → 仅 gate 推进（warn 不阻断）
//! - wait 对照锚：同子流程 wait=true 父流程停驻（防双模式混同回归）
//!
//! wait=true 停驻 + end 回调续推主链已由 subflow_wait_test.rs 覆盖，本文件
//! 补其未覆盖的两路径 + 双模式对照。

use ::common::event_bus::{DomainEventBus, InMemoryEventBus};
use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
mod common;
use common::{grant_user_access, setup_test_schema};

const USER_ID: i64 = 425201;

macro_rules! build_app {
    ($pool:expr, $bus:expr) => {{
        let ctx =
            ::common::context::RequestContext::with_username(USER_ID, "sr@test.local", "sr-test");
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

async fn seed_scope(pool: &PgPool) -> (i64, i64) {
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("JE", "FMA", "↓_CH"))
        .await
        .unwrap();
    let scope_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, _t_, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), '子流程运行-任务委派', 'scope-definition', 1, $1, $2, $3)
           RETURNING id"#,
    )
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .unwrap();
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("JE", "FMA", "↓_CH"))
        .await
        .unwrap();
    let entity_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, code, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), '委派实体', 'SR-VIP', 1, '实现', '范例', $1, $2, $3)
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

async fn create_flow(
    pool: &PgPool,
    name: &str,
    code: &str,
    ctx_id: Option<i64>,
    graph: &Value,
) -> i64 {
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
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
    .bind(code)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .unwrap();
    if let Some(ctx) = ctx_id {
        // 输入范畴经 zc_id_process_rr_context 桥落行（物理列已移除）
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

/// 某流程模板下 pending 审批实例数（父后续/子流程分别计数）
async fn pending_count(pool: &PgPool, flow_id: i64) -> i64 {
    sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           JOIN isahl."zc_id_operation_rr_event" oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           WHERE oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL
             AND oe.ref_right IN (
               SELECT oe2.ref_right FROM isahl."zc_id_operation_rr_event" oe2
               JOIN isahl."zc_id_process_rr_operation" rro
                 ON rro.ref_right = oe2.ref_left AND rro.deleted_at IS NULL
               WHERE rro.ref_left = $1 AND oe2.deleted_at IS NULL
             )
             AND NOT EXISTS (
               SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
               JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
               WHERE ls.ref_left = oa.id AND ls.deleted_at IS NULL
                 AND s.code IN ('approved','rejected','withdrawn','cancelled','abstained')
             )"#,
    )
    .bind(flow_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 子流程图（含 end 供 wait target 合法）
fn child_graph() -> Value {
    json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "a", "type": "approval", "label": "子审批", "mode": "or_sign", "next": [{"to": 2}]},
            {"id": "e", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    })
}

/// 父流程图：start → subflow(target) → 父后续 approval
fn parent_graph(target: &str, wait: bool) -> Value {
    let sub = if wait {
        json!({"id": "sub", "type": "subflow", "label": "子流程", "wait": true, "target": target, "next": [{"to": 2}]})
    } else {
        json!({"id": "sub", "type": "subflow", "label": "子流程", "target": target, "next": [{"to": 2}]})
    };
    json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            sub,
            {"id": "b", "type": "approval", "label": "父后续", "mode": "or_sign"}
        ]
    })
}

#[tokio::test]
async fn subflow_without_wait_continues_parent_immediately() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    grant_user_access(&pool, USER_ID, "approval-instances", &["approve"])
        .await
        .unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_scope(&pool).await;

    let code_c = format!(
        "SR-C-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let cf = create_flow(&pool, "子流程", &code_c, Some(scope_id), &child_graph()).await;
    let (s, _b) = post_json!(
        &app,
        &format!("/test/approval-flows/{cf}/publish"),
        json!({})
    );
    assert_eq!(s, 200);

    // 非 wait 父流程
    let code_p = format!("SR-P-{}", code_c);
    let pf = create_flow(
        &pool,
        "父流程-非wait",
        &code_p,
        Some(scope_id),
        &parent_graph(&code_c, false),
    )
    .await;
    let (s, _b) = post_json!(
        &app,
        &format!("/test/approval-flows/{pf}/publish"),
        json!({})
    );
    assert_eq!(s, 200);

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{pf}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "父 initiate: {b}");

    // 子流程首实例已创建 + 父后续实例已出现（父不等待）
    assert_eq!(
        pending_count(&pool, cf).await,
        1,
        "子流程首链应已创建（pending）"
    );
    assert_eq!(
        pending_count(&pool, pf).await,
        1,
        "父后续应已推进（非 wait 不等待子流程终局）"
    );
}

#[tokio::test]
async fn subflow_missing_target_degrades_to_gate_only() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    grant_user_access(&pool, USER_ID, "approval-instances", &["approve"])
        .await
        .unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_scope(&pool).await;

    // target 指向不存在的 code（从未发布）
    let code_missing = format!(
        "SR-MISSING-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    // 发布校验要求 target 存在——故缺失 target 的图无法发布？查语义：
    // publish 校验 target 存在（gateway_capabilities 400）。降级路径仅当
    // publish 后 target 被删/未发布时触发。构造：publish 时合法 → 删除
    // target 流程（软删）→ initiate 父 → 降级。此处用未发布同语义：
    // 先建 target 流程但**不发布**，父发布校验会被拦（400）——降级测试
    // 需绕过 publish 期校验：直接 seed 父流程为已发布态？复杂。
    // 采用最直接路径：target 流程 publish 后软删（deleted_at），父 initiate
    // 时 advance 查不到 → 降级仅 gate。
    let cf = create_flow(
        &pool,
        "将被删子流程",
        &code_missing,
        Some(scope_id),
        &child_graph(),
    )
    .await;
    let (s, _b) = post_json!(
        &app,
        &format!("/test/approval-flows/{cf}/publish"),
        json!({})
    );
    assert_eq!(s, 200);

    let code_p = format!("SR-DEG-{}", code_missing);
    let pf = create_flow(
        &pool,
        "父流程-降级",
        &code_p,
        Some(scope_id),
        &parent_graph(&code_missing, false),
    )
    .await;
    let (s, _b) = post_json!(
        &app,
        &format!("/test/approval-flows/{pf}/publish"),
        json!({})
    );
    assert_eq!(s, 200);

    // 软删子流程（target 运行期不可用）
    sqlx::query(
        r#"UPDATE isahl."zc_id_proc-approve" SET deleted_at = NOW()
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(cf)
    .execute(&pool)
    .await
    .unwrap();

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{pf}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "target 不可用父 initiate 应 200 降级: {b}");
    // 父后续实例出现（仅 gate 推进）；子流程零实例（已删 target 不触发）
    assert_eq!(
        pending_count(&pool, pf).await,
        1,
        "父后续应推进（降级仅 gate 不阻断）"
    );
    assert_eq!(pending_count(&pool, cf).await, 0, "已删子流程不应有实例");
}

#[tokio::test]
async fn subflow_wait_holds_parent_while_child_runs() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    grant_user_access(&pool, USER_ID, "approval-instances", &["approve"])
        .await
        .unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_scope(&pool).await;

    let code_c = format!(
        "SR-WC-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let cf = create_flow(&pool, "子流程", &code_c, Some(scope_id), &child_graph()).await;
    let (s, _b) = post_json!(
        &app,
        &format!("/test/approval-flows/{cf}/publish"),
        json!({})
    );
    assert_eq!(s, 200);

    // wait=true 父流程（与测试1 非 wait 对照）
    let code_p = format!("SR-WP-{}", code_c);
    let pf = create_flow(
        &pool,
        "父流程-wait",
        &code_p,
        Some(scope_id),
        &parent_graph(&code_c, true),
    )
    .await;
    let (s, _b) = post_json!(
        &app,
        &format!("/test/approval-flows/{pf}/publish"),
        json!({})
    );
    assert_eq!(s, 200);

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{pf}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "父 initiate: {b}");

    // 对照锚：子流程实例已建（1 pending）但父后续零实例（wait 停驻）
    assert_eq!(pending_count(&pool, cf).await, 1, "子流程首链应已创建");
    assert_eq!(
        pending_count(&pool, pf).await,
        0,
        "wait 父流程应停驻（父后续零实例——与非 wait 测试1 行为区分）"
    );
}
