//! 模型级种子流程写面守卫集成测试（add-model-seed-flow-guard）
//!
//! 守护可观察行为：
//! 1. `meta.managed='model-seed'` 流程（种子三流程行同款标记）：update / delete /
//!    publish / unpublish 四面全部拒绝（Validation），数据零变更；
//! 2. DTO 派生字段 `managed_by`（meta->>'managed'）随读路径回传；
//! 3. 普通流程（meta 无 managed 键）update/delete 行为不变（回归锚）；
//! 4. 归属 managed 流程的 even-approve 节点行 update/delete 拒绝；不归属的可删。

use ::common::context::RequestContext;
use ::common::error::AliothError;
use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::handlers;
use approval::models::{UpdateApprovalFlowRequest, UpdateFlowNodeRequest};
use approval::repositories::{ApprovalFlowRepository, FlowNodeRepository};
use crud::repository::AliothRepository;
use serde_json::{json, Value};
use sqlx::PgPool;

mod common;
use common::{setup_test_schema, test_code};

const USER_ID: i64 = 454545;

async fn seed_user(pool: &PgPool) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'seed-guard-test', 'seed-guard-test', 'seed-guard@test.local',
                   'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(USER_ID)
    .execute(pool)
    .await
    .unwrap();
}

/// 直插流程行（不经 repository——模拟种子/存量行）；`marker` 为 Some 携带治理标记值
///（model-seed / ns-seed——守卫对 managed 非空一视同仁，extend-managed-guard-to-ns-seeds）。
async fn insert_flow(pool: &PgPool, code: &str, marker: Option<&str>) -> i64 {
    let meta = match marker {
        Some(m) => json!({ "managed": m, "version": 1, "nodes": [] }),
        None => json!({ "version": 1, "nodes": [] }),
    };
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve"
           (notice, code, meta, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3::jsonb, $4, '设计', '实例', NULL, NULL, NULL)
           RETURNING id"#,
    )
    .bind(format!("guard-{code}"))
    .bind(code)
    .bind(meta.to_string())
    .bind(USER_ID)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 建 even-approve 节点行；`bridge` 为真时接三跳桥归属 `flow_id`。
async fn insert_node(pool: &PgPool, code: &str, bridge: Option<i64>) -> i64 {
    let node_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_appr-process" (notice, code, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, NULL, NULL, NULL) RETURNING id"#,
    )
    .bind(format!("节点-{code}"))
    .bind(code)
    .bind(USER_ID)
    .fetch_one(pool)
    .await
    .unwrap();
    if let Some(flow_id) = bridge {
        let op_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_oper-gate" (notice, code, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, NULL, NULL, NULL) RETURNING id"#,
        )
        .bind(format!("操作-{code}"))
        .bind(code)
        .bind(USER_ID)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            r#"INSERT INTO isahl.zc_id_operation_rr_event (ref_left, ref_right, created_by_id)
               VALUES ($1, $2, $3)"#,
        )
        .bind(op_id)
        .bind(node_id)
        .bind(USER_ID)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            r#"INSERT INTO isahl.zc_id_process_rr_operation
               (code, ref_left, ref_right, created_by_id)
               VALUES ($1, $2, $3, $4)"#,
        )
        .bind(code)
        .bind(flow_id)
        .bind(op_id)
        .bind(USER_ID)
        .execute(pool)
        .await
        .unwrap();
    }
    node_id
}

fn test_ctx() -> RequestContext {
    RequestContext::with_username(USER_ID, "seed-guard@test.local", "seed-guard-test")
}

/// POST 端点调用，返回 (status, body)（避免命名 TestService 具体类型）。
macro_rules! post_endpoint {
    ($app:expr, $uri:expr) => {{
        let resp = actix_web::test::call_service(
            &$app,
            actix_web::test::TestRequest::post()
                .uri(&$uri)
                .set_json(serde_json::json!({}))
                .to_request(),
        )
        .await;
        let status = resp.status().as_u16();
        let bytes = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let val: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, val)
    }};
}

/// 拒绝断言：Validation + 消息明示「种子流程」且携带标记值（managed 非空即拒，
/// extend-managed-guard-to-ns-seeds 泛化后消息形如 `种子流程（ns-seed）不可……`）。
fn assert_managed_validation(err: AliothError, marker: &str) {
    match err {
        AliothError::Validation { message, .. } => {
            assert!(
                message.contains("种子流程") && message.contains(marker),
                "拒绝消息须明示种子流程并携带标记值 {marker}，实得：{message}"
            );
        }
        other => panic!("期望 Validation 拒绝，实得：{other:?}"),
    }
}

/// 节点归属守卫拒绝断言：消息明示「种子流程」（节点守卫不带标记值——归属反查只判非空）。
fn assert_managed_node_validation(err: AliothError) {
    match err {
        AliothError::Validation { message, .. } => {
            assert!(
                message.contains("种子流程"),
                "节点拒绝消息须明示种子流程，实得：{message}"
            );
        }
        other => panic!("期望 Validation 拒绝，实得：{other:?}"),
    }
}

/// 四面拒绝：update / delete / publish / unpublish，且数据零变更。
#[tokio::test]
async fn managed_flow_write_faces_rejected() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    seed_user(&pool).await;
    let code = test_code("SEEDGUARD");
    let flow_id = insert_flow(&pool, &code, Some("model-seed")).await;

    // DTO 派生字段：读路径回传治理标记
    let repo = ApprovalFlowRepository::new(pool.clone());
    let read = repo.get(flow_id).await.unwrap().expect("flow readable");
    assert_eq!(read.managed_by.as_deref(), Some("model-seed"));

    // ① update 拒绝
    let err = repo
        .update(
            flow_id,
            UpdateApprovalFlowRequest {
                name: Some("改名尝试".into()),
                comments: None,
                meta: Some(json!({ "version": 1, "nodes": [], "tampered": true })),
                context_id: None,
                context_table: None,
                expected_updated_at: None,
            },
            USER_ID,
        )
        .await
        .expect_err("managed 流程 update 必须拒绝");
    assert_managed_validation(err, "model-seed");

    // ② delete 拒绝
    let err = repo
        .delete(flow_id, USER_ID)
        .await
        .expect_err("managed 流程 delete 必须拒绝");
    assert_managed_validation(err, "model-seed");

    // ③ publish 拒绝 ④ unpublish 拒绝（真实端点）
    let ctx = test_ctx();
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
    let (s_pub, b_pub) = post_endpoint!(app, format!("/test/approval-flows/{flow_id}/publish"));
    assert_eq!(s_pub, 400, "publish 必须 400：{b_pub}");
    assert!(
        b_pub["message"]
            .as_str()
            .unwrap_or_default()
            .contains("种子流程")
            && b_pub["message"]
                .as_str()
                .unwrap_or_default()
                .contains("model-seed"),
        "publish 拒绝消息须明示种子流程并携带标记值：{b_pub}"
    );
    let (s_unpub, b_unpub) =
        post_endpoint!(app, format!("/test/approval-flows/{flow_id}/unpublish"));
    assert_eq!(s_unpub, 400, "unpublish 必须 400：{b_unpub}");

    // 数据零变更：名称/图/软删位保持原样，且未物化任何节点
    let (name, meta, alive): (String, Value, bool) = sqlx::query_as(
        r#"SELECT notice, meta, deleted_at IS NULL FROM isahl.zc_id_process WHERE id = $1"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(name, format!("guard-{code}"), "名称不得被改写");
    assert_eq!(meta["managed"], "model-seed", "治理标记不得被覆盖");
    assert!(meta.get("tampered").is_none(), "图不得被改写");
    assert!(alive, "流程行不得被软删");
    let materialized: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_process_rr_operation
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(materialized, 0, "publish 不得物化 managed 流程节点");

    // 清理
    sqlx::query("DELETE FROM isahl.zc_id_process WHERE id = $1")
        .bind(flow_id)
        .execute(&pool)
        .await
        .unwrap();
}

/// 回归锚：普通流程 update/delete 行为不变、managed_by 为 None。
#[tokio::test]
async fn normal_flow_write_unaffected() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    seed_user(&pool).await;
    let code = test_code("NORMALFLOW");
    let flow_id = insert_flow(&pool, &code, None).await;

    let repo = ApprovalFlowRepository::new(pool.clone());
    let before = repo.get(flow_id).await.unwrap().expect("flow readable");
    assert_eq!(before.managed_by, None, "普通流程无治理标记");

    let updated = repo
        .update(
            flow_id,
            UpdateApprovalFlowRequest {
                name: Some("普通流程改名".into()),
                comments: None,
                meta: None,
                context_id: None,
                context_table: None,
                expected_updated_at: None,
            },
            USER_ID,
        )
        .await
        .expect("普通流程 update 必须放行")
        .expect("行存在");
    assert_eq!(updated.name, "普通流程改名");

    repo.delete(flow_id, USER_ID)
        .await
        .expect("普通流程 delete 必须放行");
    let alive: bool =
        sqlx::query_scalar("SELECT deleted_at IS NULL FROM isahl.zc_id_process WHERE id = $1")
            .bind(flow_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!alive, "普通流程应已软删");

    // 清理
    sqlx::query("DELETE FROM isahl.zc_id_process WHERE id = $1")
        .bind(flow_id)
        .execute(&pool)
        .await
        .unwrap();
}

/// 归属守卫：managed 流程的节点行拒绝写；不归属节点可删。
#[tokio::test]
async fn managed_flow_node_write_rejected() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    seed_user(&pool).await;
    let code = test_code("SEEDNODE");
    let flow_id = insert_flow(&pool, &code, Some("model-seed")).await;
    let owned = insert_node(&pool, &format!("{code}-OWN"), Some(flow_id)).await;

    let repo = FlowNodeRepository::new(pool.clone());

    // update 拒绝（三跳桥反查归属）
    let err = repo
        .update(
            owned,
            UpdateFlowNodeRequest {
                label: Some("改名尝试".into()),
                code: None,
            },
            USER_ID,
        )
        .await
        .expect_err("managed 流程节点 update 必须拒绝");
    assert_managed_node_validation(err);

    // delete 拒绝
    let err = repo
        .delete(owned, USER_ID)
        .await
        .expect_err("managed 流程节点 delete 必须拒绝");
    assert_managed_node_validation(err);
    let alive: bool = sqlx::query_scalar(
        r#"SELECT deleted_at IS NULL FROM isahl."zc_id_even-approve" WHERE id = $1"#,
    )
    .bind(owned)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(alive, "归属节点不得被软删");

    // 不归属任何流程的节点 → 删除放行
    let free = insert_node(&pool, &format!("{code}-FREE"), None).await;
    repo.delete(free, USER_ID)
        .await
        .expect("无归属节点 delete 必须放行");
    let free_alive: bool = sqlx::query_scalar(
        r#"SELECT deleted_at IS NULL FROM isahl."zc_id_even-approve" WHERE id = $1"#,
    )
    .bind(free)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!free_alive, "无归属节点应已软删");

    // 清理
    sqlx::query("DELETE FROM isahl.zc_id_process WHERE id = $1")
        .bind(flow_id)
        .execute(&pool)
        .await
        .unwrap();
}

/// ns-seed 变体（extend-managed-guard-to-ns-seeds）：managed 非空即拒——namespace 种子
/// 流程（WZ FLOW-FREIGHT / Cosmic FLOW-VERCTRL / AVIC FLOW-STD 等）同受全部写面保护。
#[tokio::test]
async fn ns_seed_flow_write_faces_rejected() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    seed_user(&pool).await;
    let code = test_code("NSSEEDGUARD");
    let flow_id = insert_flow(&pool, &code, Some("ns-seed")).await;

    let repo = ApprovalFlowRepository::new(pool.clone());
    let read = repo.get(flow_id).await.unwrap().expect("flow readable");
    assert_eq!(read.managed_by.as_deref(), Some("ns-seed"));

    // update / delete 拒绝（消息携带 ns-seed 标记值）
    let err = repo
        .update(
            flow_id,
            UpdateApprovalFlowRequest {
                name: Some("改名尝试".into()),
                comments: None,
                meta: None,
                context_id: None,
                context_table: None,
                expected_updated_at: None,
            },
            USER_ID,
        )
        .await
        .expect_err("ns-seed 流程 update 必须拒绝");
    assert_managed_validation(err, "ns-seed");
    let err = repo
        .delete(flow_id, USER_ID)
        .await
        .expect_err("ns-seed 流程 delete 必须拒绝");
    assert_managed_validation(err, "ns-seed");

    // publish / unpublish 拒绝（真实端点）
    let ctx = test_ctx();
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
    let (s_pub, b_pub) = post_endpoint!(app, format!("/test/approval-flows/{flow_id}/publish"));
    assert_eq!(s_pub, 400, "publish 必须 400：{b_pub}");
    assert!(
        b_pub["message"]
            .as_str()
            .unwrap_or_default()
            .contains("ns-seed"),
        "publish 拒绝消息须携带 ns-seed 标记值：{b_pub}"
    );
    let (s_unpub, _) = post_endpoint!(app, format!("/test/approval-flows/{flow_id}/unpublish"));
    assert_eq!(s_unpub, 400, "unpublish 必须 400");

    // 归属节点删除拒绝 + 行存活
    let node = insert_node(&pool, &format!("{code}-N1"), Some(flow_id)).await;
    let node_repo = FlowNodeRepository::new(pool.clone());
    let err = node_repo
        .delete(node, USER_ID)
        .await
        .expect_err("ns-seed 归属节点 delete 必须拒绝");
    assert_managed_node_validation(err);
    let alive: bool = sqlx::query_scalar(
        r#"SELECT deleted_at IS NULL FROM isahl."zc_id_even-approve" WHERE id = $1"#,
    )
    .bind(node)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(alive, "归属节点不得被软删");

    // 清理（节点桥随行删除）
    sqlx::query("DELETE FROM isahl.zc_id_process WHERE id = $1")
        .bind(flow_id)
        .execute(&pool)
        .await
        .unwrap();
}
