//! fix-approval-engine-semantics 指派/会签/条件选边集成测试（P0-1/P1-4/P1-5）
//!
//! 验证（引擎层，经 HTTP approve 端点 + 直接 advance 调用）：
//! - 岗位 → NGAC UA 成员 → fk_operator；fk_subject 透传发起人；fk_previous 串联
//! - and_sign：全员通过才推进；任一驳回 → 兄弟取消
//! - or_sign：首个通过定案推进，其余兄弟 cancelled
//! - sequential：按解析顺序逐个建实例
//! - 空岗位 → fk_operator NULL（不回退自审）
//! - 边条件求值选边 + fail-closed

mod common;

use ::common::testing::connect_test_db;
use actix_web::{dev::Service, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::json;
use sqlx::PgPool;

use common::{ensure_role_member, grant_user_access, setup_test_schema, wire_approval_node};

/// 节点规格（测试流构造）
struct NodeSpec {
    code: &'static str,
    node_type: &'static str,
    next: Vec<i64>, // 出边（node_id）
}

/// 构造流程（refactor-flow-node-operation-model：节点=操作）：
/// even-approve 模板 + operation 主体 + 模板桥 + rro（ref_right=operation）。
/// 返回 (flow_id, op_ids)
async fn make_flow(pool: &PgPool, flow_code: &str, nodes: &[NodeSpec]) -> (i64, Vec<i64>) {
    let flow_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_process (id, notice, code, comments, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 'test', 1)
           RETURNING id"#,
    )
    .bind(flow_code)
    .bind(flow_code)
    .fetch_one(pool)
    .await
    .unwrap();

    let mut ids = Vec::new();
    for spec in nodes.iter() {
        let op_id = add_node(pool, flow_id, spec.code, spec.node_type).await;
        sqlx::query(
            r#"UPDATE isahl.zc_id_process_rr_operation
               SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
        )
        .bind(flow_id)
        .bind(json!(spec.next))
        .bind(op_id)
        .execute(pool)
        .await
        .unwrap();
        ids.push(op_id);
    }
    (flow_id, ids)
}

/// 节点构造（新形态）：even-approve 模板 + operation 主体（按动作子类）+
/// 模板桥（rr_event）+ rro（ref_right=operation）。返回 operation id。
async fn add_node(pool: &PgPool, flow_id: i64, code: &str, node_type: &str) -> i64 {
    let template_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_even-approve"
           (notice, created_by_id, code, comments)
           VALUES ($1, 1, $2, $3)
           RETURNING id"#,
    )
    .bind(code)
    .bind(code)
    .bind(code)
    .fetch_one(pool)
    .await
    .unwrap();
    let op_id: i64 = match node_type {
        "action" => sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_oper-action" (notice, code, created_by_id)
               VALUES ($1, $2, 1) RETURNING id"#,
        )
        .bind(code)
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap(),
        "approve" | "approval" | "oper-approve" => sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_oper-approve" (notice, code, created_by_id)
               VALUES ($1, $2, 1) RETURNING id"#,
        )
        .bind(code)
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap(),
        _ => sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_oper-gate" (notice, code, created_by_id)
               VALUES ($1, $2, 1) RETURNING id"#,
        )
        .bind(code)
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap(),
    };
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(op_id)
    .bind(template_id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_process_rr_operation
           (id, code, ref_left, ref_right, comments, "next-ops", created_by_id)
           VALUES (isahl.gen_next_uid(791), $1, $2, $3, $4, '[]'::jsonb, 1)"#,
    )
    .bind(code)
    .bind(flow_id)
    .bind(op_id)
    .bind(node_type)
    .execute(pool)
    .await
    .unwrap();
    op_id
}

/// 中间审批节点（新形态）：add_node + wire（岗位桥/签署模式），返回 operation id。
async fn add_approve_node(
    pool: &PgPool,
    flow_id: i64,
    code: &str,
    _label: &str,
    assignees: &[i64],
    sign_mode: &str,
) -> i64 {
    let op = add_node(pool, flow_id, code, "approve").await;
    wire_approval_node(pool, op, assignees, sign_mode)
        .await
        .unwrap();
    op
}

/// 审批 HTTP 调用（scope /test，用户上下文注入）
async fn call_approve(
    pool: &PgPool,
    actor: i64,
    instance_id: i64,
) -> actix_web::dev::ServiceResponse {
    let ctx = ::common::context::RequestContext::with_username(actor, "actor@test", "actor");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(std::sync::Arc::new(
                ::common::event_bus::InMemoryEventBus::new(),
            )
                as std::sync::Arc<dyn ::common::event_bus::DomainEventBus>))
            .service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::approve_reject::register),
            ),
    )
    .await;
    let req = test::TestRequest::post()
        .uri(&format!("/test/approval-instances/{}/approve", instance_id))
        .set_json(json!({"opinion": "同意"}))
        .to_request();
    test::call_service(&app, req).await
}

/// 实例的当前桥状态 code
async fn instance_status(pool: &PgPool, instance_id: i64) -> Option<String> {
    sqlx::query_scalar(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" ls
           JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
           WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL
           ORDER BY ls.created_at DESC LIMIT 1"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

/// 节点的实例列表：(id, fk_operator, fk_previous)
async fn node_instances(pool: &PgPool, node_id: i64) -> Vec<(i64, Option<i64>, Option<i64>)> {
    // 排除操作叶行：节点模型接线后 operation 定义与 instance 同表
    // zc_id_oper-approve，且引擎为每个实例也写 rr_event 桥（实例即操作）。
    // 判别：实例 INSERT 绑定 tpl_id=event_id（非空）；操作定义行 tpl_id NULL。
    // 实例挂节点事件模板：node_id=operation → 模板桥反查 even-approve 模板
    let template: i64 = sqlx::query_scalar(
        r#"SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
           WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(node_id)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_as(
        r#"SELECT oa.id, oa.fk_operator, oa.fk_previous FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = oa.id AND oe.ref_right = $1
           WHERE oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL
           ORDER BY oa.id"#,
    )
    .bind(template)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// 创建首节点实例（发起人=actor，实体上下文透传；实例挂节点事件模板）
async fn create_first_instance(
    pool: &PgPool,
    node_id: i64,
    actor: i64,
    comments: Option<&str>,
) -> i64 {
    let template: i64 = sqlx::query_scalar(
        r#"SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
           WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(node_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let instance_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (id, notice, code, fk_subject, fk_operator, comments, created_by_id, tpl_id)
           VALUES (isahl.gen_next_zuid(), '发起', 'START', $1, $1, $2, $1, $3)
           RETURNING id"#,
    )
    .bind(actor)
    .bind(comments)
    .bind(node_id)
    .fetch_one(pool)
    .await
    .unwrap();
    // fk_approve 列已移除：实例↔审批事件经 rr_event 桥
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(instance_id)
    .bind(template)
    .execute(pool)
    .await
    .unwrap();
    instance_id
}

const APP1: i64 = 2001;
const M1: i64 = 2002;
const M2: i64 = 2003;
const M3: i64 = 2004;

#[tokio::test]
async fn role_resolves_operator_and_advances_chain() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    ensure_role_member(&pool, "r_role_a", M1).await.unwrap();
    grant_user_access(&pool, APP1, "approval-instances", &["approve"])
        .await
        .unwrap();

    let (flow, ids) = make_flow(
        &pool,
        "FLOW-ASSIGN-A",
        &[NodeSpec {
            code: "N1",
            node_type: "approve",
            next: vec![],
        }],
    )
    .await;
    let n2 = add_approve_node(&pool, flow, "N2", "岗位节点", &[M1], "sequential").await;
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow)
    .bind(json!([n2]))
    .bind(ids[0])
    .execute(&pool)
    .await
    .unwrap();

    let inst1 = create_first_instance(&pool, ids[0], APP1, Some(r#"{"entityType":"x"}"#)).await;
    let resp = call_approve(&pool, APP1, inst1).await;
    assert!(
        resp.status().is_success(),
        "approve 失败: {:?}",
        resp.status()
    );

    let n2_insts = node_instances(&pool, n2).await;
    assert_eq!(n2_insts.len(), 1);
    assert_eq!(n2_insts[0].1, Some(M1), "fk_operator 必须解析为岗位成员");
    let fk_subject: Option<i64> =
        sqlx::query_scalar("SELECT fk_subject FROM isahl.\"zc_id_oper-approve\" WHERE id = $1")
            .bind(n2_insts[0].0)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(fk_subject, Some(APP1), "fk_subject 必须透传发起人");
    assert_eq!(n2_insts[0].2, Some(inst1), "fk_previous 必须串联源实例");
}

#[tokio::test]
async fn and_sign_waits_for_all_members() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    ensure_role_member(&pool, "r_role_b", M2).await.unwrap();
    ensure_role_member(&pool, "r_role_b", M3).await.unwrap();
    grant_user_access(&pool, APP1, "approval-instances", &["approve"])
        .await
        .unwrap();
    grant_user_access(&pool, M2, "approval-instances", &["approve"])
        .await
        .unwrap();
    grant_user_access(&pool, M3, "approval-instances", &["approve"])
        .await
        .unwrap();

    let (flow, ids) = make_flow(
        &pool,
        "FLOW-AND-SIGN",
        &[NodeSpec {
            code: "N1",
            node_type: "approve",
            next: vec![],
        }],
    )
    .await;
    let n2 = add_approve_node(&pool, flow, "N2", "会签节点", &[M2, M3], "and_sign").await;
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow)
    .bind(json!([n2]))
    .bind(ids[0])
    .execute(&pool)
    .await
    .unwrap();
    let n3 = add_approve_node(&pool, flow, "N3", "末节点", &[], "sequential").await;
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow)
    .bind(json!([n3]))
    .bind(n2)
    .execute(&pool)
    .await
    .unwrap();

    let inst1 = create_first_instance(&pool, ids[0], APP1, Some(r#"{"entityType":"x"}"#)).await;
    let resp = call_approve(&pool, APP1, inst1).await;
    assert!(resp.status().is_success());

    // 会签：全员两实例
    let n2_insts = node_instances(&pool, n2).await;
    assert_eq!(n2_insts.len(), 2, "and_sign 必须为全员建实例");
    assert!(n2_insts.iter().any(|x| x.1 == Some(M2)));
    assert!(n2_insts.iter().any(|x| x.1 == Some(M3)));

    // 首人通过 → 不推进
    let m2_inst = n2_insts.iter().find(|x| x.1 == Some(M2)).unwrap().0;
    let resp = call_approve(&pool, M2, m2_inst).await;
    assert!(resp.status().is_success());
    assert_eq!(node_instances(&pool, n3).await.len(), 0, "会签未齐不得推进");
    assert_eq!(
        instance_status(&pool, m2_inst).await.as_deref(),
        Some("approved")
    );

    // 末人通过 → 推进
    let m3_inst = n2_insts.iter().find(|x| x.1 == Some(M3)).unwrap().0;
    let resp = call_approve(&pool, M3, m3_inst).await;
    assert!(resp.status().is_success());
    assert_eq!(node_instances(&pool, n3).await.len(), 1, "会签齐后必须推进");
}

#[tokio::test]
async fn and_sign_reject_cancels_siblings() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    ensure_role_member(&pool, "r_role_b2", M2).await.unwrap();
    ensure_role_member(&pool, "r_role_b2", M3).await.unwrap();
    grant_user_access(&pool, APP1, "approval-instances", &["approve"])
        .await
        .unwrap();
    grant_user_access(&pool, M2, "approval-instances", &["approve", "reject"])
        .await
        .unwrap();
    grant_user_access(&pool, M3, "approval-instances", &["approve", "reject"])
        .await
        .unwrap();

    let (flow, ids) = make_flow(
        &pool,
        "FLOW-AND-REJ",
        &[NodeSpec {
            code: "N1",
            node_type: "approve",
            next: vec![],
        }],
    )
    .await;
    let n2 = add_approve_node(&pool, flow, "N2", "会签节点2", &[M2, M3], "and_sign").await;
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow)
    .bind(json!([n2]))
    .bind(ids[0])
    .execute(&pool)
    .await
    .unwrap();

    let inst1 = create_first_instance(&pool, ids[0], APP1, Some(r#"{"entityType":"x"}"#)).await;
    call_approve(&pool, APP1, inst1).await;
    let n2_insts = node_instances(&pool, n2).await;
    assert_eq!(n2_insts.len(), 2);

    // M2 驳回 → M3 兄弟 cancelled
    let m2_inst = n2_insts.iter().find(|x| x.1 == Some(M2)).unwrap().0;
    let m3_inst = n2_insts.iter().find(|x| x.1 == Some(M3)).unwrap().0;
    let ctx = ::common::context::RequestContext::with_username(M2, "actor@test", "actor");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(std::sync::Arc::new(
                ::common::event_bus::InMemoryEventBus::new(),
            )
                as std::sync::Arc<dyn ::common::event_bus::DomainEventBus>))
            .service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::approve_reject::register),
            ),
    )
    .await;
    let req = test::TestRequest::post()
        .uri(&format!("/test/approval-instances/{}/reject", m2_inst))
        .set_json(json!({"opinion": "不同意"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    assert_eq!(
        instance_status(&pool, m2_inst).await.as_deref(),
        Some("rejected")
    );
    assert_eq!(
        instance_status(&pool, m3_inst).await.as_deref(),
        Some("cancelled"),
        "会签驳回必须取消兄弟实例"
    );
}

#[tokio::test]
async fn or_sign_first_approve_cancels_siblings() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    ensure_role_member(&pool, "r_role_c", M2).await.unwrap();
    ensure_role_member(&pool, "r_role_c", M3).await.unwrap();
    grant_user_access(&pool, APP1, "approval-instances", &["approve"])
        .await
        .unwrap();
    grant_user_access(&pool, M2, "approval-instances", &["approve"])
        .await
        .unwrap();

    let (flow, ids) = make_flow(
        &pool,
        "FLOW-OR-SIGN",
        &[NodeSpec {
            code: "N1",
            node_type: "approve",
            next: vec![],
        }],
    )
    .await;
    let n2 = add_approve_node(&pool, flow, "N2", "或签节点", &[M2, M3], "or_sign").await;
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow)
    .bind(json!([n2]))
    .bind(ids[0])
    .execute(&pool)
    .await
    .unwrap();
    let n3 = add_approve_node(&pool, flow, "N3", "末节点", &[], "sequential").await;
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow)
    .bind(json!([n3]))
    .bind(n2)
    .execute(&pool)
    .await
    .unwrap();

    let inst1 = create_first_instance(&pool, ids[0], APP1, Some(r#"{"entityType":"x"}"#)).await;
    call_approve(&pool, APP1, inst1).await;
    let n2_insts = node_instances(&pool, n2).await;
    assert_eq!(n2_insts.len(), 2);

    let m2_inst = n2_insts.iter().find(|x| x.1 == Some(M2)).unwrap().0;
    let m3_inst = n2_insts.iter().find(|x| x.1 == Some(M3)).unwrap().0;
    let resp = call_approve(&pool, M2, m2_inst).await;
    assert!(resp.status().is_success());
    assert_eq!(
        node_instances(&pool, n3).await.len(),
        1,
        "或签首个通过必须推进"
    );
    assert_eq!(
        instance_status(&pool, m3_inst).await.as_deref(),
        Some("cancelled"),
        "或签落选兄弟必须取消"
    );
}

#[tokio::test]
async fn sequential_chains_remaining_members() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    ensure_role_member(&pool, "r_role_d", M2).await.unwrap();
    ensure_role_member(&pool, "r_role_d", M3).await.unwrap();
    grant_user_access(&pool, APP1, "approval-instances", &["approve"])
        .await
        .unwrap();
    grant_user_access(&pool, M2, "approval-instances", &["approve"])
        .await
        .unwrap();
    grant_user_access(&pool, M3, "approval-instances", &["approve"])
        .await
        .unwrap();

    let (flow, ids) = make_flow(
        &pool,
        "FLOW-SEQ",
        &[NodeSpec {
            code: "N1",
            node_type: "approve",
            next: vec![],
        }],
    )
    .await;
    let n2 = add_approve_node(&pool, flow, "N2", "依次节点", &[M2, M3], "sequential").await;
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow)
    .bind(json!([n2]))
    .bind(ids[0])
    .execute(&pool)
    .await
    .unwrap();
    let n3 = add_approve_node(&pool, flow, "N3", "末节点", &[], "sequential").await;
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow)
    .bind(json!([n3]))
    .bind(n2)
    .execute(&pool)
    .await
    .unwrap();

    let inst1 = create_first_instance(&pool, ids[0], APP1, Some(r#"{"entityType":"x"}"#)).await;
    call_approve(&pool, APP1, inst1).await;

    // 第一轮：仅建 M2 实例
    let n2_insts = node_instances(&pool, n2).await;
    assert_eq!(n2_insts.len(), 1, "sequential 首轮只建首位成员");
    assert_eq!(n2_insts[0].1, Some(M2));
    assert_eq!(n2_insts[0].2, Some(inst1));

    // M2 通过 → 建 M3 实例（同节点），不推进
    let m2_inst = n2_insts[0].0;
    let resp = call_approve(&pool, M2, m2_inst).await;
    assert!(resp.status().is_success());
    let n2_insts = node_instances(&pool, n2).await;
    assert_eq!(n2_insts.len(), 2, "sequential 第二轮建第二位成员");
    let m3_inst = n2_insts.iter().find(|x| x.1 == Some(M3)).unwrap();
    assert_eq!(m3_inst.2, Some(m2_inst), "依次链 fk_previous 必须串联");
    assert_eq!(node_instances(&pool, n3).await.len(), 0, "依次未完不得推进");

    // M3 通过 → 推进
    let resp = call_approve(&pool, M3, m3_inst.0).await;
    assert!(resp.status().is_success());
    assert_eq!(node_instances(&pool, n3).await.len(), 1, "依次完成必须推进");
}

#[tokio::test]
async fn unassigned_role_keeps_operator_null() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    grant_user_access(&pool, APP1, "approval-instances", &["approve"])
        .await
        .unwrap();

    let (flow, ids) = make_flow(
        &pool,
        "FLOW-NULL-ROLE",
        &[NodeSpec {
            code: "N1",
            node_type: "approve",
            next: vec![],
        }],
    )
    .await;
    let n2 = add_approve_node(&pool, flow, "N2", "空岗位节点", &[], "sequential").await;
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow)
    .bind(json!([n2]))
    .bind(ids[0])
    .execute(&pool)
    .await
    .unwrap();

    let inst1 = create_first_instance(&pool, ids[0], APP1, Some(r#"{"entityType":"x"}"#)).await;
    call_approve(&pool, APP1, inst1).await;
    let n2_insts = node_instances(&pool, n2).await;
    assert_eq!(n2_insts.len(), 1);
    assert_eq!(
        n2_insts[0].1, None,
        "空岗位必须 fk_operator NULL，绝不回退自审"
    );
}

#[tokio::test]
async fn condition_edges_select_branches() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    grant_user_access(&pool, APP1, "approval-instances", &["approve"])
        .await
        .unwrap();

    // 流：N1 → [N2(cond amount>100), N3(默认)]
    let (flow, ids) = make_flow(
        &pool,
        "FLOW-COND",
        &[NodeSpec {
            code: "N1",
            node_type: "approve",
            next: vec![],
        }],
    )
    .await;
    let n2 = add_approve_node(&pool, flow, "N2", "大额分支", &[], "sequential").await;
    let n3 = add_approve_node(&pool, flow, "N3", "默认分支", &[], "sequential").await;
    // 表达式契约（remove-comments-json-embedding 后）：ctx 仅含 entityId
    // = 当前审批节点事件 id（非实例 id——同一节点多实例共享）。条件边按
    // entityId 与节点事件比对：先置恒不命中（entityId == 0）→ 默认分支；
    // 再改为命中当前节点 → 大额分支。
    let inst1 = create_first_instance(&pool, ids[0], APP1, Some(r#"{"entityType":"x"}"#)).await;
    let inst2 = create_first_instance(&pool, ids[0], APP1, Some(r#"{"entityType":"x"}"#)).await;

    // N1 出边：N2(cond) + N3(默认) —— 对象项格式（publish 新批）
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow)
    .bind(json!([{"id": n2, "cond": "entityId == 0"}, n3]))
    .bind(ids[0])
    .execute(&pool)
    .await
    .unwrap();

    // 不命中 → 默认分支
    call_approve(&pool, APP1, inst1).await;
    assert_eq!(
        node_instances(&pool, n2).await.len(),
        0,
        "未命中不得走大额分支"
    );
    assert_eq!(
        node_instances(&pool, n3).await.len(),
        1,
        "未命中必须走默认分支"
    );

    // 命中（entityId == 当前节点事件）→ 大额分支（新实例）
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow)
    .bind(json!([{"id": n2, "cond": format!("entityId == {}", ids[0])}, n3]))
    .bind(ids[0])
    .execute(&pool)
    .await
    .unwrap();
    call_approve(&pool, APP1, inst2).await;
    assert_eq!(
        node_instances(&pool, n2).await.len(),
        1,
        "命中必须走大额分支"
    );
    assert_eq!(
        node_instances(&pool, n3).await.len(),
        1,
        "默认分支不重复创建"
    );
}

#[tokio::test]
async fn condition_undefined_ident_blocks_advance() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    grant_user_access(&pool, APP1, "approval-instances", &["approve"])
        .await
        .unwrap();

    // 流：N1 → [N2(cond nope>1)] 唯一出边且条件引用了未定义变量
    let (flow, ids) = make_flow(
        &pool,
        "FLOW-COND-BLOCK",
        &[NodeSpec {
            code: "N1",
            node_type: "approve",
            next: vec![],
        }],
    )
    .await;
    let n2 = add_approve_node(&pool, flow, "N2", "未定义条件", &[], "sequential").await;
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow)
    .bind(json!([{"id": n2, "cond": "nope > 1"}]))
    .bind(ids[0])
    .execute(&pool)
    .await
    .unwrap();

    let inst1 = create_first_instance(&pool, ids[0], APP1, Some(r#"{"amount": 50}"#)).await;
    call_approve(&pool, APP1, inst1).await;
    assert_eq!(
        node_instances(&pool, n2).await.len(),
        0,
        "未定义标识符必须 fail-closed 阻断推进"
    );
}

#[tokio::test]
async fn condition_node_expr_error_returns_structured_error() {
    // P2：condition 节点 expr 求值错误必须显式报出（前端可识别并回灌 chat-ai 自愈），
    // 而非静默阻断——fail-closed + 错误结构化
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    grant_user_access(&pool, APP1, "approval-instances", &["approve"])
        .await
        .unwrap();

    // 流：N1 → [N2(condition 节点, expr 引用未定义变量)]
    let (flow, ids) = make_flow(
        &pool,
        "FLOW-COND-ERR",
        &[NodeSpec {
            code: "N1",
            node_type: "approve",
            next: vec![],
        }],
    )
    .await;
    let n2 = add_node(&pool, flow, "N2", "condition").await;
    // 节点类型分类（advance_auto_node 按 ck_cate-proc_op='condition' 进入条件分支）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_cate-proc_op" (id, notice, code, enable, created_by_id)
           SELECT isahl.gen_next_uid(360), '条件','condition',true,1
           WHERE NOT EXISTS(SELECT 1 FROM isahl."zc_id_cate-proc_op" WHERE code='condition' AND deleted_at IS NULL)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"UPDATE isahl."zc_id_operation" SET "ck_cate-proc_op" =
             (SELECT id FROM isahl."zc_id_cate-proc_op" WHERE code='condition' AND deleted_at IS NULL)
           WHERE id = $1"#,
    )
    .bind(n2)
    .execute(&pool)
    .await
    .unwrap();
    // 出边
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow)
    .bind(json!([n2]))
    .bind(ids[0])
    .execute(&pool)
    .await
    .unwrap();
    // meta：N2 节点 expr（advance_auto_node 经 rr_operation.code=N2 反查）
    sqlx::query(r#"UPDATE isahl.zc_id_process SET meta = $2::jsonb WHERE id = $1"#)
        .bind(flow)
        .bind(json!({"nodes": [{"id": "N2", "type": "condition", "expr": "nope > 1"}]}))
        .execute(&pool)
        .await
        .unwrap();

    let inst1 = create_first_instance(&pool, ids[0], APP1, Some(r#"{"amount": 50}"#)).await;
    let resp = call_approve(&pool, APP1, inst1).await;
    assert!(
        !resp.status().is_success(),
        "condition 节点表达式求值错误必须显式失败（P2 结构化）"
    );
    assert_eq!(
        node_instances(&pool, n2).await.len(),
        0,
        "fail-closed：不创建实例"
    );
}
