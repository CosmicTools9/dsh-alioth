//! 运行时链路集成测试（fix-flow-designer-runtime-chain）：
//! validate / initiate（实体桥 + 条件选边）/ restore（快照重发布）/ cc 事件。

use ::common::event_bus::{DomainEventBus, InMemoryEventBus};
use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
mod common;
use common::{ensure_role_member, setup_test_schema};

const USER_ID: i64 = 424242;

macro_rules! build_app {
    ($pool:expr) => {{
        let ctx =
            ::common::context::RequestContext::with_username(USER_ID, "rt@test.local", "rt-test");
        test::init_service(
            App::new().app_data(web::Data::new($pool.clone())).service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::validate::register)
                    .configure(handlers::context_field_domain::register)
                    .configure(handlers::initiate::register)
                    .configure(handlers::version::register)
                    .configure(handlers::publish::register),
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
        let _status: u16 = resp.status().as_u16();
        let _body: Value = test::read_body_json(resp).await;
        (_status, _body)
    }};
}

async fn test_user(pool: &PgPool) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'rt-test', 'rt-test', 'rt@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(USER_ID)
    .execute(pool)
    .await
    .unwrap();
}

/// 建流程行（meta 设计图 + fk_context 范畴定义行）
async fn create_flow(pool: &PgPool, name: &str, ctx_id: Option<i64>, graph: &Value) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve" (notice, meta, code, fk_context, created_by_id, _f_, _t_)
           VALUES ($1, $2::jsonb, 'draft', $3, 1, '实现', '范例') RETURNING id"#,
    )
    .bind(name)
    .bind(graph.to_string())
    .bind(ctx_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 在 zc_id_task-commission 建范畴定义行（scope-definition）与业务实体行
async fn seed_task_commission(pool: &PgPool) -> (i64, i64) {
    let scope_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, _t_, created_by_id)
           VALUES (isahl.gen_next_zuid(), '测试-任务委派', 'scope-definition', 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .expect("insert scope-def row");
    let entity_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, code, created_by_id)
           VALUES (isahl.gen_next_zuid(), '委派实体', 'VIP', 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .expect("insert entity row");
    (scope_id, entity_id)
}

#[tokio::test]
async fn validate_endpoint_accepts_and_rejects() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let app = build_app!(pool);

    let (s, b): (u16, Value) = post_json!(
        &app,
        "/test/approval-flows/validate",
        json!({"nodes": [{"id": "s", "type": "start", "label": "开始", "eventLeaf": "zc_id_even-accident"}]})
    );
    assert_eq!(s, 200, "合法图应 200: {b}");

    let (s, b) = post_json!(
        &app,
        "/test/approval-flows/validate",
        json!({"nodes": [{"id": "x", "type": "subflow", "label": "子流程", "target": "AF-OTHER"}]})
    );
    // 2026-09-01 能力补齐：subflow 放行白名单（validate 不查 target 存在性——
    // publish 物化时校验 target 引用存在且已发布）
    assert_eq!(s, 200, "subflow 类型应通过 validate: {b}");

    let (s, b) = post_json!(
        &app,
        "/test/approval-flows/validate",
        json!({"nodes": [{"id": "x", "type": "not-a-type", "label": "未知"}]})
    );
    assert_eq!(s, 400, "未知类型应 400: {b}");

    let (s, _) = post_json!(&app, "/test/approval-flows/validate", json!({"foo": 1}));
    assert_eq!(s, 400, "无 nodes 应 400");
}

#[tokio::test]
async fn initiate_creates_chain_with_entity_binding_and_condition() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    ensure_role_member(&pool, "default", USER_ID).await.unwrap();

    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "n-cond", "type": "condition", "label": "VIP 判断",
             "next": [{"to": 2, "cond": "code == 'VIP'"}, {"to": 3}]},
            {"id": "n-a", "type": "approve", "label": "VIP 审批"},
            {"id": "n-b", "type": "approve", "label": "普通审批"},
            {"id": "n-end", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let flow_id = create_flow(&pool, "发起链路测试", Some(scope_id), &graph).await;
    let app = build_app!(pool);

    let (s, b): (u16, Value) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200: {b}");

    // 未绑定范畴流程拒绝
    let bare = create_flow(&pool, "无范畴流程", None, &graph).await;
    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{bare}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 400, "未绑定范畴应 400");

    // 范畴不一致拒绝
    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_even-alert", "entity_id": entity_id})
    );
    assert_eq!(s, 400, "范畴不一致应 400");

    // 正常发起（code='VIP' → 条件边 A）
    let (s, b): (u16, Value) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "initiate 应 200: {b}");
    let ids = b["data"]["instance_ids"].as_array().expect("instance_ids");
    // DEBUG: 列出 flow 全部实例与节点
    {
        let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
            r#"SELECT rro.code, o.notice, s.code FROM isahl."zc_id_process_rr_operation" rro
               LEFT JOIN isahl."zc_id_oper-approve" oa ON oa.id = rro.ref_right AND oa.deleted_at IS NULL
               LEFT JOIN isahl."zc_id_lifecycle_r_primary-status" ls ON ls.ref_left = oa.id AND ls.deleted_at IS NULL
               LEFT JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
               LEFT JOIN isahl."zc_id_operation" o ON o.id = rro.ref_right
               WHERE rro.ref_left = $1 AND rro.deleted_at IS NULL"#,
        )
        .bind(flow_id)
        .fetch_all(&pool)
        .await
        .unwrap();
        eprintln!("DBG flow nodes: {:?}", rows);
        let insts: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" WHERE tpl_id IS NOT NULL"#,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        eprintln!("DBG total instances: {}", insts);
    }
    assert_eq!(ids.len(), 1, "应创建 1 个首链实例: {b}");

    let instance_id = ids[0].as_str().unwrap().parse::<i64>().unwrap();
    let node: String = sqlx::query_scalar(
        r#"SELECT ea.notice FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           JOIN isahl."zc_id_even-approve" ea ON ea.id = oe.ref_right AND ea.deleted_at IS NULL
           WHERE oa.id = $1
             AND EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_event oe2
                         JOIN isahl.zc_id_process_rr_operation rro2
                           ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL
                         WHERE oe2.ref_right = ea.id AND oe2.deleted_at IS NULL)
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(instance_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(node, "VIP 审批", "条件边应命中 VIP 节点");

    let bound: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(SELECT 1 FROM isahl.zc_id_operation_rr_task
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL)"#,
    )
    .bind(instance_id)
    .bind(entity_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(bound, "实例应携带实体桥绑定");

    let comments: Option<String> =
        sqlx::query_scalar(r#"SELECT comments FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
            .bind(instance_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let c = comments.unwrap_or_default();
    assert!(c.contains("流程发起"), "comments 应为发起摘要，实际: {c}");
    assert!(
        !c.trim_start().starts_with('{'),
        "comments MUST NOT 为 JSON 信封: {c}"
    );
}

#[tokio::test]
async fn restore_republishes_snapshot_and_rejects_legacy() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    ensure_role_member(&pool, "default", USER_ID).await.unwrap();
    let (scope_id, _) = seed_task_commission(&pool).await;

    let graph_v1 = json!({
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "eventLeaf": "zc_id_even-accident"},
            {"id": "a", "type": "approve", "label": "V1审批"},
            {"id": "e", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let flow_id = create_flow(&pool, "版本恢复测试", Some(scope_id), &graph_v1).await;
    let app = build_app!(pool);

    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200);

    let graph_v2 = json!({
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "eventLeaf": "zc_id_even-accident"},
            {"id": "b", "type": "approve", "label": "V2审批"},
            {"id": "e", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    sqlx::query("UPDATE isahl.zc_id_process SET comments = $2 WHERE id = $1")
        .bind(flow_id)
        .bind(graph_v2.to_string())
        .execute(&pool)
        .await
        .unwrap();
    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200);

    let (s, b): (u16, Value) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/versions/1/restore"),
        json!({})
    );
    assert_eq!(s, 200, "restore v1 应 200: {b}");
    let nodes = b["data"]["nodes"].as_array().expect("nodes");
    let labels: Vec<&str> = nodes.iter().filter_map(|n| n["label"].as_str()).collect();
    assert!(labels.contains(&"V1审批"), "恢复后应含 V1 节点: {labels:?}");
    assert!(
        !labels.contains(&"V2审批"),
        "恢复后不应含 V2 节点: {labels:?}"
    );

    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/versions/99/restore"),
        json!({})
    );
    assert_eq!(s, 400, "legacy 版本应 400");
}

#[tokio::test]
async fn entity_created_auto_initiates_bound_flow() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    ensure_role_member(&pool, "default", USER_ID).await.unwrap();
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    let graph = json!({
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "a", "type": "approve", "label": "自动触发审批"},
            {"id": "e", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let flow_id = create_flow(&pool, "自动触发测试", Some(scope_id), &graph).await;
    let app = build_app!(pool);
    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200");

    // 实体创建 → maybe_auto_initiate（EntityCreated 订阅处理核心）→ 自动发起
    approval::handlers::auto_initiate::maybe_auto_initiate(
        &pool,
        "zc_id_task-commission",
        entity_id,
        USER_ID,
        None,
    )
    .await
    .expect("maybe_auto_initiate");

    let created: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
             SELECT 1 FROM isahl."zc_id_oper-approve" oa
             JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
             JOIN isahl."zc_id_even-approve" ea ON ea.id = oe.ref_right AND ea.deleted_at IS NULL
             WHERE oa.tpl_id IS NOT NULL AND oa.fk_subject = $2
               AND EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_event oe2
                           JOIN isahl.zc_id_process_rr_operation rro2
                             ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL
                           WHERE oe2.ref_right = ea.id AND oe2.deleted_at IS NULL
                             AND rro2.ref_left = $1))"#,
    )
    .bind(flow_id)
    .bind(USER_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(created, "实体创建后应自动发起绑定流程实例");

    // 非三域叶表实体 → 静默跳过
    approval::handlers::auto_initiate::maybe_auto_initiate(
        &pool,
        "zc_id_process",
        flow_id,
        USER_ID,
        None,
    )
    .await
    .expect("非三域叶表应静默跳过");
}

#[tokio::test]
async fn reaching_end_materializes_conclusion_statement() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    ensure_role_member(&pool, "default", USER_ID).await.unwrap();
    common::grant_user_access(&pool, USER_ID, "approval-instances", &["approve"])
        .await
        .unwrap();
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    // 终端节点语义链（2026-08-29 裁决）：end 配置 statement 叶表 stat-inspection
    let graph = json!({
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "a", "type": "approve", "label": "审批", "next": [{"to": 2}]},
            {"id": "e", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let flow_id = create_flow(&pool, "终局陈述测试", Some(scope_id), &graph).await;

    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let ctx =
        ::common::context::RequestContext::with_username(USER_ID, "end@test.local", "end-test");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(bus))
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
    .await;

    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200");

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "initiate 应 200: {b}");
    let ids = b["data"]["instance_ids"].as_array().expect("instance_ids");
    let inst: i64 = ids[0].as_str().unwrap().parse().unwrap();

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-instances/{inst}/approve"),
        json!({"opinion": "同意"})
    );
    assert_eq!(s, 200, "approve 应 200: {b}");

    // 终局断言 1：end 范例（stat-inspection 叶）存在且经 rr_statement 挂 op 行
    let tpl_id: i64 = sqlx::query_scalar(
        r#"SELECT rs.ref_right
           FROM isahl.zc_id_operation_rr_statement rs
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = rs.ref_left AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = 'e' AND rs.deleted_at IS NULL
           LIMIT 1"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    // 终局断言 2：结论 statement 实例同表物化（tpl_id→范例，tpl_id 同表关联铁律）
    let concl: Vec<(i64, String, i64)> = sqlx::query_as(
        r#"SELECT id, replace(tableoid::regclass::text, '"', ''), tpl_id
           FROM isahl.zc_id_statement WHERE tpl_id = $1 AND deleted_at IS NULL"#,
    )
    .bind(tpl_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        concl.len(),
        1,
        "到达 end 应物化恰好 1 条结论 statement 实例"
    );
    assert_eq!(concl[0].1, "zc_id_stat-inspection", "实例与范例同叶表");
    assert_eq!(concl[0].2, tpl_id, "tpl_id 指向范例");

    // 终局断言 3：结论实例经 rr_statement 挂 gate 实例（oper-gate 行）
    let gate_bound: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
             SELECT 1 FROM isahl.zc_id_operation_rr_statement rs
             JOIN isahl."zc_id_oper-gate" g ON g.id = rs.ref_left AND g.deleted_at IS NULL
             WHERE rs.ref_right = $1 AND rs.deleted_at IS NULL)"#,
    )
    .bind(concl[0].0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        gate_bound,
        "结论 statement 实例应经 rr_statement 挂 end 节点 gate 实例"
    );
}

#[tokio::test]
async fn cc_node_publishes_event_on_advance() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    ensure_role_member(&pool, "default", USER_ID).await.unwrap();
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    let graph = json!({
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "c", "type": "cc", "label": "抄送财务", "recipients": "role:finance", "next": [{"to": 2}]},
            {"id": "a", "type": "approve", "label": "审批"},
            {"id": "e", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let flow_id = create_flow(&pool, "CC事件测试", Some(scope_id), &graph).await;
    let app = build_app!(pool);

    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200);

    let bus = Arc::new(InMemoryEventBus::new());
    let mut rx = bus
        .subscribe("ApprovalCc")
        .await
        .expect("subscribe ApprovalCc");

    let ids = approval::advance::initiate_flow(
        &pool,
        flow_id,
        USER_ID,
        "zc_id_task-commission",
        entity_id,
        // 无执行锚点（直调发起；HTTP 路径由 initiate handler 物化 实现·实例 行）
        None,
        Some(&(bus.clone() as Arc<dyn DomainEventBus>)),
    )
    .await
    .expect("initiate_flow");
    assert_eq!(ids.len(), 1, "首链应有 1 个审批实例");

    let evt = rx.try_recv().ok();
    assert!(evt.is_some(), "推进至 cc 节点应发布 ApprovalCc 事件");
    let evt = evt.unwrap();
    assert_eq!(evt.event_type, "ApprovalCc");
    assert_eq!(
        evt.payload.get("recipients").and_then(|v| v.as_str()),
        Some("role:finance")
    );
    assert_eq!(
        evt.payload.get("entity_id").and_then(|v| v.as_i64()),
        Some(entity_id)
    );
}

#[tokio::test]
async fn context_field_domain_endpoint_contract() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let app = build_app!(pool);

    // 白名单外叶表 → 400
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/test/approval-flows/context-field-domain?table=zc_id_process&column=fk_list")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 400, "非三域叶表 MUST 400");

    // 在册 lookup 字段（subjects 目标可能空集）→ 200 空数组亦合法
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/test/approval-flows/context-field-domain?table=zc_id_appr-payment&column=fk_subject")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200, "在册值域字段应 200");
}
