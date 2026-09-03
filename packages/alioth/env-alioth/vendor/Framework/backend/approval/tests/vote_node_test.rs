//! vote 节点集成测试（2026-09-02 add-vote-flow-node）：
//! - publish：cate code 'vote' + 载体 timeline.quorum 物化 + rr_approve 岗位桥
//! - 运行时：全员建票（Vote 模式）→ quorum 未达等待 → quorum 达标推进至 end
//!   并物化结论；非法 quorum（负数）发布被拒。

use ::common::event_bus::{DomainEventBus, InMemoryEventBus};
use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
mod common;
use common::{ensure_role_member, grant_user_access, setup_test_schema};

const VOTER_1: i64 = 424601;
const USER_ID: i64 = 424601;

macro_rules! build_app {
    ($pool:expr, $bus:expr) => {{
        let ctx = ::common::context::RequestContext::with_username(
            VOTER_1,
            "vote1@test.local",
            "vote-test",
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

async fn seed_voters(pool: &PgPool, tag: &str) {
    // per-tag 用户对（岗位解析按用户全局——同用户跨 role 的岗位会放大 assignees；
    // 每个 tag 独立用户即天然隔离；VOTER_1 为审批执行上下文用户，与投票人不重叠亦可）
    let base: i64 = match tag {
        "pub" => 424611,
        "gate" => 424621,
        "abs" => 424631,
        "pct" => 424641,
        _ => 424601,
    };
    let (u1, u2) = (base, base + 1);
    let role = format!("vote-{}", tag);
    let code_a = format!("vote-{}-a", tag);
    let code_b = format!("vote-{}-b", tag);
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_attribute (id, o_name, fk_policy_class)
           SELECT isahl.gen_next_zuid(), $1, pc.id FROM isahl_auth.ngac_policy_class pc
           WHERE pc.o_name = 'default'
             AND NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_user_attribute ua
                             WHERE ua.o_name = $1)
           LIMIT 1"#,
    )
    .bind(&role)
    .execute(pool)
    .await
    .unwrap();
    // 成员集合精确化：清空该 role 全部成员再重建（防历史残留放大 assignees）
    sqlx::query(
        r#"DELETE FROM isahl_auth.ngac_user_rr_attribute rel
           USING isahl_auth.ngac_user_attribute ua
           WHERE rel.fk_user_attribute = ua.id AND ua.o_name = $1
             AND rel.deleted_at IS NULL"#,
    )
    .bind(&role)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_subj-position"
           WHERE notice = $1 OR notice = $2"#,
    )
    .bind(&code_a)
    .bind(&code_b)
    .execute(pool)
    .await
    .unwrap();
    for (uid, code) in [(u1, code_a), (u2, code_b)] {
        ensure_role_member(pool, &role, uid).await.unwrap();
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_subj-position"
               (id, code, notice, fk_user, created_by_id, created_at, updated_at)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3, $3, NOW(), NOW())"#,
        )
        .bind(&code)
        .bind(&code)
        .bind(uid)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// 在 zc_id_task-commission 建范畴定义行（scope-definition）与业务实体行
async fn seed_task_commission(pool: &PgPool) -> (i64, i64) {
    let scope_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, _t_, created_by_id)
           VALUES (isahl.gen_next_zuid(), '投票-任务委派', 'scope-definition', 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .expect("insert scope-def row");
    let entity_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, code, created_by_id)
           VALUES (isahl.gen_next_zuid(), '委派实体', 'VIP-VOTE', 1)
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .expect("insert entity row");
    (scope_id, entity_id)
}

async fn create_flow(pool: &PgPool, name: &str, ctx_id: Option<i64>, graph: &Value) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve" (notice, meta, code, fk_context, created_by_id, _f_, _t_)
           VALUES ($1, $2::jsonb, $3, $4, 1, '实现', '范例') RETURNING id"#,
    )
    .bind(name)
    .bind(graph.to_string())
    .bind(format!("VOTE-{}", name))
    .bind(ctx_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn vote_graph(quorum: i64) -> Value {
    json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "v", "type": "vote", "label": "CCB 投票", "role": "vote-role", "quorum": quorum, "next": [{"to": 2}]},
            {"id": "e", "type": "end", "label": "结束", "outcome": "complete", "statementLeaf": "zc_id_stat-inspection"}
        ]
    })
}

/// 流程结论（end 到达物化）存在性
async fn conclusion_count(pool: &PgPool, flow_id: i64) -> i64 {
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
async fn vote_publish_materializes_cate_quorum_and_bridge() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    seed_voters(&pool, "pub").await;
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, _entity_id) = seed_task_commission(&pool).await;

    let mut g = vote_graph(2);
    g["nodes"][1]["role"] = json!(format!("vote-pub"));
    let flow_id = create_flow(&pool, "Publish", Some(scope_id), &g).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "vote 流程 publish 应 200: {b}");

    // vote 节点 operation 的 cate code = 'vote'
    let cate: Option<String> = sqlx::query_scalar(
        r#"SELECT c.code FROM isahl."zc_id_cate-proc_op" c
           JOIN isahl.zc_id_operation o ON o."ck_cate-proc_op" = c.id AND o.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = o.id AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = 'v' AND c.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(
        cate.as_deref(),
        Some("vote"),
        "vote 节点 cate code 应为 'vote'"
    );

    // 载体 timeline quorum = 2
    let quorum: Option<String> = sqlx::query_scalar(
        r#"SELECT ea.timeline->>'quorum' FROM isahl."zc_id_even-approve" ea
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = 'v' AND ea.deleted_at IS NULL
           LIMIT 1"#,
    )
    .bind(flow_id)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(
        quorum.as_deref(),
        Some("2"),
        "载体 timeline quorum 应物化 2"
    );

    // rr_approve 岗位桥：两位投票人的岗位均接线
    let bridged: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_operation_rr_approve" ra
           JOIN isahl."zc_id_subj-position" pos ON pos.id = ra.ref_right AND pos.deleted_at IS NULL
           JOIN isahl."zc_id_process_rr_operation" rro
             ON rro.ref_right = ra.ref_left AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = 'v' AND ra.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bridged, 2, "两位投票人岗位应全部接线 rr_approve");
}

#[tokio::test]
async fn vote_negative_quorum_rejected_on_publish() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    seed_voters(&pool, "cfg").await;
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, _entity_id) = seed_task_commission(&pool).await;

    let flow_id = create_flow(&pool, "BadQuorum", Some(scope_id), &vote_graph(-1)).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 400, "负数 quorum 发布应被拒: {b}");
}

#[tokio::test]
async fn vote_abstain_triggers_auto_reject_when_quorum_unmet() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    seed_voters(&pool, "abs").await;
    grant_user_access(&pool, VOTER_1, "approval-instances", &["approve"])
        .await
        .unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus.clone());
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    let mut g = vote_graph(2);
    g["nodes"][1]["role"] = json!(format!("vote-abs"));
    let flow_id = create_flow(&pool, "Abstain", Some(scope_id), &g).await;
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
    assert_eq!(s, 200, "initiate 应 200");
    let ids = b["data"]["instance_ids"].as_array().expect("ids");
    assert_eq!(ids.len(), 2);

    // 一票通过 + 一票弃权 → quorum(2) 未达且全员已行动 → 自动 rejected 终局（不推进）
    let (s, _) = post_json!(
        &app,
        &format!(
            "/test/approval-instances/{}/approve",
            ids[0].as_str().unwrap()
        ),
        json!({"opinion": "同意"})
    );
    assert_eq!(s, 200);
    // 订阅在首 approve 后：只捕获 abstain 触发的终局事件
    let mut rx = bus.subscribe("ApprovalCompleted").await.unwrap();
    let (s, b) = post_json!(
        &app,
        &format!(
            "/test/approval-instances/{}/abstain",
            ids[1].as_str().unwrap()
        ),
        json!({"opinion": "无意见"})
    );
    assert_eq!(s, 200, "abstain 应 200: {b}");
    assert_eq!(b["data"]["status"], "abstained");

    // 全员终态未达 quorum → ApprovalCompleted rejected 事件
    let evt = rx.recv().await.expect("rejected 事件");
    assert_eq!(evt.event_type, "ApprovalCompleted");
    assert_eq!(
        evt.payload["result"], "rejected",
        "自动终局事件 result=rejected"
    );
    // 未推进（end 无结论实例）
    assert_eq!(conclusion_count(&pool, flow_id).await, 0);
}

#[tokio::test]
async fn vote_percent_quorum_advances_on_majority() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    seed_voters(&pool, "pct").await;
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    // pct=50，2 票 → 阈值 ceil(2*0.5)=1：首票通过即达标（取消余票并推进）
    let mut g = vote_graph(2);
    g["nodes"][1]["quorum"] = serde_json::Value::Null;
    g["nodes"][1]["quorumPct"] = json!(50);
    g["nodes"][1]["role"] = json!("vote-pct");
    let flow_id = create_flow(&pool, "Pct", Some(scope_id), &g).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200: {b}");

    // quorumPct 物化
    let pct: Option<String> = sqlx::query_scalar(
        r#"SELECT ea.timeline->>'quorumPct' FROM isahl."zc_id_even-approve" ea
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
           JOIN isahl."zc_id_process_rr_operation" rro
             ON rro.ref_right = oe.ref_left AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = 'v' AND ea.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow_id)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    assert_eq!(pct.as_deref(), Some("50"));

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200);
    let ids = b["data"]["instance_ids"].as_array().expect("ids");
    assert_eq!(ids.len(), 2, "pct 模式仍全员建票");
    let (s, _) = post_json!(
        &app,
        &format!(
            "/test/approval-instances/{}/approve",
            ids[0].as_str().unwrap()
        ),
        json!({"opinion": "同意"})
    );
    assert_eq!(s, 200);
    assert_eq!(
        conclusion_count(&pool, flow_id).await,
        1,
        "pct=50 首票即达标推进"
    );
}

#[tokio::test]
async fn vote_quorum_config_mutual_exclusion_and_range() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    seed_voters(&pool, "cfg").await;
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, _e) = seed_task_commission(&pool).await;

    // quorum + quorumPct 并存 → 400
    let mut g = vote_graph(2);
    g["nodes"][1]["quorumPct"] = json!(60);
    let flow_id = create_flow(&pool, "Both", Some(scope_id), &g).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 400, "quorum 与 quorumPct 并存应拒: {b}");

    // quorumPct 越界 → 400
    let mut g2 = vote_graph(2);
    g2["nodes"][1]["quorum"] = serde_json::Value::Null;
    g2["nodes"][1]["quorumPct"] = json!(150);
    let flow2 = create_flow(&pool, "PctBad", Some(scope_id), &g2).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow2}/publish"),
        json!({})
    );
    assert_eq!(s, 400, "quorumPct 越界应拒: {b}");
}

#[tokio::test]
async fn abstain_rejected_on_non_vote_instance() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    grant_user_access(&pool, USER_ID, "approval-instances", &["approve"])
        .await
        .unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_task_commission(&pool).await;
    // 普通 approval 节点（非 vote）
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "s", "type": "start", "label": "开始", "drive": "event", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "a", "type": "approval", "label": "审批", "mode": "or_sign"}
        ]
    });
    let flow_id = create_flow(&pool, "NonVote", Some(scope_id), &graph).await;
    let (s, _) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200);
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200);
    let inst = b["data"]["instance_ids"][0].as_str().unwrap().to_string();
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-instances/{inst}/abstain"),
        json!({"opinion": ""})
    );
    assert_eq!(s, 400, "非 vote 节点弃权应 400: {b}");
}

#[tokio::test]
async fn vote_weighted_sources_quorum_advances_on_weight_sum() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    // 加权专用 tag：userA(role src 'role-wa' 成员, weight 2) + userB(role 'role-wb', weight 1)
    let role_a = "vote-wa";
    let role_b = "vote-wb";
    let (u1, u2) = (424651i64, 424652i64);
    // 幂等：清理前次运行残留（成员/岗位按 tag 隔离）
    sqlx::query(
        r#"DELETE FROM isahl_auth.ngac_user_rr_attribute rel
           USING isahl_auth.ngac_user_attribute ua
           WHERE rel.fk_user_attribute = ua.id AND ua.o_name IN ('vote-wa', 'vote-wb')"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(r#"DELETE FROM isahl."zc_id_subj-position" WHERE fk_user IN (424651, 424652)"#)
        .execute(&pool)
        .await
        .unwrap();
    for (role, uids) in [(role_a, vec![u1]), (role_b, vec![u2])] {
        sqlx::query(
            r#"INSERT INTO isahl_auth.ngac_user_attribute (id, o_name, fk_policy_class)
               SELECT isahl.gen_next_zuid(), $1, pc.id FROM isahl_auth.ngac_policy_class pc
               WHERE pc.o_name = 'default'
                 AND NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_user_attribute ua
                                 WHERE ua.o_name = $1)
               LIMIT 1"#,
        )
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
        for uid in uids {
            ensure_role_member(&pool, role, uid).await.unwrap();
            sqlx::query(
                r#"INSERT INTO isahl."zc_id_subj-position"
                   (id, code, notice, fk_user, created_by_id, created_at, updated_at)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $3, NOW(), NOW())"#,
            )
            .bind(format!("{}-p{}", role, uid))
            .bind(format!("{}-p{}", role, uid))
            .bind(uid)
            .execute(&pool)
            .await
            .unwrap();
        }
    }
    grant_user_access(&pool, USER_ID, "approval-instances", &["approve"])
        .await
        .unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    // 源 A weight 2（1 人），源 B weight 1（1 人）→ 总权重 3；quorum=2（权重和）
    let mut g = vote_graph(2);
    g["nodes"][1]["role"] = serde_json::Value::Null;
    g["nodes"][1]["voteSources"] = json!([
        {"kind": "role", "id": role_a, "weight": 2},
        {"kind": "role", "id": role_b, "weight": 1}
    ]);
    let flow_id = create_flow(&pool, "Weighted", Some(scope_id), &g).await;
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "加权发布: {b}");

    // resolvedWeights 物化
    let rw: Option<Value> = sqlx::query_scalar(
        r#"SELECT ea.timeline->'resolvedWeights' FROM isahl."zc_id_even-approve" ea
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
           JOIN isahl."zc_id_process_rr_operation" rro
             ON rro.ref_right = oe.ref_left AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = 'v' AND ea.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow_id)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    let rw = rw.expect("resolvedWeights");
    assert_eq!(rw.as_array().map(|a| a.len()), Some(2), "两源两用户: {rw}");
    let w1 = rw[0]["weight"].as_i64().unwrap_or(0);
    let w2 = rw[1]["weight"].as_i64().unwrap_or(0);
    assert_eq!(w1 + w2, 3, "权重合计 3");

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": entity_id})
    );
    assert_eq!(s, 200, "initiate: {b}");
    let ids = b["data"]["instance_ids"].as_array().expect("ids");
    assert_eq!(ids.len(), 2, "全员建票");

    // u1 通过（权重 2）→ 达标（quorum=2 权重和）推进；u2 票被取消
    let (s, _) = post_json!(
        &app,
        &format!(
            "/test/approval-instances/{}/approve",
            ids[0].as_str().unwrap()
        ),
        json!({"opinion": "同意"})
    );
    assert_eq!(s, 200);
    assert_eq!(
        conclusion_count(&pool, flow_id).await,
        1,
        "权重 2 ≥ quorum 2：加权用户通过即达标"
    );
}
#[tokio::test]
async fn vote_quorum_gate_advances_after_quorum_met() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    seed_voters(&pool, "gate").await;
    // VOTER_1 以审批人身份执行 approve（NGAC 资源级 approve 授权即可，无 operator 强校验）
    grant_user_access(&pool, VOTER_1, "approval-instances", &["approve"])
        .await
        .unwrap();
    let bus: Arc<dyn DomainEventBus> = Arc::new(InMemoryEventBus::new());
    let app = build_app!(pool, bus);
    let (scope_id, entity_id) = seed_task_commission(&pool).await;

    let mut g = vote_graph(2);
    g["nodes"][1]["role"] = json!(format!("vote-gate"));
    let flow_id = create_flow(&pool, "Runtime", Some(scope_id), &g).await;
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
    let ids = b["data"]["instance_ids"]
        .as_array()
        .expect("instance_ids")
        .iter()
        .map(|v| v.as_str().unwrap().parse::<i64>().unwrap())
        .collect::<Vec<i64>>();
    assert_eq!(ids.len(), 2, "vote 节点应全员建票（2 张）");

    // 第 1 票通过：quorum=2 未达 → 不推进（无结论、第 2 票仍在途）
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-instances/{}/approve", ids[0]),
        json!({"opinion": "同意"})
    );
    assert_eq!(s, 200, "approve 应 200: {b}");
    assert_eq!(
        conclusion_count(&pool, flow_id).await,
        0,
        "quorum 未达不得推进到 end"
    );
    let pending2: bool = sqlx::query_scalar(
        r#"SELECT NOT EXISTS (
             SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
             JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
             WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL
               AND s.code IN ('approved','rejected','withdrawn','cancelled')
           )"#,
    )
    .bind(ids[1])
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(pending2, "第 2 票应保持待办（未被取消）");

    // 第 2 票通过：quorum 达标 → 推进到 end，物化结论
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-instances/{}/approve", ids[1]),
        json!({"opinion": "同意"})
    );
    assert_eq!(s, 200, "approve 应 200: {b}");
    assert_eq!(
        conclusion_count(&pool, flow_id).await,
        1,
        "quorum 达标应推进到 end 并物化结论"
    );
}
