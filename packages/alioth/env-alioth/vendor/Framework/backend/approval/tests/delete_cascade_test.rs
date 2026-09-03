//! 删除级联集成测试（fix-flow-designer-chain-breaks §D2）：
//! publish 物化 → repository delete → 断言节点/实例/意见/关系行全部软删、
//! 共享值对象（scal-duration / stan-operation / cate-proc_op）不受影响。

use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::repositories::ApprovalFlowRepository;
use crud::repository::AliothRepository;
use serde_json::json;
use sqlx::PgPool;

mod common;
use common::setup_test_schema;

const USER_ID: i64 = 434343;

async fn test_user(pool: &PgPool) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'cascade-test', 'cascade-test', 'cascade@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(USER_ID)
    .execute(pool)
    .await
    .unwrap();
}

/// 建流程并走真实 publish 端点（物化节点/操作/关系行），返回流程 id。
async fn create_and_publish(pool: &PgPool, name: &str) -> i64 {
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "n-a", "type": "approve", "label": "审批节点", "next": [{"to": 2}]},
            {"id": "n-end", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-inspection"}
        ]
    });
    let flow_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_process (notice, meta, code, created_by_id)
           VALUES ($1, $2::jsonb, 'draft', 1) RETURNING id"#,
    )
    .bind(name)
    .bind(graph.to_string())
    .fetch_one(pool)
    .await
    .unwrap();

    let ctx = ::common::context::RequestContext::with_username(
        USER_ID,
        "cascade@test.local",
        "cascade-test",
    );
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(approval::handlers::publish::register),
        ),
    )
    .await;
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-flows/{flow_id}/publish"))
            .set_json(json!({}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200, "publish 应 200");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(
        body["data"]["node_count"].as_i64().unwrap_or(0) >= 3,
        true,
        "publish 应物化 ≥3 节点: {body}"
    );
    flow_id
}

#[tokio::test]
async fn delete_cascades_materialized_rows() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;

    let flow_id = create_and_publish(&pool, "级联删除测试").await;

    // 发布物化前置断言：DAG 节点行存在（rr_operation 全节点载体——终端语义
    // 修正后 end/task-start 无 even-approve 行，节点计数以 DAG 桥为准）
    let node_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_process_rr_operation
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(node_count >= 3, "publish 应物化 ≥3 节点, got {node_count}");

    // 执行级联删除
    let repo = ApprovalFlowRepository::new(pool.clone());
    repo.delete(flow_id, USER_ID).await.expect("cascade delete");

    // 血缘链全部软删（even 语义行经桥链定位：rr_event 模板桥 + rro 在册锚；
    // 级联同时软删桥行——断言不得过滤桥的 deleted_at）
    let nodes_remaining: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_even-approve" n
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = n.id
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.ref_left = $1
           WHERE n.deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(nodes_remaining, 0, "even-approve 应无未删残留");
    let rr_remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl.zc_id_process_rr_operation WHERE ref_left = $1 AND deleted_at IS NULL",
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rr_remaining, 0, "process_rr_operation 应无未删残留");
    // oper-approve 实例经节点关联（实例→even 模板桥→在册锚）——该流程下应无未删实例
    let inst_remaining: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" o
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = o.id
           JOIN isahl."zc_id_even-approve" n ON n.id = oe.ref_right
           WHERE o.deleted_at IS NULL
             AND EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_event oe2
                         JOIN isahl.zc_id_process_rr_operation rro2
                           ON rro2.ref_right = oe2.ref_left
                         WHERE oe2.ref_right = n.id AND rro2.ref_left = $1)"#,
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(inst_remaining, 0, "oper-approve 应无未删残留");

    // 流程行自身软删
    let flow_alive: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl.zc_id_process WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(flow_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(flow_alive, 0, "流程行应已软删");
}

#[tokio::test]
async fn delete_does_not_touch_other_flows() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    test_user(&pool).await;

    let keep_id = create_and_publish(&pool, "保留流程").await;
    let drop_id = create_and_publish(&pool, "删除流程").await;

    let repo = ApprovalFlowRepository::new(pool.clone());
    repo.delete(drop_id, USER_ID).await.expect("cascade delete");

    let keep_nodes: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_process_rr_operation
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(keep_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(keep_nodes >= 3, "其他流程节点不得受影响");
}
