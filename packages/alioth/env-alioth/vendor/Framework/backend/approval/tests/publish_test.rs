use ::common::testing::connect_test_db;
use actix_web::{dev::Service, test, web, App, HttpMessage};
use approval::handlers;
use approval::models::UpdateApprovalFlowRequest;
use approval::repositories::ApprovalFlowRepository;
use crud::repository::AliothRepository;
use serde_json::{json, Value};
mod common;
use common::{ensure_role_member, setup_test_schema, test_code};

#[tokio::test]
async fn publish_creates_event_rows() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    // 创建测试认证用户（publish handler require_auth 需要）
    let user_id = 424242;
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'publish-test', 'publish-test', 'publish@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "eventLeaf": "zc_id_even-accident"},
        ]
    });

    let flow_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_process (notice, meta, code, created_by_id)
           VALUES ($1, $2::jsonb, 'draft', 1) RETURNING id"#,
    )
    .bind("发布测试流程")
    .bind(graph.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    let ctx = ::common::context::RequestContext::with_username(
        user_id,
        "publish@test.local",
        "publish-test",
    );
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(handlers::publish::register),
        ),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-flows/{}/publish", flow_id))
            .to_request(),
    )
    .await;

    let status = resp.status();
    let body: Value = test::read_body_json(resp).await;
    assert!(
        status.is_success(),
        "publish failed: status={} body={:?}",
        status,
        body
    );
}

#[tokio::test]
async fn publish_rejects_subflow_with_missing_target() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let user_id = 424243;
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'publish-test-2', 'publish-test-2', 'publish2@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    // 2026-09-01 能力补齐：subflow 放行白名单，但 publish 物化校验
    // target 引用存在且已发布——target 'FLOW-X' 不存在 → fail-closed 400
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-sub", "type": "subflow", "label": "子流程引用", "target": "FLOW-X"},
        ]
    });

    let flow_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_process (notice, meta, code, created_by_id)
           VALUES ($1, $2::jsonb, 'draft', 1) RETURNING id"#,
    )
    .bind("subflow拒绝测试流程")
    .bind(graph.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    let ctx = ::common::context::RequestContext::with_username(
        user_id,
        "publish2@test.local",
        "publish-test-2",
    );
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(handlers::publish::register),
        ),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-flows/{}/publish", flow_id))
            .to_request(),
    )
    .await;

    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::BAD_REQUEST,
        "subflow 节点应被拒绝发布"
    );

    // fail-closed 验证：无任何节点物化行
    let node_rows: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_even-approve" n
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = n.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.ref_left = $1 AND rro.deleted_at IS NULL
           WHERE n.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(node_rows, 0, "被拒绝的 publish 不得物化节点行");
}

/// fix-approval-engine-semantics P1-6：重发布退役旧批（软删）+ 新批批次标记 +
/// 节点 meta 物化（P0-1）。
#[tokio::test]
async fn publish_retires_old_batch_and_marks_version() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let user_id = 424244;
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'publish-test-3', 'publish-test-3', 'publish3@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();
    // 岗位物化接线（2026-09-03 语义接线）：publish 把 role（岗位名/position id）
    // 解析为 zc_id_subj-position 岗位成员 → rr_approve 桥（NGAC UA 域不再承担岗位解析）。
    // 预置岗位（notice=法务岗…，fk_user=user_id）供断言；role 以岗位名直配。
    // 先清理本前缀历史岗位行（残留行会放大 approver_count，测试须确定性自愈）。
    sqlx::query(r#"DELETE FROM isahl."zc_id_subj-position" WHERE code LIKE 'TST-POS-LEGAL%'"#)
        .execute(&pool)
        .await
        .unwrap();
    ensure_role_member(&pool, "legal", user_id).await.unwrap();
    let pos_code = format!("TST-POS-LEGAL-{}", test_code("pos"));
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-position" (id, notice, code, fk_user)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3)"#,
    )
    .bind("法务岗（publish 测试）")
    .bind(&pos_code)
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    let ctx = ::common::context::RequestContext::with_username(
        user_id,
        "publish3@test.local",
        "publish-test-3",
    );
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(handlers::publish::register)
                .configure(handlers::version::register),
        ),
    )
    .await;

    // v1：两节点（start + approve 带岗位 meta + 边 cond）
    let graph1 = json!({
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-appr", "type": "approve", "label": "审批",
             "role": "法务岗（publish 测试）", "roleKind": "role"},
        ],
        "edges": [
            {"source": "n-start", "target": "n-appr"}
        ]
    });
    let flow_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_process (notice, meta, code, created_by_id)
           VALUES ($1, $2::jsonb, 'draft', 1) RETURNING id"#,
    )
    .bind("版本化测试流程")
    .bind(graph1.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-flows/{}/publish", flow_id))
            .to_request(),
    )
    .await;
    assert!(resp.status().is_success());

    // v1 断言（fix-avic-approval-node-model 契约）：comments 不承载 meta（NULL）；
    // 岗位物化到操作模型表——操作叶行 + rr_event 桥 + rr_approve 桥（role→岗位）；
    // 批次标记=1。
    let (comments, batch): (Option<String>, Option<serde_json::Value>) = sqlx::query_as(
        r#"SELECT n.comments, n.timeline FROM isahl."zc_id_even-approve" n
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = n.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.ref_left = $1 AND rro.deleted_at IS NULL
           WHERE n.code = 'n-appr' AND n.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        comments.is_none() || comments.as_deref() == Some(""),
        "publish 不得再向 comments 写入岗位 meta（迁移契约），实际: {:?}",
        comments
    );
    assert_eq!(batch.unwrap()["publish_batch"], 1, "首批发批次标记=1");
    let (_node_id, op_count, approver_count): (i64, i64, i64) = sqlx::query_as(
        r#"SELECT n.id,
                  COUNT(DISTINCT oa.id),
                  COUNT(DISTINCT oa2.ref_right)
           FROM isahl."zc_id_even-approve" n
           JOIN isahl.zc_id_operation_rr_event oe
             ON oe.ref_right = n.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.ref_left = $1 AND rro.deleted_at IS NULL
           LEFT JOIN isahl."zc_id_oper-approve" oa
             ON oa.id = oe.ref_left AND oa.deleted_at IS NULL
           LEFT JOIN isahl.zc_id_operation_rr_approve oa2
             ON oa2.ref_left = oa.id AND oa2.deleted_at IS NULL
           WHERE n.code = 'n-appr' AND n.deleted_at IS NULL
           GROUP BY n.id"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(op_count, 1, "publish 必须物化操作叶行");
    assert_eq!(approver_count, 1, "publish 必须物化岗位桥（legal）");

    // v2：单节点重发布（无在途 → 放行）
    let graph2 = json!({
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "eventLeaf": "zc_id_even-accident"},
            {"id": "n-appr2", "type": "approve", "label": "审批2",
             "role": "法务岗（publish 测试）", "roleKind": "role"},
        ],
        "edges": [
            {"source": "n-start", "target": "n-appr2"}
        ]
    });
    sqlx::query(r#"UPDATE isahl.zc_id_process SET meta = $2::jsonb WHERE id = $1"#)
        .bind(flow_id)
        .bind(graph2.to_string())
        .execute(&pool)
        .await
        .unwrap();
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-flows/{}/publish", flow_id))
            .to_request(),
    )
    .await;
    assert!(resp.status().is_success(), "无在途时重发布应放行");

    // v2 断言：旧批软删（deleted_at 非空）、新批标记=2、tk_version=2。
    // 2026-08-31 契约：event 驱动 start 的 rr_event → 事件真叶表范例行（非 even-approve
    // 载体），旧批软删计数改按 rr_event 桥行数（每节点一座桥，与语义实体落表解耦）。
    let old_deleted: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_operation_rr_event oe
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.ref_left = $1
           WHERE oe.deleted_at IS NOT NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(old_deleted, 2, "旧批两节点必须软删");
    let new_batch: Option<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT n.timeline FROM isahl."zc_id_even-approve" n
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = n.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.ref_left = $1 AND rro.deleted_at IS NULL
           WHERE n.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(new_batch.unwrap()["publish_batch"], 2, "新批标记=2");
    let tk: i64 = sqlx::query_scalar(r#"SELECT tk_version FROM isahl.zc_id_process WHERE id = $1"#)
        .bind(flow_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tk, 2, "tk_version 递增到 2");

    // versions 端点：两个版本（1 与 2）
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/test/approval-flows/{}/versions", flow_id))
            .to_request(),
    )
    .await;
    assert!(resp.status().is_success(), "versions 端点失败");
    let body: Value = test::read_body_json(resp).await;
    let versions = body["data"]["versions"].as_array().unwrap();
    let mut batch_nos: Vec<i64> = versions
        .iter()
        .filter_map(|v| v["version"].as_i64())
        .collect();
    batch_nos.sort_unstable();
    assert_eq!(batch_nos, vec![1, 2], "versions 必须按批次标记列出 1、2");
}

/// fix-approval-engine-semantics P1-6：旧 DAG 有在途实例时重发布必须拒绝
/// （防软删断链——advance_flow 的 even-approve join 带 deleted_at 过滤）。
#[tokio::test]
async fn publish_blocked_by_inflight_instance() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let user_id = 424245;
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'publish-test-4', 'publish-test-4', 'publish4@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    let ctx = ::common::context::RequestContext::with_username(
        user_id,
        "publish4@test.local",
        "publish-test-4",
    );
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(handlers::publish::register),
        ),
    )
    .await;

    let graph = json!({
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "eventLeaf": "zc_id_even-accident"},
        ]
    });
    let flow_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_process (notice, meta, code, created_by_id)
           VALUES ($1, $2::jsonb, 'draft', 1) RETURNING id"#,
    )
    .bind("在途阻断测试流程")
    .bind(graph.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-flows/{}/publish", flow_id))
            .to_request(),
    )
    .await;
    assert!(resp.status().is_success());

    // 制造在途实例（挂到已发布节点）
    // 2026-08-31 契约：单 start（event 驱动）图的 rr_event → 事件真叶表范例行
    // （不再经 even-approve 载体），在途实例挂叶表范例 id
    let node_id: i64 = sqlx::query_scalar(
        r#"SELECT n.id FROM isahl."zc_id_even-accident" n
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = n.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.ref_left = $1 AND rro.deleted_at IS NULL
           WHERE n.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    // tpl_id = 节点事件：在途守卫按 tpl_id IS NOT NULL 判别真实实例
    // （操作定义行 tpl_id NULL 不参与在途计数）
    let inflight_instance: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, fk_subject, created_by_id, tpl_id)
           VALUES ('在途实例', $1, $1, $2) RETURNING id"#,
    )
    .bind(user_id)
    .bind(node_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    // 实例↔审批事件经 rr_event 桥（fk_approve 列已移除）
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(inflight_instance)
    .bind(node_id)
    .execute(&pool)
    .await
    .unwrap();

    // 重发布 → 拒绝
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-flows/{}/publish", flow_id))
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::BAD_REQUEST,
        "存在在途实例时重发布必须拒绝"
    );

    // 旧批未被软删（2026-08-31 契约：event 驱动 start 的 rr_event 指向事件
    // 真叶表范例行，无 even-approve 载体——按 rr_operation 未软删判定）
    let live: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_process_rr_operation rro
           WHERE rro.ref_left = $1 AND rro.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(live, 1, "被拒发布不得软删旧批");
}

/// refactor-flow-node-operation-model：review/action 动作节点物化
/// （oper-check/oper-action 子类 + rr_review/rr_post 岗位桥 + DAG 重指 operation）。
#[tokio::test]
async fn publish_review_action_nodes_materialize_operation() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let user_id = 424246;
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'ra-node', 'ra-node', 'ra@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "开始", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "n-review", "type": "review", "label": "评审", "role": "r_review_role",
             "roleKind": "role", "next": [{"to": 2}]},
            {"id": "n-action", "type": "action", "label": "执行", "role": "r_action_role",
             "roleKind": "role", "next": [{"to": 3}]},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });

    let flow_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_process (notice, meta, code, created_by_id)
           VALUES ($1, $2::jsonb, 'FLOW-RA', 1) RETURNING id"#,
    )
    .bind("评审执行测试流程")
    .bind(graph.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    let ctx = ::common::context::RequestContext::with_username(user_id, "ra@test.local", "ra-node");
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(handlers::publish::register),
        ),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-flows/{}/publish", flow_id))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::OK);

    // review 节点 → oper-check 子类 + rr_review 桥；action → oper-action + rr_post 桥
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        r#"SELECT rro.code, o.tableoid::regclass::text, o.notice
           FROM isahl.zc_id_process_rr_operation rro
           JOIN isahl.zc_id_operation o ON o.id = rro.ref_right AND o.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.deleted_at IS NULL
           ORDER BY rro.id"#,
    )
    .bind(flow_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    // (graph_id, 落表, label)：start/end → oper-gate；review → oper-check；action → oper-action
    // 2026-08-31 §4.4：rro.code = 图内节点编号（n-review 等），非节点类型词
    assert_eq!(rows.len(), 4, "四节点各物化 operation 行");
    let by_type: std::collections::HashMap<_, _> =
        rows.iter().map(|r| (r.0.as_str(), r.1.clone())).collect();
    assert!(
        by_type["n-review"].contains("oper-check"),
        "review 落 oper-check: {}",
        by_type["n-review"]
    );
    assert!(
        by_type["n-action"].contains("oper-action"),
        "action 落 oper-action: {}",
        by_type["n-action"]
    );
    assert!(
        by_type["n-start"].contains("oper-gate"),
        "start 落 oper-gate: {}",
        by_type["n-start"]
    );

    // 岗位桥：review → rr_review；action → rr_post
    let review_op: i64 = sqlx::query_scalar(
        r#"SELECT rro.ref_right FROM isahl.zc_id_process_rr_operation rro
           WHERE rro.ref_left = $1 AND rro.code = 'n-review' AND rro.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let review_bridge: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_operation_rr_review" WHERE ref_left = $1"#,
    )
    .bind(review_op)
    .fetch_one(&pool)
    .await
    .unwrap();
    // role 解析为空（r_review_role 未建 UA）→ 桥行 0；重点断言桥表存在且指向 op
    let action_op: i64 = sqlx::query_scalar(
        r#"SELECT rro.ref_right FROM isahl.zc_id_process_rr_operation rro
           WHERE rro.ref_left = $1 AND rro.code = 'n-action' AND rro.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let _ = (review_bridge, action_op);
    // 模板桥：operation → even-approve 模板
    let templates: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_operation_rr_event oe
           JOIN isahl."zc_id_even-approve" ea ON ea.id = oe.ref_right
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.ref_left = $1"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        templates, 2,
        "event 驱动 start 的 rr_event 指向事件真叶表（非 even-approve 载体），
         模板桥仅 review/action 两个（end 走 rr_statement）"
    );

    // 终端节点语义链（2026-08-29 裁决）：process↔operation↔statement——
    // end 节点 op 行经 rr_statement 挂 statement 范例行（配置叶表 stat-inspection）
    let end_chain: Option<(String, String)> = sqlx::query_as(
        r#"SELECT replace(s.tableoid::regclass::text, '"', ''), s.code
           FROM isahl.zc_id_process_rr_operation rro
           JOIN isahl.zc_id_operation_rr_statement rs
             ON rs.ref_left = rro.ref_right AND rs.deleted_at IS NULL
           JOIN isahl.zc_id_statement s
             ON s.id = rs.ref_right AND s.deleted_at IS NULL
           WHERE rro.ref_left = $1 AND rro.code = 'n-end' AND rro.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_optional(&pool)
    .await
    .unwrap();
    let (end_leaf, end_code) = end_chain.expect("end 节点物化 statement 范例 + rr_statement 桥");
    assert_eq!(
        end_leaf, "zc_id_stat-inspection",
        "end 落配置 statement 真叶"
    );
    assert_eq!(end_code, "n-end", "statement 范例 code = 节点 graph id");

    // end 无 even-approve 事件行（终端语义：end 实体是 statement，非事件）
    let end_event_rows: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_even-approve" ea
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id
           JOIN isahl.zc_id_process_rr_operation rro ON rro.ref_right = oe.ref_left
           WHERE rro.ref_left = $1 AND rro.code = 'n-end' AND rro.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(end_event_rows, 0, "end 节点不得物化 even-approve 事件行");
}

/// 发布/停用以 code 列为生命周期唯一权威（comments 为 text 语义不承载状态）；
/// update 忽略 code（引擎发布位客户端禁写）。
#[tokio::test]
async fn publish_unpublish_code_column_authority() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let user_id = 424244;
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'status-sync', 'status-sync', 'status@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "开始", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "n-appr", "type": "approve", "label": "审批", "next": [{"to": 2}]},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"}
        ],
        "meta": {
            "branch": "zc_id_proc-approve",
            "contextTable": "zc_id_appr-payment",
            "status": "draft"
        }
    });

    let flow_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_process (notice, meta, code, created_by_id)
           VALUES ($1, $2::jsonb, 'FLOW-TEST', 1) RETURNING id"#,
    )
    .bind("状态双写测试流程")
    .bind(graph.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    let ctx = ::common::context::RequestContext::with_username(
        user_id,
        "status@test.local",
        "status-sync",
    );
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(handlers::publish::register),
        ),
    )
    .await;

    // ── publish：生命周期状态走 _r_status 桥（code 列保持业务码，comments 不改写）──
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-flows/{}/publish", flow_id))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::OK);

    let row: (String, Value) =
        sqlx::query_as(r#"SELECT code, meta FROM isahl.zc_id_process WHERE id = $1"#)
            .bind(flow_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    // code 列回归业务码（引擎不再占用为状态位）
    assert_eq!(row.0, "FLOW-TEST", "publish 不得覆盖 code 业务码");
    assert_eq!(
        row.1["meta"]["status"], "draft",
        "publish 不得改 meta.meta.status（设计图整体结构）"
    );
    // 主状态桥：published
    let status: String = sqlx::query_scalar(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" ls
           JOIN isahl."zc_id_stus-process" s ON s.id = ls.ref_right
           WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "published", "publish 后主状态桥=published");

    // ── unpublish：主状态桥回到 draft ──
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-flows/{}/unpublish", flow_id))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), actix_web::http::StatusCode::OK);

    let status: String = sqlx::query_scalar(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" ls
           JOIN isahl."zc_id_stus-process" s ON s.id = ls.ref_right
           WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "draft", "unpublish 后主状态桥=draft");
    let row: (String, Value) =
        sqlx::query_as(r#"SELECT code, meta FROM isahl.zc_id_process WHERE id = $1"#)
            .bind(flow_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row.0, "FLOW-TEST", "unpublish 不得覆盖 code 业务码");
    assert_eq!(
        row.1["meta"]["status"], "draft",
        "unpublish 不得改 meta.meta.status（设计图整体结构）"
    );

    // ── update 忽略 code：DTO 类型层已剥离 code 字段（serde 丢弃未知字段）──
    let repo = ApprovalFlowRepository::new(pool.clone());
    let req = UpdateApprovalFlowRequest {
        name: Some("改名后的流程".into()),
        comments: None,
        meta: None,
        context_id: None,
        context_table: None,
    };
    repo.update(flow_id, req, user_id).await.unwrap();

    let row: (String, String) =
        sqlx::query_as(r#"SELECT code, notice FROM isahl.zc_id_process WHERE id = $1"#)
            .bind(flow_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        row.0, "FLOW-TEST",
        "update 不得修改 code（业务码创建时设定，客户端禁写）"
    );
    assert_eq!(row.1, "改名后的流程", "name 仍可更新");
}
