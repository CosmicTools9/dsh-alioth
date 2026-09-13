//! 静态缺陷扫描门禁集成测试（add-approval-flow-static-scan）：
//! - validate：分级响应（400 + details.errors 硬错 / 200 + warnings 疑似；空审批人 DB 实查）
//! - publish：硬错阻断（不物化）/ 仅警告放行（物化成功）
//! - 物化行数 == 扫描图节点数（envelope 解析一致性锚点）

use ::common::event_bus::{DomainEventBus, InMemoryEventBus};
use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
mod common;
use common::setup_test_schema;

const USER_ID: i64 = 424601;

macro_rules! build_app {
    ($pool:expr, $bus:expr) => {{
        let ctx = ::common::context::RequestContext::with_username(
            USER_ID,
            "scan@test.local",
            "scan-test",
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
                        .configure(handlers::validate::register)
                        .configure(approval::simulate::register),
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

fn end_node(id: &str, label: &str) -> Value {
    json!({
        "id": id, "type": "end", "label": label, "outcome": "complete",
        "statementLeaf": "zc_id_stat-inspection",
    })
}

/// 仅警告图：approve 节点无审批人配置 + 全条件出边无兜底；end 可达 → 扫描零 error
fn warning_only_graph() -> Value {
    json!({
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event",
             "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "a1", "type": "approve", "label": "会签审批", "next": [
                {"to": 2, "cond": "amount > 1000"},
                {"to": 3, "cond": "amount <= 1000"},
            ]},
            end_node("e1", "通过结束"),
            end_node("e2", "拒绝结束"),
        ]
    })
}

/// 硬错图：自动节点成环无出口（且 end 不可达）→ AUTO_CYCLE + END_UNREACHABLE
fn auto_cycle_graph() -> Value {
    json!({
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event",
             "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "c1", "type": "condition", "label": "条件A", "next": [{"to": 2}]},
            {"id": "c2", "type": "condition", "label": "条件B", "next": [{"to": 1}]},
            end_node("e1", "结束"),
        ]
    })
}

/// 无 end 图（NO_END 硬错）
fn no_end_graph() -> Value {
    json!({
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event",
             "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "a1", "type": "approve", "label": "审批", "direct": {"pos": "部门经理"}},
        ]
    })
}

/// 空审批人 DB 实查图：配置存在但岗位不存在 → APPROVER_EMPTY（非 UNCONFIGURED）
fn empty_approver_graph() -> Value {
    json!({
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event",
             "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "a1", "type": "approve", "label": "审批", "direct": {"pos": "岗位-不存在-scan-xyz"}, "next": [{"to": 2}]},
            end_node("e1", "结束"),
        ]
    })
}

async fn create_flow(pool: &PgPool, code: &str, graph: &Value) -> i64 {
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool.clone(), ("JC", "FTA", "↑_NA"))
            .await
            .unwrap();
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve" (notice, meta, code, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2::jsonb, $3, 1, '实现', '范例', $4, $5, $6) RETURNING id"#,
    )
    .bind(code)
    .bind(graph.to_string())
    .bind(format!("SCAN-{code}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("insert flow")
}

fn error_codes(body: &Value) -> Vec<String> {
    body["details"]["errors"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|f| f["code"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn warning_codes(body: &Value) -> Vec<String> {
    body["data"]["warnings"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|f| f["code"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn publish_blocked_by_auto_cycle() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool.clone(), bus);
    let flow_id = create_flow(&pool, "AutoCycle", &auto_cycle_graph()).await;

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 400, "自动环发布应被拒: {b}");
    let codes = error_codes(&b);
    assert!(
        codes.iter().any(|c| c == "AUTO_CYCLE"),
        "errors 应含 AUTO_CYCLE: {codes:?}"
    );
    // 阻断 = 未物化：rr_operation 零行
    let rows: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_process_rr_operation
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 0, "硬错阻断不得物化任何节点/边");
}

#[tokio::test]
async fn publish_warns_only_and_materializes() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool.clone(), bus);
    let flow_id = create_flow(&pool, "WarnOnly", &warning_only_graph()).await;

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "仅警告发布应成功: {b}");
    // 物化行数 == 图节点数（envelope 解析一致性锚点）
    let rows: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_process_rr_operation
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 4, "物化 operation 行数应与图节点数一致");
}

#[tokio::test]
async fn validate_graded_warnings() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);

    let (s, b) = post_json!(&app, "/test/approval-flows/validate", warning_only_graph());
    assert_eq!(s, 200, "仅警告校验应 200: {b}");
    assert_eq!(b["data"]["valid"], json!(true));
    let codes = warning_codes(&b);
    assert!(
        codes.iter().any(|c| c == "EMPTY_FANOUT"),
        "warnings 应含 EMPTY_FANOUT: {codes:?}"
    );
    assert!(
        codes.iter().any(|c| c == "APPROVER_UNCONFIGURED"),
        "warnings 应含 APPROVER_UNCONFIGURED: {codes:?}"
    );
}

#[tokio::test]
async fn validate_hard_error_returns_400_with_details() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);

    let (s, b) = post_json!(&app, "/test/approval-flows/validate", no_end_graph());
    assert_eq!(s, 400, "无 end 图校验应 400: {b}");
    assert_eq!(b["code"], json!("VALIDATION_ERROR"));
    let codes = error_codes(&b);
    assert!(
        codes.iter().any(|c| c == "NO_END"),
        "details.errors 应含 NO_END: {codes:?}"
    );
}

#[tokio::test]
async fn validate_empty_approver_db_check_warns() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);

    let (s, b) = post_json!(
        &app,
        "/test/approval-flows/validate",
        empty_approver_graph()
    );
    assert_eq!(s, 200, "空审批人属警告: {b}");
    let codes = warning_codes(&b);
    assert!(
        codes.iter().any(|c| c == "APPROVER_EMPTY"),
        "warnings 应含 APPROVER_EMPTY（DB 实查空集）: {codes:?}"
    );
    assert!(
        !codes.iter().any(|c| c == "APPROVER_UNCONFIGURED"),
        "配置存在不应报 UNCONFIGURED: {codes:?}"
    );
}

#[tokio::test]
async fn simulate_replay_reaches_end_with_zero_writes() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool.clone(), bus);
    let flow_id = create_flow(&pool, "SimWarn", &warning_only_graph()).await;

    let inst_before: i64 = sqlx::query_scalar(r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve""#)
        .fetch_one(&pool)
        .await
        .unwrap();
    // 走查：s → a1(人工，意图走第 0 条出边 → e1 end)
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/simulate"),
        json!({
            "context": {},
            "steps": [{ "node": "a1", "edge": 0 }]
        })
    );
    assert_eq!(s, 200, "simulate 应 200: {b}");
    assert_eq!(b["data"]["reached_end"], json!(true), "应到达 end: {b}");
    assert!(b["data"]["awaiting_human"].is_null(), "{b}");
    let inst_after: i64 = sqlx::query_scalar(r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve""#)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(inst_after, inst_before, "dry-run 不得物化任何审批实例");
    let flow_rows: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_process_rr_operation"
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(flow_rows, 0, "simulate 不得物化节点行（流程仍为零物化）");
}

#[tokio::test]
async fn simulate_waits_when_human_intent_missing() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let flow_id = create_flow(&pool, "SimWait", &warning_only_graph()).await;

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/simulate"),
        json!({ "context": {}, "steps": [] })
    );
    assert_eq!(s, 200);
    assert_eq!(
        b["data"]["awaiting_human"],
        json!("a1"),
        "应停在人工节点等待意图: {b}"
    );
    assert_eq!(b["data"]["reached_end"], json!(false));
}

fn proven_empty_fanout_graph() -> Value {
    json!({
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event",
             "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "c1", "type": "condition", "label": "必停条件", "next": [
                {"to": 2, "cond": "2 > 3"},
                {"to": 3, "cond": "amount > 9999 AND amount < 1"},
            ]},
            end_node("e1", "结束A"),
            end_node("e2", "结束B"),
        ]
    })
}

fn infeasible_edge_with_default_graph() -> Value {
    json!({
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event",
             "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "a1", "type": "approve", "label": "审批", "direct": {"pos": "部门经理"}, "next": [
                {"to": 2, "cond": "1 > 2"},
                {"to": 3},
            ]},
            end_node("e1", "通过结束"),
            end_node("e2", "拒绝结束"),
        ]
    })
}

#[tokio::test]
async fn publish_blocked_by_proven_empty_fanout() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool.clone(), bus);
    let flow_id = create_flow(&pool, "ProvenFanout", &proven_empty_fanout_graph()).await;

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 400, "可证明空扇出发布应被拒: {b}");
    let codes = error_codes(&b);
    assert!(
        codes.iter().any(|c| c == "FANOUT_EMPTY_PROVEN"),
        "errors 应含 FANOUT_EMPTY_PROVEN: {codes:?}"
    );
    let rows: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_process_rr_operation
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 0, "阻断发布不得物化");
}

#[tokio::test]
async fn validate_warns_infeasible_edge_with_default_allowed() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);

    let (s, b) = post_json!(
        &app,
        "/test/approval-flows/validate",
        infeasible_edge_with_default_graph()
    );
    assert_eq!(s, 200, "恒假边 + 兜底 → 仅警告: {b}");
    let codes = warning_codes(&b);
    assert!(
        codes.iter().any(|c| c == "EDGE_INFEASIBLE"),
        "warnings 应含 EDGE_INFEASIBLE: {codes:?}"
    );
    assert!(
        !codes.iter().any(|c| c == "FANOUT_EMPTY_PROVEN"),
        "含兜底不得升级阻断: {codes:?}"
    );
}
