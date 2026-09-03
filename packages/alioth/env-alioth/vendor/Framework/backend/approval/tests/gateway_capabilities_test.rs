//! 流程设计器能力补齐集成测试（2026-09-01）：
//! - subflow：target 存在且已发布 → publish 200；target 不存在/未发布 → 400；
//!   自引用 → 400
//! - loop：publish 200 + timeline 物化 loopExpr/loopMaxIter
//! - branch：joinRule 物化 timeline（all/any）

use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
mod common;
use common::setup_test_schema;

const USER_ID: i64 = 424299;

macro_rules! build_app {
    ($pool:expr) => {{
        let ctx =
            ::common::context::RequestContext::with_username(USER_ID, "gw@test.local", "gw-test");
        test::init_service(
            App::new().app_data(web::Data::new($pool.clone())).service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::publish::register),
            ),
        )
        .await
    }};
}

async fn test_user(pool: &sqlx::PgPool) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'gw-test', 'gw-test', 'gw@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(USER_ID)
    .execute(pool)
    .await
    .unwrap();
}

/// 建流程行（meta 设计图）
async fn create_flow(pool: &sqlx::PgPool, name: &str, code: &str, graph: &Value) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve" (notice, meta, code, created_by_id, _f_, _t_)
           VALUES ($1, $2::jsonb, $3, 1, '实现', '范例') RETURNING id"#,
    )
    .bind(name)
    .bind(graph.to_string())
    .bind(code)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 标记流程已发布（主状态桥 published）
async fn mark_published(pool: &sqlx::PgPool, flow_id: i64) {
    let existing: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_stus-process" WHERE code = 'published' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(pool)
    .await
    .unwrap();
    let status_id: i64 = match existing {
        Some(id) => id,
        None => sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_stus-process" (id, code, notice)
                   VALUES (isahl.gen_next_zuid(), 'published', '已发布') RETURNING id"#,
        )
        .fetch_one(pool)
        .await
        .unwrap(),
    };
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (id, ref_left, ref_right)
           VALUES (isahl.gen_next_zuid(), $1, $2)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(flow_id)
    .bind(status_id)
    .execute(pool)
    .await
    .unwrap();
}

/// 调用 publish 端点返回状态码
macro_rules! publish_flow {
    ($app:expr, $flow_id:expr) => {{
        let resp = test::call_service(
            $app,
            test::TestRequest::post()
                .uri(&format!("/test/approval-flows/{}/publish", $flow_id))
                .to_request(),
        )
        .await;
        let st = resp.status().as_u16();
        if st >= 400 {
            let b: Value = test::read_body_json(resp).await;
            eprintln!("publish {} -> {}: {:?}", $flow_id, st, b);
        }
        st
    }};
}

/// 有效图（start + end 配置齐全；end 须 statementLeaf 白名单）
fn valid_graph() -> Value {
    json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "drive": "event", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"},
        ]
    })
}

#[tokio::test]
async fn subflow_publish_validates_target_flow() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let app = build_app!(pool);

    // 被引用流程（已发布）
    let sub_flow = create_flow(&pool, "子流程", "AF-SUB", &valid_graph()).await;
    mark_published(&pool, sub_flow).await;

    // 1. target 存在且已发布 → 200
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "drive": "event", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-sub", "type": "subflow", "label": "子流程引用", "target": "AF-SUB"},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"},
        ]
    });
    let flow = create_flow(&pool, "父流程-成功", "AF-PARENT-OK", &graph).await;
    let s = publish_flow!(&app, flow);
    assert_eq!(s, 200, "target 已发布的 subflow 应发布成功");

    // 物化验证：subflow target 落 timeline
    let target: Option<String> = sqlx::query_scalar(
        r#"SELECT ea.timeline->>'target' FROM isahl."zc_id_even-approve" ea
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_operation o ON o.id = oe.ref_left AND o.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro ON rro.ref_right = o.id
           WHERE rro.ref_left = $1 AND ea.deleted_at IS NULL
             AND rro.code = 'n-sub' LIMIT 1"#,
    )
    .bind(flow)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    assert_eq!(
        target.as_deref(),
        Some("AF-SUB"),
        "subflow target 应物化于 timeline"
    );

    // 2. target 不存在 → 400
    let graph2 = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "drive": "event", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-sub", "type": "subflow", "label": "子流程引用", "target": "NO-SUCH-FLOW"},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"},
        ]
    });
    let flow2 = create_flow(&pool, "父流程-缺target", "AF-PARENT-MISS", &graph2).await;
    let s2 = publish_flow!(&app, flow2);
    assert_eq!(s2, 400, "target 不存在的 subflow 应 400");

    // 3. target 存在但未发布 → 400
    let _un_pub = create_flow(&pool, "未发布子流程", "AF-UNPUB", &valid_graph()).await;
    let graph3 = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "drive": "event", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-sub", "type": "subflow", "label": "子流程引用", "target": "AF-UNPUB"},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"},
        ]
    });
    let flow3 = create_flow(&pool, "父流程-未发布target", "AF-PARENT-UNPUB", &graph3).await;
    let s3 = publish_flow!(&app, flow3);
    assert_eq!(s3, 400, "target 未发布的 subflow 应 400");

    // 4. 自引用 → 400
    let graph4 = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "drive": "event", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-sub", "type": "subflow", "label": "子流程引用", "target": "AF-SELF"},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"},
        ]
    });
    let flow4 = create_flow(&pool, "自引用流程", "AF-SELF", &graph4).await;
    let s4 = publish_flow!(&app, flow4);
    assert_eq!(s4, 400, "自引用 subflow 应 400");
}

#[tokio::test]
async fn loop_and_branch_publish_materialize_timeline() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let app = build_app!(pool);

    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "drive": "event", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-loop", "type": "loop", "label": "循环", "loopExpr": "{{count}} < 3", "maxIter": 10,
             "next": [{"to": 2, "cond": "{{count}} < 3"}, {"to": 3}]},
            {"id": "n-approve", "type": "approval", "label": "审批", "mode": "or_sign"},
            {"id": "n-branch", "type": "branch", "label": "汇聚", "joinRule": "any"},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"},
        ]
    });
    let flow = create_flow(&pool, "循环汇聚流程", "AF-LOOP-BR", &graph).await;
    let s = publish_flow!(&app, flow);
    assert_eq!(s, 200, "loop+branch 图应发布成功");

    // loop timeline 物化
    let (loop_expr, loop_iter): (Option<String>, Option<i64>) = sqlx::query_as(
        r#"SELECT ea.timeline->>'loopExpr', (ea.timeline->>'loopMaxIter')::bigint
           FROM isahl."zc_id_even-approve" ea
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_operation o ON o.id = oe.ref_left AND o.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro ON rro.ref_right = o.id
           WHERE rro.ref_left = $1 AND rro.code = 'n-loop' AND ea.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .unwrap_or((None, None));
    assert_eq!(
        loop_expr.as_deref(),
        Some("{{count}} < 3"),
        "loopExpr 应物化"
    );
    assert_eq!(loop_iter, Some(10), "loopMaxIter 应物化");

    // branch joinRule 物化
    let join_rule: Option<String> = sqlx::query_scalar(
        r#"SELECT ea.timeline->>'joinRule' FROM isahl."zc_id_even-approve" ea
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_operation o ON o.id = oe.ref_left AND o.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro ON rro.ref_right = o.id
           WHERE rro.ref_left = $1 AND rro.code = 'n-branch' AND ea.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    assert_eq!(join_rule.as_deref(), Some("any"), "branch joinRule 应物化");
}

#[tokio::test]
async fn loop_formula_publish_materializes_chain_and_rejects_bad_formula() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let app = build_app!(pool);

    // 1. 合法 loopFormula：发布成功 + formula/standard 链物化 + operation.meta
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "drive": "event", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-loop", "type": "loop", "label": "循环", "loopFormula": "cursor < maxIter",
             "loopVars": [{"name": "maxIter", "init": 3}],
             "next": [{"to": 2}, {"to": 3}]},
            {"id": "n-approve", "type": "approval", "label": "审批", "mode": "or_sign"},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"},
        ]
    });
    let flow = create_flow(&pool, "循环公式流程", "AF-LOOP-FMLA", &graph).await;
    let s = publish_flow!(&app, flow);
    assert_eq!(s, 200, "合法 loopFormula 应发布成功");

    // operation.meta loop 写入（vars/cursor/formula）
    let meta: Option<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT o.meta FROM isahl."zc_id_operation" o
           JOIN isahl.zc_id_process_rr_operation rro ON rro.ref_right = o.id
           WHERE rro.ref_left = $1 AND rro.code = 'n-loop' AND o.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    let meta = meta.expect("operation.meta 应存在");
    assert_eq!(meta["loop"]["vars"]["maxIter"], 3, "maxIter 局部变量应物化");
    assert_eq!(
        meta["loop"]["cursors"],
        json!({}),
        "cursors 初始空对象（D2 执行域键容器，flat cursor 弃写）"
    );
    assert_eq!(meta["loop"]["formula"], "cursor < maxIter");

    // formula 行（code LOOP-FMLA-n-loop + Rhai 表达式 + context engine）
    let (fmla_expr, fmla_ctx): (String, serde_json::Value) = sqlx::query_as(
        r#"SELECT fo.expression, fo.context FROM isahl.zc_id_formula fo
           WHERE fo.code = 'LOOP-FMLA-n-loop' AND fo.deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(fmla_expr, "cursor < maxIter");
    assert_eq!(fmla_ctx["engine"], "rhai");

    // standard 实现·实例（tpl_id → 范例）+ 双桥
    let std_inst: Option<(String, Option<i64>)> = sqlx::query_as(
        r#"SELECT s._t_, s.tpl_id FROM isahl.zc_id_standard s
           WHERE s.code = 'LOOP-STD-n-loop' AND s.deleted_at IS NULL
           ORDER BY s.id DESC LIMIT 1"#,
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    let (t_inst, tpl) = std_inst.expect("standard 实例应存在");
    assert_eq!(t_inst, "实例", "应为实现·实例");
    assert!(tpl.is_some(), "实例应挂 tpl_id → 范例");
    let op_std: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_operation_rr_standard rs
           JOIN isahl.zc_id_process_rr_operation rro ON rro.ref_right = rs.ref_left
           WHERE rro.ref_left = $1 AND rro.code = 'n-loop' AND rs.deleted_at IS NULL"#,
    )
    .bind(flow)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(op_std, 1, "operation_rr_standard 桥应存在");
    let std_fmla: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_standard_r_formula rf
           JOIN isahl.zc_id_operation_rr_standard rs ON rs.ref_right = rf.ref_left AND rs.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro ON rro.ref_right = rs.ref_left
           WHERE rro.ref_left = $1 AND rf.deleted_at IS NULL"#,
    )
    .bind(flow)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(std_fmla, 1, "standard_r_formula 桥应存在");

    // 2. 非法 Rhai 公式 → 400 + 不物化
    let bad_graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "drive": "event", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-loop", "type": "loop", "label": "循环", "loopFormula": "cursor < maxIter (("},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"},
        ]
    });
    let bad_flow = create_flow(&pool, "坏公式流程", "AF-LOOP-BAD", &bad_graph).await;
    let sb = publish_flow!(&app, bad_flow);
    assert_eq!(sb, 400, "非法 Rhai 公式应 400");

    // 3. 缺 loopFormula（无 loopExpr 旧值）→ 400
    let no_fmla = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "drive": "event", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-loop", "type": "loop", "label": "循环", "next": [{"to": 2}, {"to": 3}]},
            {"id": "n-approve", "type": "approval", "label": "审批", "mode": "or_sign"},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"},
        ]
    });
    let nf_flow = create_flow(&pool, "缺公式流程", "AF-LOOP-NOFMLA", &no_fmla).await;
    let sn = publish_flow!(&app, nf_flow);
    assert_eq!(sn, 400, "缺 loopFormula 且无 loopExpr 应 400");

    // 4. 旧 loopExpr 图（存量兼容）→ 200
    let legacy_graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "drive": "event", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-loop", "type": "loop", "label": "循环", "loopExpr": "{{count}} < 3", "maxIter": 10,
             "next": [{"to": 2, "cond": "{{count}} < 3"}, {"to": 3}]},
            {"id": "n-approve", "type": "approval", "label": "审批", "mode": "or_sign"},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"},
        ]
    });
    let lg_flow = create_flow(&pool, "旧循环流程", "AF-LOOP-LEGACY", &legacy_graph).await;
    let sl = publish_flow!(&app, lg_flow);
    assert_eq!(sl, 200, "旧 loopExpr 图应兼容发布");
}

#[tokio::test]
async fn loop_formula_embeds_threshold_with_default_maxiter() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;
    let app = build_app!(pool);

    // 公式内嵌阈值（cursor < 3）且无 loopVars——meta.vars 默认 maxIter=10
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "drive": "event", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-loop-emb", "type": "loop", "label": "循环", "loopFormula": "cursor < 3",
             "next": [{"to": 2}, {"to": 3}]},
            {"id": "n-approve", "type": "approval", "label": "审批", "mode": "or_sign"},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"},
        ]
    });
    let flow = create_flow(&pool, "内嵌阈值循环", "AF-LOOP-EMBED", &graph).await;
    let s = publish_flow!(&app, flow);
    assert_eq!(s, 200, "公式内嵌阈值无 loopVars 应发布成功");

    let meta: Option<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT o.meta FROM isahl."zc_id_operation" o
           JOIN isahl.zc_id_process_rr_operation rro ON rro.ref_right = o.id
           WHERE rro.ref_left = $1 AND rro.code = 'n-loop-emb' AND o.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    let meta = meta.expect("operation.meta 应存在");
    assert_eq!(
        meta["loop"]["vars"]["maxIter"], 10,
        "无 loopVars 时 maxIter 默认 10 兜底"
    );
    assert_eq!(meta["loop"]["formula"], "cursor < 3");

    // 旧 loopVars 图兼容：loopVars 存在时合并保留
    let legacy_graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "drive": "event", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-loop-emb", "type": "loop", "label": "循环", "loopFormula": "cursor < maxIter",
             "loopVars": [{"name": "maxIter", "init": 5}],
             "next": [{"to": 2}, {"to": 3}]},
            {"id": "n-approve", "type": "approval", "label": "审批", "mode": "or_sign"},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"},
        ]
    });
    let legacy_flow = create_flow(&pool, "旧 loopVars 循环", "AF-LOOP-VARS", &legacy_graph).await;
    let sl = publish_flow!(&app, legacy_flow);
    assert_eq!(sl, 200, "旧 loopVars 图应兼容发布");
    let lmeta: Option<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT o.meta FROM isahl."zc_id_operation" o
           JOIN isahl.zc_id_process_rr_operation rro ON rro.ref_right = o.id
           WHERE rro.ref_left = $1 AND rro.code = 'n-loop-emb' AND o.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(legacy_flow)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    let lmeta = lmeta.expect("legacy operation.meta 应存在");
    assert_eq!(
        lmeta["loop"]["vars"]["maxIter"], 5,
        "旧 loopVars 的 maxIter 应保留"
    );
}
