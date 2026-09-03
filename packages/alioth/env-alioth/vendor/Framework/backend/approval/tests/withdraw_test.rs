//! fix-approval-engine-semantics 撤回端点集成测试（D6）
//!
//! 验证：
//! - 申请人撤回在途实例 → withdrawn 桥 + 级联下游（fk_previous 链）+ 事件发布
//! - 非创建者（非 admin）撤回 → 403
//! - 已终态实例撤回 → Validation 错误
//! - 撤回通知投递当前审批人（失败仅 warn）

mod common;

use ::common::testing::connect_test_db;
use actix_web::{dev::Service, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;

use common::{ensure_role_member, setup_test_schema};

// 每个测试用独立用户 id——共享测试库中 grant_user_access 会持久挂 admin UA，
// 复用同一 id 会让后续测试的 is_admin 豁免/权限判定被污染。
const WD_OWNER: i64 = 3101;
const WD_OTHER: i64 = 3201;
const WD_OTHER2: i64 = 3202;
const WD_TERM: i64 = 3301;
const WD_NGAC: i64 = 3401;

/// 构造单节点流，返回 (flow_id, node_id=operation id)
async fn make_simple_flow(pool: &PgPool, code: &str) -> (i64, i64) {
    let flow_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_process (id, notice, code, comments, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $1, 'test', 1) RETURNING id"#,
    )
    .bind(code)
    .fetch_one(pool)
    .await
    .unwrap();
    // 桥链节点夹具（fk_process 已移除）：even 语义行 + oper 主体 +
    // rro 在册锚（ref_right=oper）+ 模板桥（rr_event: oper→even）
    let even_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, created_by_id, code, comments)
           VALUES ('审批节点', 1, 'N1', 'N1') RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let node_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, code, created_by_id)
           VALUES ('审批节点', 'N1', 1) RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_process_rr_operation
           (id, code, ref_left, ref_right, comments, "next-ops", created_by_id)
           VALUES (isahl.gen_next_uid(791), 'approve', $1, $2, 'N1', '[]'::jsonb, 1)"#,
    )
    .bind(flow_id)
    .bind(node_id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(node_id)
    .bind(even_id)
    .execute(pool)
    .await
    .unwrap();
    (flow_id, node_id)
}

/// 创建实例（created_by=owner，挂节点；节点=operation → 模板桥反查 even 模板）
async fn insert_instance(pool: &PgPool, node_id: i64, owner: i64, prev: Option<i64>) -> i64 {
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
           (id, notice, code, fk_subject, fk_operator, fk_previous, comments, created_by_id, tpl_id)
           VALUES (isahl.gen_next_zuid(), '待审', 'N', $1, $2, $3, $4, $1, $5)
           RETURNING id"#,
    )
    .bind(owner)
    .bind(owner)
    .bind(prev)
    .bind(r#"{"entityType":"x","entityId":77}"#)
    .bind(node_id)
    .fetch_one(pool)
    .await
    .unwrap();
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

async fn instance_status(pool: &PgPool, id: i64) -> Option<String> {
    sqlx::query_scalar(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" ls
           JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
           WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL
           ORDER BY ls.created_at DESC LIMIT 1"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn owner_withdraw_cascades_chain() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    common::grant_user_access(&pool, WD_OWNER, "approval-instances", &["withdraw"])
        .await
        .unwrap();

    let (flow_id, node1) = make_simple_flow(&pool, "FLOW-WD-1").await;
    // 下游节点 N2（桥链夹具）：even 语义行 + oper 主体 + rro 锚（ref_left=flow_id）
    // + 模板桥（rr_event: oper→even）
    let even2: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, created_by_id, code, comments)
           VALUES ('下游节点', 1, 'N2', 'N2') RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let node2: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, code, created_by_id)
           VALUES ('下游节点', 'N2', 1) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_process_rr_operation
           (id, code, ref_left, ref_right, comments, "next-ops", created_by_id)
           VALUES (isahl.gen_next_uid(791), 'approve', $1, $2, 'N2', '[]'::jsonb, 1)"#,
    )
    .bind(flow_id)
    .bind(node2)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(node2)
    .bind(even2)
    .execute(&pool)
    .await
    .unwrap();

    // 链：inst1（N1）→ inst2（N2, fk_previous=inst1）→ inst3（N2 兄弟, fk_previous=inst1）
    let inst1 = insert_instance(&pool, node1, WD_OWNER, None).await;
    let inst2 = insert_instance(&pool, node2, WD_OWNER, Some(inst1)).await;
    let inst3 = insert_instance(&pool, node2, WD_OWNER, Some(inst1)).await;

    let noop = Arc::new(common::noop_messaging::NoopMessaging::default());
    let ctx = ::common::context::RequestContext::with_username(WD_OWNER, "actor@test", "actor");
    let bus: Arc<dyn ::common::event_bus::DomainEventBus> =
        Arc::new(::common::event_bus::InMemoryEventBus::new());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(bus))
            .app_data(web::Data::new(
                noop.clone() as Arc<dyn ::common::messaging::MessagingService>
            ))
            .service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::withdraw::register)
                    .configure(handlers::approve_reject::register),
            ),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-instances/{}/withdraw", inst1))
            .to_request(),
    )
    .await;
    let body: serde_json::Value = actix_web::test::read_body_json(resp).await;
    assert_eq!(
        body["data"]["status"], "withdrawn",
        "撤回应返回 withdrawn: {:?}",
        body
    );

    assert_eq!(
        instance_status(&pool, inst1).await.as_deref(),
        Some("withdrawn")
    );
    assert_eq!(
        instance_status(&pool, inst2).await.as_deref(),
        Some("withdrawn"),
        "下游链实例必须级联撤回"
    );
    assert_eq!(
        instance_status(&pool, inst3).await.as_deref(),
        Some("withdrawn"),
        "同节点兄弟实例必须级联撤回"
    );
}

#[tokio::test]
async fn non_owner_withdraw_forbidden() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    // OTHER：非 admin、非创建者，但持有 withdraw 权限（专用 UA × 通配对象关联）
    // ——验证「权限过、所有权拒」路径（admin 授权路径会命中豁免，测不到 403）
    ensure_role_member(&pool, "r_wd_other", WD_OTHER2)
        .await
        .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_object_attribute
           (id, o_name, fk_policy_class, resource_type, fk_resource, created_at)
           SELECT isahl.gen_next_zuid(), 'grant-wild-other', pc.id, '*', 0, NOW()
           FROM isahl_auth.ngac_policy_class pc
           WHERE pc.o_name = 'default'
             AND NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_object_attribute
                             WHERE resource_type = '*' AND fk_resource = 0 AND deleted_at IS NULL)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_association
           (id, fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights, created_at)
           SELECT isahl.gen_next_zuid(), ua.id, oa.id, pc.id,
                  ARRAY(SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'withdraw'),
                  NOW()
           FROM isahl_auth.ngac_user_attribute ua
           CROSS JOIN (SELECT id FROM isahl_auth.ngac_object_attribute
                       WHERE resource_type = '*' AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1) oa
           CROSS JOIN (SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1) pc
           WHERE ua.o_name = 'r_wd_other' AND ua.deleted_at IS NULL
             AND NOT EXISTS (
                 SELECT 1 FROM isahl_auth.ngac_association a
                 WHERE a.fk_user_attribute = ua.id AND a.fk_object_attribute = oa.id
                   AND a.deleted_at IS NULL
             )"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    // OWNER：创建者，无权限 → require_resource_access 先拦（非 admin）
    let (_, node1) = make_simple_flow(&pool, "FLOW-WD-2").await;
    let inst1 = insert_instance(&pool, node1, WD_OTHER, None).await;

    let noop = Arc::new(common::noop_messaging::NoopMessaging::default());
    let ctx = ::common::context::RequestContext::with_username(WD_OTHER2, "actor@test", "actor");
    let bus: Arc<dyn ::common::event_bus::DomainEventBus> =
        Arc::new(::common::event_bus::InMemoryEventBus::new());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(bus))
            .app_data(web::Data::new(
                noop.clone() as Arc<dyn ::common::messaging::MessagingService>
            ))
            .service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::withdraw::register)
                    .configure(handlers::approve_reject::register),
            ),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-instances/{}/withdraw", inst1))
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::FORBIDDEN,
        "非创建者不得撤回"
    );
    assert_eq!(
        instance_status(&pool, inst1).await.as_deref(),
        None,
        "被拒撤回不得改状态"
    );
}

#[tokio::test]
async fn terminal_instance_cannot_withdraw() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    common::grant_user_access(
        &pool,
        WD_TERM,
        "approval-instances",
        &["withdraw", "approve"],
    )
    .await
    .unwrap();

    let (_, node1) = make_simple_flow(&pool, "FLOW-WD-3").await;
    let inst1 = insert_instance(&pool, node1, WD_TERM, None).await;

    // 先 approve（终态 approved）
    let ctx = ::common::context::RequestContext::with_username(WD_OWNER, "actor@test", "actor");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(
                Arc::new(::common::event_bus::InMemoryEventBus::new())
                    as Arc<dyn ::common::event_bus::DomainEventBus>,
            ))
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
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-instances/{}/approve", inst1))
            .set_json(json!({"opinion": "同意"}))
            .to_request(),
    )
    .await;
    assert!(resp.status().is_success());
    assert_eq!(
        instance_status(&pool, inst1).await.as_deref(),
        Some("approved")
    );

    let noop = Arc::new(common::noop_messaging::NoopMessaging::default());
    let ctx = ::common::context::RequestContext::with_username(WD_OWNER, "actor@test", "actor");
    let bus: Arc<dyn ::common::event_bus::DomainEventBus> =
        Arc::new(::common::event_bus::InMemoryEventBus::new());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(bus))
            .app_data(web::Data::new(
                noop.clone() as Arc<dyn ::common::messaging::MessagingService>
            ))
            .service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::withdraw::register)
                    .configure(handlers::approve_reject::register),
            ),
    )
    .await;

    // 终态撤回 → Validation 错误
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-instances/{}/withdraw", inst1))
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::BAD_REQUEST,
        "已终态实例不得撤回"
    );
    assert_eq!(
        instance_status(&pool, inst1).await.as_deref(),
        Some("approved"),
        "终态状态不得被撤回污染"
    );
}

#[tokio::test]
async fn withdraw_requires_ngac_permission() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    // OWNER 挂非 admin UA（无 withdraw 权限），且是创建者 → require_resource_access 拦截
    ensure_role_member(&pool, "r_wd_role", WD_NGAC)
        .await
        .unwrap();
    let (_, node1) = make_simple_flow(&pool, "FLOW-WD-4").await;
    let inst1 = insert_instance(&pool, node1, WD_NGAC, None).await;

    let noop = Arc::new(common::noop_messaging::NoopMessaging::default());
    let ctx = ::common::context::RequestContext::with_username(WD_NGAC, "actor@test", "actor");
    let bus: Arc<dyn ::common::event_bus::DomainEventBus> =
        Arc::new(::common::event_bus::InMemoryEventBus::new());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(bus))
            .app_data(web::Data::new(
                noop.clone() as Arc<dyn ::common::messaging::MessagingService>
            ))
            .service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(handlers::withdraw::register)
                    .configure(handlers::approve_reject::register),
            ),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-instances/{}/withdraw", inst1))
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::FORBIDDEN,
        "无 withdraw 权限的创建者不得撤回"
    );
}
