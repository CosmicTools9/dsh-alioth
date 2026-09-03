//! Enriched Instance Handler — actix_web 集成测试
//!
//! 测试真实 HTTP handler 而不是精简 SQL 副本，
//! 覆盖路由注册、响应 envelope、状态推导、applicant 解析。
//!
//! 语义：fk_approve → zc_id_even-approve（审批事件），非 zc_id_process
//! lk_urgent 在 zc_id_even-approve 上

use ::common::testing::connect_test_db;
use actix_web::{dev::Service, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::Value;
use sqlx::PgPool;
mod common;
use common::{setup_test_schema, test_code};

/// 创建紧急级别 (zc_id_leve-urgent)
async fn insert_urgency(pool: &PgPool, notice: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_leve-urgent" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(notice)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 创建流程 (zc_id_process)
async fn insert_flow(pool: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl.zc_id_process (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 创建审批节点（桥链模型）：even 语义行 + oper 主体 + rro 在册锚 +
/// 模板桥（rr_event）。返回 even id。
async fn insert_event(pool: &PgPool, label: &str, flow_id: i64, lk_urgent: Option<i64>) -> i64 {
    let even_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, lk_urgent, created_by_id)
           VALUES ($1, $2, 1) RETURNING id"#,
    )
    .bind(label)
    .bind(lk_urgent)
    .fetch_one(pool)
    .await
    .unwrap();
    let op_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(label)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_process_rr_operation
           (id, code, ref_left, ref_right, comments, created_by_id)
           VALUES (isahl.gen_next_uid(791), 'approve', $1, $2, $3, 1)"#,
    )
    .bind(flow_id)
    .bind(op_id)
    .bind(label)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(op_id)
    .bind(even_id)
    .execute(pool)
    .await
    .unwrap();
    even_id
}

/// 创建工程师
async fn insert_employee(pool: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_subj-employee" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 生命周期主状态桥直插（D7 桥真值语义：derived_status 不再按意见派生）
async fn set_status(pool: &PgPool, instance_id: i64, code: &str, notice: &str) {
    let sid: i64 = match sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT id FROM isahl."zc_id_stus-approve" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(code)
    .fetch_optional(pool)
    .await
    .unwrap()
    .flatten()
    {
        Some(id) => id,
        None => sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO isahl."zc_id_stus-approve" (id, code, notice)
               VALUES (isahl.gen_next_zuid(), $1, $2) RETURNING id"#,
        )
        .bind(code)
        .bind(notice)
        .fetch_one(pool)
        .await
        .unwrap(),
    };
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (id, ref_left, ref_right)
           VALUES (isahl.gen_next_zuid(), $1, $2)"#,
    )
    .bind(instance_id)
    .bind(sid)
    .execute(pool)
    .await
    .unwrap();
}

/// 创建审批实例 (zc_id_oper-approve) — 实例经 rr_event 桥挂审批事件 (zc_id_even-approve)
async fn insert_instance(
    pool: &PgPool,
    notice: &str,
    event_id: Option<i64>,
    fk_subject: Option<i64>,
) -> i64 {
    let instance_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, fk_subject, created_by_id)
           VALUES ($1, $2, 1) RETURNING id"#,
    )
    .bind(notice)
    .bind(fk_subject)
    .fetch_one(pool)
    .await
    .unwrap();
    if let Some(ev) = event_id {
        sqlx::query(
            r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
        )
        .bind(instance_id)
        .bind(ev)
        .execute(pool)
        .await
        .unwrap();
    }
    instance_id
}

/// 创建审批意见 (zc_id_deta-opinion) — fk_list → 事件 ID
async fn insert_action(pool: &PgPool, notice: &str, fk_event: i64) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_deta-opinion" (notice, fk_list, created_by_id)
           VALUES ($1, $2, 1) RETURNING id"#,
    )
    .bind(notice)
    .bind(fk_event)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn test_enriched_handler_default_list() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("hdlr-def");

    let flow = insert_flow(&pool, &format!("flow-{code}")).await;
    let emp = insert_employee(&pool, "测试用户").await;
    // 创建紧急级别并获取 ID
    let urg_id = insert_urgency(&pool, "high").await;
    // 创建审批事件，引用紧急级别 → priority=high
    let event = insert_event(&pool, &format!("ev-{code}"), flow, Some(urg_id)).await;
    // 实例引用事件
    let inst = insert_instance(&pool, &format!("inst-{code}"), Some(event), Some(emp)).await;
    // 意见引用实例（enriched 派生 status 按 a.fk_list = i.id 查）
    insert_action(&pool, "审批通过", inst).await;
    // D7：状态以生命周期桥为准（意见不再派生 status）
    set_status(&pool, inst, "approved", "已通过").await;

    let ctx = ::common::context::RequestContext::with_username(emp, "actor@test", "actor");
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(handlers::enriched_instance::register),
        ),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/test/approval-instances/enriched?scope=all")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(
        resp.status().is_success(),
        "enriched handler should return 200"
    );

    let body: Value = test::read_body_json(resp).await;
    let items = body["items"]
        .as_array()
        .expect("response should have items array");
    let my = items
        .iter()
        .find(|i| {
            i["id"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| i["id"].as_i64())
                == Some(inst)
        })
        .expect("test instance in response");

    assert_eq!(my["node_name"], format!("inst-{code}"), "node_name matches");
    assert_eq!(
        my["status"], "approved",
        "approved action → status=approved"
    );
    assert_eq!(
        my["result"], "approved",
        "approved action → result=approved"
    );
    assert_eq!(my["applicant"], "测试用户", "applicant resolved");
    assert_eq!(my["priority"], "high", "lk_urgent=2 → priority=high");
    let total = body["total"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| body["total"].as_i64());
    assert!(total.is_some(), "total present");
    let page = body["page"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| body["page"].as_i64());
    assert_eq!(page, Some(1), "page defaults to 1");
    let page_size = body["page_size"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| body["page_size"].as_i64());
    assert_eq!(page_size, Some(50), "page_size defaults to 50");
}

#[tokio::test]
async fn test_enriched_handler_status_filter() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("hdlr-flt");

    let flow_a = insert_flow(&pool, &format!("fa-{code}")).await;
    let event_a = insert_event(&pool, &format!("ev-a-{code}"), flow_a, None).await;
    let inst_a =
        insert_instance(&pool, &format!("inst-approved-{code}"), Some(event_a), None).await;
    // 意见引用实例；D7：状态以生命周期桥为准（意见不再派生 status）
    insert_action(&pool, "审批通过", inst_a).await;
    set_status(&pool, inst_a, "approved", "已通过").await;

    let flow_p = insert_flow(&pool, &format!("fp-{code}")).await;
    let event_p = insert_event(&pool, &format!("ev-p-{code}"), flow_p, None).await;
    insert_instance(&pool, &format!("inst-pending-{code}"), Some(event_p), None).await;
    let emp = insert_employee(&pool, "测试用户2").await;

    let ctx = ::common::context::RequestContext::with_username(emp, "actor@test", "actor");
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(handlers::enriched_instance::register),
        ),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/test/approval-instances/enriched?scope=all&status=approved")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body: Value = test::read_body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    let approved = items
        .iter()
        .find(|i| i["node_name"].as_str().is_some_and(|n| n.contains(&code)));
    assert!(
        approved.is_some(),
        "test-approved instance should appear with status=approved filter"
    );
    let a = approved.unwrap();
    assert_eq!(a["status"], "approved");
    let total = body["total"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| body["total"].as_i64())
        .unwrap_or(0);
    assert!(items.len() <= total as usize);
}

#[tokio::test]
async fn test_enriched_handler_pending_status() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("hdlr-pend");

    let flow = insert_flow(&pool, &format!("flow-{code}")).await;
    let event = insert_event(&pool, &format!("ev-{code}"), flow, None).await;
    insert_instance(&pool, &format!("inst-{code}"), Some(event), None).await;
    let emp = insert_employee(&pool, "测试用户3").await;

    let ctx = ::common::context::RequestContext::with_username(emp, "actor@test", "actor");
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(handlers::enriched_instance::register),
        ),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/test/approval-instances/enriched?scope=all")
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body: Value = test::read_body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    let my = items
        .iter()
        .find(|i| i["node_name"] == format!("inst-{code}"))
        .expect("instance in response");

    assert_eq!(my["status"], "pending", "no action → status=pending");
    assert_eq!(my["result"], "pending", "no action → result=pending");
    assert_eq!(
        my["applicant"],
        serde_json::Value::Null,
        "no fk_subject → applicant=null"
    );
    assert_eq!(
        my["priority"],
        serde_json::Value::Null,
        "no lk_urgent → priority=null"
    );
}

/// todo scope：待办过滤 = fk_operator=我 OR（实例桥→模板桥→operation→岗位桥→position.fk_user=我）
/// 无岗位关联的实例不得出现在 todo 列表（scope=all 才可见）
#[tokio::test]
async fn test_enriched_todo_scope_filters_by_assignee() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("hdlr-todo");

    let flow = insert_flow(&pool, &format!("flow-{code}")).await;
    let emp = insert_employee(&pool, "待办人").await;
    // 有岗位链的事件（assignable）与无岗位链事件（unrelated）
    let ev_assign = insert_event(&pool, &format!("ev-a-{code}"), flow, None).await;
    let ev_plain = insert_event(&pool, &format!("ev-p-{code}"), flow, None).await;
    let inst_assign =
        insert_instance(&pool, &format!("inst-a-{code}"), Some(ev_assign), Some(emp)).await;
    let inst_plain =
        insert_instance(&pool, &format!("inst-p-{code}"), Some(ev_plain), Some(emp)).await;

    // 岗位链：operation ← 模板桥(ref_left=op, ref_right=模板) + rr_approve(op→position, position.fk_user=emp)
    let op_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_operation (notice, created_by_id) VALUES ($1, 1) RETURNING id"#,
    )
    .bind(format!("op-{code}"))
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(op_id)
    .bind(ev_assign)
    .execute(&pool)
    .await
    .unwrap();
    // 在册锚：todo 过滤的桥链判据要求 op 经 process_rr_operation 锚定流程
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_process_rr_operation
           (id, code, ref_left, ref_right, comments, created_by_id)
           VALUES (isahl.gen_next_uid(791), 'approve', $1, $2, $3, 1)"#,
    )
    .bind(flow)
    .bind(op_id)
    .bind(format!("op-{code}"))
    .execute(&pool)
    .await
    .unwrap();
    let pos_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_subj-position" (notice, fk_user, created_by_id)
           VALUES ($1, $2, 1) RETURNING id"#,
    )
    .bind(format!("pos-{code}"))
    .bind(emp)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_operation_rr_approve" (ref_left, ref_right, created_by_id)
           VALUES ($1, $2, 1)"#,
    )
    .bind(op_id)
    .bind(pos_id)
    .execute(&pool)
    .await
    .unwrap();

    let ctx = ::common::context::RequestContext::with_username(emp, "actor@test", "actor");
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(handlers::enriched_instance::register),
        ),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/test/approval-instances/enriched?scope=todo&status=pending")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200, "todo scope 应 200");
    let body: serde_json::Value = test::read_body_json(resp).await;
    let items = body["items"].as_array().expect("items array");
    let ids: Vec<i64> = items
        .iter()
        .filter_map(|i| {
            i["id"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| i["id"].as_i64())
        })
        .collect();
    assert!(
        ids.contains(&inst_assign),
        "岗位链实例应在 todo 列表: {ids:?}"
    );
    assert!(
        !ids.contains(&inst_plain),
        "无岗位链实例不得出现在 todo 列表: {ids:?}"
    );
}

/// q 模糊搜索：命中 node_name（实例 notice）或 applicant_name（申请人姓名）
#[tokio::test]
async fn test_enriched_q_search_filters() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("hdlr-q");

    let flow = insert_flow(&pool, &format!("flow-{code}")).await;
    let emp = insert_employee(&pool, "张全有").await;
    let ev = insert_event(&pool, &format!("ev-{code}"), flow, None).await;
    let _hit = insert_instance(&pool, "付款申请-华东", Some(ev), Some(emp)).await;
    let _miss = insert_instance(&pool, "请假申请", Some(ev), Some(emp)).await;

    let ctx = ::common::context::RequestContext::with_username(emp, "actor@test", "actor");
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(handlers::enriched_instance::register),
        ),
    )
    .await;

    // 节点名命中
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/test/approval-instances/enriched?scope=all&q=%E4%BB%98%E6%AC%BE")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = test::read_body_json(resp).await;
    let names: Vec<&str> = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|i| i["node_name"].as_str())
        .collect();
    assert!(
        names.iter().any(|n| n.contains("付款")),
        "q 应命中节点名: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.contains("请假")),
        "q 不应命中无关节点: {names:?}"
    );

    // 申请人命中
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/test/approval-instances/enriched?scope=all&q=%E5%BC%A0%E5%85%A8")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = test::read_body_json(resp).await;
    // total 序列化为 zuid 字符串（serde_zuid），字符串/数字双形态解析
    let total = body["total"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| body["total"].as_i64())
        .unwrap_or(0);
    assert!(total >= 1, "q 按申请人姓名应命中: total={total}");
}

/// todo scope admin 管理视角：持 admin UA 的用户按管理视角全量可见（含指派给
/// 他人的实例）；非 admin 用户仍按处理人/岗位链隔离（见
/// test_enriched_todo_scope_filters_by_assignee）。
#[tokio::test]
async fn test_enriched_todo_scope_admin_sees_all() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("hdlr-admin");

    let flow = insert_flow(&pool, &format!("flow-{code}")).await;
    let emp = insert_employee(&pool, "待办人").await;
    let ev = insert_event(&pool, &format!("ev-{code}"), flow, None).await;
    // 实例指派给 emp（他人实例，admin 不应因 fk_operator 命中）
    let inst = insert_instance(&pool, &format!("inst-{code}"), Some(ev), Some(emp)).await;

    // admin 用户：绑定 admin UA（ensure_role_member 幂等预置 UA + auth_users 行）
    let admin_user: i64 = 4_000_000_000 + (std::process::id() as i64 % 100_000) * 10 + 1;
    common::ensure_role_member(&pool, "admin", admin_user)
        .await
        .unwrap();

    let ctx = ::common::context::RequestContext::with_username(admin_user, "admin@test", "admin");
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(handlers::enriched_instance::register),
        ),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/test/approval-instances/enriched?scope=todo&status=pending")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200, "todo scope 应 200");
    let body: serde_json::Value = test::read_body_json(resp).await;
    let items = body["items"].as_array().expect("items array");
    let ids: Vec<i64> = items
        .iter()
        .filter_map(|i| {
            i["id"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| i["id"].as_i64())
        })
        .collect();
    assert!(
        ids.contains(&inst),
        "admin UA 用户应可见他人实例（管理视角）: {ids:?}"
    );
}
