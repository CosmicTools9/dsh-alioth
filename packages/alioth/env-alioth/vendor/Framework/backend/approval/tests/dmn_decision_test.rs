//! DMN 决策表集成测试（fix-flow-designer-editing-gaps B3）
//!
//! 守护可观察行为：
//! 1. 发布物化：decision 节点 timeline.dmn 落库、出边 label 进 next-ops；
//! 2. FIRST 策略实体上下文路由：规则 `code == 'C1'` 命中 → 沿 label=go-a 边推进，
//!    go-b 分支零实例；
//! 3. UNIQUE 多命中：发布通过（结构合法）、运行时推进 fail-closed Validation 阻断
//!    （错误含命中数与输出集），双分支均无实例；
//! 4. 空规则发布：publish 400（fail-closed 结构校验）。

use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};

mod common;
use common::setup_test_schema;

const USER_ID: i64 = 443901;

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

async fn test_user(pool: &sqlx::PgPool) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'dmn-test', 'dmn-test', 'dmn@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(USER_ID)
    .execute(pool)
    .await
    .unwrap();
}

/// 种子 task 域范畴定义 + 实体（code=C1 供规则求值）
async fn seed_task_commission(pool: &sqlx::PgPool) -> (i64, i64) {
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("JE", "FMA", "↓_CH"))
        .await
        .unwrap();
    let scope_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, _t_, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), 'DMN-委派定义', 'scope-definition', 1, $1, $2, $3)
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
    let e1: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-commission" (id, notice, code, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), '委派实体一', 'C1', 1, '实现', '范例', $1, $2, $3) RETURNING id"#,
    )
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .unwrap();
    (scope_id, e1)
}

async fn create_flow(pool: &sqlx::PgPool, name: &str, ctx_id: i64, graph: &Value) -> i64 {
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool.clone(), ("JC", "FTA", "↑_NA"))
            .await
            .unwrap();
    let flow_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve" (notice, meta, code, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2::jsonb, 'draft', 1, '实现', '范例', $3, $4, $5) RETURNING id"#,
    )
    .bind(name)
    .bind(graph.to_string())
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .unwrap();
    // 输入范畴经 zc_id_process_rr_context 桥落行（物理列已移除）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_process_rr_context"
           (ref_left, ref_right, code, notice, created_by_id)
           VALUES ($1, $2, 'bind-context', '流程上下文绑定', 1)"#,
    )
    .bind(flow_id)
    .bind(ctx_id)
    .execute(pool)
    .await
    .unwrap();
    flow_id
}

/// 按 rro.code（图内编号）统计该模板节点的审批实例数（实例经 tpl_id 挂模板 op）
async fn instance_count(pool: &sqlx::PgPool, flow_id: i64, graph_code: &str) -> i64 {
    sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oa.tpl_id AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = $2
             AND oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL"#,
    )
    .bind(flow_id)
    .bind(graph_code)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn decision_graph(policy: &str, rules: Value) -> Value {
    json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "n-decision", "type": "decision", "label": "分级决策",
             "dmn": {"hitPolicy": policy, "inputs": [{"name": "code"}], "rules": rules},
             "next": [{"to": 2, "label": "go-a"}, {"to": 3, "label": "go-b"}]},
            {"id": "n-high", "type": "approve", "label": "高位审批", "next": [{"to": 4}]},
            {"id": "n-low", "type": "approve", "label": "常规审批", "next": [{"to": 4}]},
            {"id": "n-end", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    })
}
macro_rules! build_app {
    ($pool:expr) => {{
        let ctx =
            ::common::context::RequestContext::with_username(USER_ID, "dmn@test.local", "dmn-test");
        test::init_service(
            App::new().app_data(web::Data::new($pool.clone())).service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::publish::register)
                    .configure(handlers::initiate::register),
            ),
        )
        .await
    }};
}

#[tokio::test]
async fn first_policy_routes_by_entity_context() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let (scope_id, e1) = seed_task_commission(&pool).await;

    let rules = json!([
        {"match": ["code == 'C1'"], "output": "go-a"},
        {"match": [null], "output": "go-b"}
    ]);
    let graph = decision_graph("FIRST", rules);
    let flow_id = create_flow(&pool, "DMN-FIRST 路由", scope_id, &graph).await;
    let app = build_app!(&pool);

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200: {b}");

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": e1})
    );
    assert_eq!(s, 200, "initiate 应 200: {b}");

    // 实体 code=C1 → 规则 1 命中输出 go-a → 沿 label=go-a 边推进：
    // n-high（高位审批）有实例，n-low（go-b）零实例
    let high = instance_count(&pool, flow_id, "n-high").await;
    let low = instance_count(&pool, flow_id, "n-low").await;
    assert!(high >= 1, "go-a 分支应有审批实例（实际 {high}）");
    assert_eq!(low, 0, "go-b 分支不应有实例（实际 {low}）");

    // 扇出抵达时间：命中分支的目标 operation 被盖 qk_arrived（→ zc_id_scal-date）
    // 且解析出的时刻可读——运行时上下文 `arrived_at` 的取值来源。
    let (arrived_id, arrived_at): (Option<i64>, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as(
            r#"SELECT o.qk_arrived, sd.date
               FROM isahl.zc_id_process_rr_operation rro
               JOIN isahl.zc_id_operation o ON o.id = rro.ref_right AND o.deleted_at IS NULL
               LEFT JOIN isahl."zc_id_scal-date" sd ON sd.id = o.qk_arrived
               WHERE rro.ref_left = $1 AND rro.code = $2 AND rro.deleted_at IS NULL
               LIMIT 1"#,
        )
        .bind(flow_id)
        .bind("n-high")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        arrived_id.is_some(),
        "命中分支的目标 operation 应有 qk_arrived"
    );
    assert!(
        arrived_at.is_some(),
        "qk_arrived 应解析出抵达时刻（scal-date.date）"
    );
}

#[tokio::test]
async fn unique_multi_match_blocks_fail_closed() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let (scope_id, e1) = seed_task_commission(&pool).await;

    // 两行恒命中（第一行 code==C1 真 + 第二行通配）→ UNIQUE 违例
    let rules = json!([
        {"match": ["code == 'C1'"], "output": "go-a"},
        {"match": [null], "output": "go-b"}
    ]);
    let graph = decision_graph("UNIQUE", rules);
    let flow_id = create_flow(&pool, "DMN-UNIQUE 违例", scope_id, &graph).await;
    let app = build_app!(&pool);

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "结构合法应可发布: {b}");

    // 运行时推进 fail-closed：initiate 返回非 2xx，双分支零实例
    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": e1})
    );
    assert_ne!(s, 200, "UNIQUE 多命中应 fail-closed 阻断: {b}");
    let msg = b.to_string();
    assert!(
        msg.contains("UNIQUE") || msg.contains("命中"),
        "错误应含违例详情: {msg}"
    );
    assert_eq!(instance_count(&pool, flow_id, "n-high").await, 0);
    assert_eq!(instance_count(&pool, flow_id, "n-low").await, 0);
}

#[tokio::test]
async fn empty_rules_rejected_at_publish() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let (scope_id, _e1) = seed_task_commission(&pool).await;

    let graph = decision_graph("FIRST", json!([]));
    let flow_id = create_flow(&pool, "DMN-空表拒绝", scope_id, &graph).await;
    let app = build_app!(&pool);

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 400, "零规则决策表应 400 拒绝: {b}");
    assert!(b.to_string().contains("≥1"), "错误应指向规则数量: {b}");
}

#[tokio::test]
async fn output_without_matching_edge_label_rejected_at_publish() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let (scope_id, _e1) = seed_task_commission(&pool).await;

    // 输出 go-c 无匹配 label 出边，且两条出边均带 label（无兜底边）→ 发布后必然停滞
    let mut graph = decision_graph("FIRST", json!([{"match": [null], "output": "go-c"}]));
    graph["nodes"][1]["next"] = json!([
        {"to": 2, "label": "go-a"},
        {"to": 3, "label": "go-b"}
    ]);
    let flow_id = create_flow(&pool, "DMN-输出无边拒绝", scope_id, &graph).await;
    let app = build_app!(&pool);

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 400, "输出无匹配 label 出边应 400 拒绝: {b}");
    assert!(b.to_string().contains("go-c"), "错误应点名输出值: {b}");
}

// ── 扩展策略集成（extend-dmn-decision-table-full）──
#[tokio::test]
async fn priority_policy_routes_lowest_priority_hit() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let (scope_id, e1) = seed_task_commission(&pool).await;

    // 高金额命中 big（priority 9）+ 通配兜底 small（priority 1）→ small 胜出 → go-b
    let graph = decision_graph(
        "PRIORITY",
        json!([
            {"match": ["code == 'C1'"], "output": "go-a", "priority": 9},
            {"match": [null], "output": "go-b", "priority": 1}
        ]),
    );
    let flow_id = create_flow(&pool, "DMN-PRIORITY 路由", scope_id, &graph).await;
    let app = build_app!(&pool);

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200: {b}");

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": e1})
    );
    assert_eq!(s, 200, "initiate 应 200: {b}");

    // priority 1 的 go-b 胜出（行序反例——显式优先级压过行序）
    assert_eq!(
        instance_count(&pool, flow_id, "n-high").await,
        0,
        "go-a 不应有实例"
    );
    assert!(
        instance_count(&pool, flow_id, "n-low").await >= 1,
        "go-b 应有实例"
    );
}

#[tokio::test]
async fn priority_same_priority_multi_hit_blocks_fail_closed() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let (scope_id, e1) = seed_task_commission(&pool).await;

    // 两行同 priority 5 且输出不同（code==C1 真 + 通配）→ 违例阻断
    let graph = decision_graph(
        "PRIORITY",
        json!([
            {"match": ["code == 'C1'"], "output": "go-a", "priority": 5},
            {"match": [null], "output": "go-b", "priority": 5}
        ]),
    );
    let flow_id = create_flow(&pool, "DMN-PRIORITY 同优违例", scope_id, &graph).await;
    let app = build_app!(&pool);

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "结构合法应可发布: {b}");

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": e1})
    );
    assert_ne!(s, 200, "PRIORITY 同优多命中应 fail-closed: {b}");
    assert!(
        b.to_string().contains("PRIORITY"),
        "错误应含 PRIORITY 详情: {b}"
    );
    assert_eq!(instance_count(&pool, flow_id, "n-high").await, 0);
    assert_eq!(instance_count(&pool, flow_id, "n-low").await, 0);
}

#[tokio::test]
async fn collect_list_fans_out_both_branches_in_parallel() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let (scope_id, e1) = seed_task_commission(&pool).await;

    // COLLECT-list：通配规则输出 go-a（规则1）+ go-b（规则2）双命中 → 并行扇出双分支
    let graph = decision_graph(
        "COLLECT",
        json!([
            {"match": [null], "output": "go-a"},
            {"match": [null], "output": "go-b"}
        ]),
    );
    let flow_id = create_flow(&pool, "DMN-COLLECT 并行", scope_id, &graph).await;
    let app = build_app!(&pool);

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200: {b}");

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": e1})
    );
    assert_eq!(s, 200, "initiate 应 200: {b}");

    // 双分支并行：go-a 与 go-b 均有实例
    assert!(
        instance_count(&pool, flow_id, "n-high").await >= 1,
        "go-a 应有实例"
    );
    assert!(
        instance_count(&pool, flow_id, "n-low").await >= 1,
        "go-b 应有实例"
    );
}

#[tokio::test]
async fn multi_output_column_length_mismatch_rejected_at_publish() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let (scope_id, _e1) = seed_task_commission(&pool).await;

    // outputs 两列但规则 output 仅一值 → 400（伴随列长度违例）
    let mut graph = decision_graph("FIRST", json!([{"match": [null], "output": ["go-a"]}]));
    graph["nodes"][1]["dmn"]["outputs"] = json!([{"name": "route"}, {"name": "band"}]);
    let flow_id = create_flow(&pool, "DMN-多输出列长违例", scope_id, &graph).await;
    let app = build_app!(&pool);

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 400, "outputs 两列但单值 output 应 400: {b}");
    assert!(
        b.to_string().contains("输出列数"),
        "错误应指向输出列数: {b}"
    );
}

#[tokio::test]
async fn collect_non_numeric_sum_rejected_fail_closed() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let (scope_id, e1) = seed_task_commission(&pool).await;

    // aggregation=sum 但输出含非数值 → publish 结构合法（数值性属求值期），
    // 运行时推进 fail-closed 阻断（与 dmn.rs Violation 镜像）
    let mut graph = decision_graph(
        "COLLECT",
        json!([
            {"match": [null], "output": "go-a"},
            {"match": [null], "output": "go-b"}
        ]),
    );
    graph["nodes"][1]["dmn"]["aggregation"] = json!("sum");
    let flow_id = create_flow(&pool, "DMN-COLLECT sum 非数值", scope_id, &graph).await;
    let app = build_app!(&pool);

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "sum 聚合结构合法应可发布（数值性求值期判定）: {b}");

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/initiate"),
        json!({"entity_table": "zc_id_task-commission", "entity_id": e1})
    );
    assert_ne!(s, 200, "sum 聚合遇非数值输出应 fail-closed: {b}");
    assert!(b.to_string().contains("非数值"), "错误应含非数值详情: {b}");
}

/// 载体行 comments 读出（图内节点 code → rro → op 行 → rr_event 桥 → even 模板行）
async fn even_comments(pool: &sqlx::PgPool, flow_id: i64, code: &str) -> Option<String> {
    sqlx::query_scalar(
        r#"SELECT e.comments FROM isahl."zc_id_even-approve" e
           JOIN isahl.zc_id_operation_rr_event oe
             ON oe.ref_right = e.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = $2
             AND e.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(code)
    .fetch_optional(pool)
    .await
    .unwrap()
    .flatten()
}

#[tokio::test]
async fn decision_description_writes_even_comments() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let (scope_id, _e1) = seed_task_commission(&pool).await;

    let mut graph = decision_graph(
        "FIRST",
        json!([
            {"match": ["code == 'C1'"], "output": "go-a"},
            {"match": [null], "output": "go-b"}
        ]),
    );
    graph["nodes"][1]["dmn"]["description"] =
        json!("金额≥10000 且客户等级 A → 大额审批；其余 → 普通审批");
    let flow_id = create_flow(&pool, "DMN-决策说明落 comments", scope_id, &graph).await;
    let app = build_app!(&pool);

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200: {b}");

    let desc = even_comments(&pool, flow_id, "n-decision").await;
    assert_eq!(
        desc.as_deref(),
        Some("金额≥10000 且客户等级 A → 大额审批；其余 → 普通审批"),
        "决策说明应落载体行 comments"
    );
    // 非 decision 节点 comments 不受影响（仍 NULL）
    assert_eq!(
        even_comments(&pool, flow_id, "n-start").await,
        None,
        "start 节点 comments 应保持 NULL"
    );
}

#[tokio::test]
async fn decision_without_description_keeps_null_comments() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let (scope_id, _e1) = seed_task_commission(&pool).await;

    let graph = decision_graph(
        "FIRST",
        json!([
            {"match": ["code == 'C1'"], "output": "go-a"},
            {"match": [null], "output": "go-b"}
        ]),
    );
    let flow_id = create_flow(&pool, "DMN-无说明 NULL", scope_id, &graph).await;
    let app = build_app!(&pool);

    let (s, b) = post_json!(
        &app,
        &format!("/test/approval-flows/{flow_id}/publish"),
        json!({})
    );
    assert_eq!(s, 200, "publish 应 200: {b}");

    assert_eq!(
        even_comments(&pool, flow_id, "n-decision").await,
        None,
        "description 缺省时 comments 应保持 NULL"
    );
}
