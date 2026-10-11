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
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool.clone(), ("JC", "FTA", "↑_NA"))
            .await
            .unwrap();
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_proc-approve" (notice, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, 1, $2, $3, $4) RETURNING id"#,
    )
    .bind(name)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 创建审批节点（桥链模型）：even 语义行 + oper 主体 + rro 在册锚 +
/// 模板桥（rr_event）。返回 even id。
async fn insert_event(pool: &PgPool, label: &str, flow_id: i64, lk_urgent: Option<i64>) -> i64 {
    // 坐标三元组（§6.12 声明即必须）：库内既有行实证 JC/FTA/↑_NA；值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (even_dk_scene, even_dk_factor, even_dk_function) =
        ontology_binding::resolve(&pool.clone(), ("JC", "FTA", "↑_NA"))
            .await
            .unwrap();
    let even_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_appr-process" (notice, lk_urgent, created_by_id, _t_, dk_scene, dk_factor, dk_function)
                   VALUES ($1, $2, 1, 'flow-context', $3, $4, $5) RETURNING id"#,
    )
    .bind(label)
    .bind(lk_urgent)
    .bind(even_dk_scene)
    .bind(even_dk_factor)
    .bind(even_dk_function)
    .fetch_one(pool)
    .await
    .unwrap();
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("JE", "FTA", "↓_EZ"))
        .await
        .unwrap();
    let op_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES ($1, 1, '实现', '范例', $2, $3, $4) RETURNING id"#,
    )
    .bind(label)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
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
           VALUES (isahl.gen_next_uid(267), $1, $2, 1)"#,
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
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("ZJ", "LNC", "↓_EH"))
        .await
        .unwrap();
    let person: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_empl-natural"
           (notice, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES ($1, 1, '实现', '范例', $2, $3, $4) RETURNING id"#,
    )
    .bind(name)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .unwrap();
    // 该雇员同时持有登录账号（同一 id）：岗位任职账号判据要求账号在册且活跃
    // （fix-approver-incumbent-source：任职账号集合 = 标量 ∪ 桥派生，仅计 is_active 账号）
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, username, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, $2, 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(person)
    .bind(format!("test-user-{person}"))
    .execute(pool)
    .await
    .expect("insert employee account");
    person
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
            r#"INSERT INTO isahl."zc_id_stus-approve" (id, code, notice, flag)
               VALUES (isahl.gen_next_uid(73), $1, $2, 'end') RETURNING id"#,
        )
        .bind(code)
        .bind(notice)
        .fetch_one(pool)
        .await
        .unwrap(),
    };
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (id, ref_left, ref_right)
           VALUES (isahl.gen_next_uid(260), $1, $2)"#,
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
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("JE", "FTA", "↓_EZ"))
        .await
        .unwrap();
    let instance_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, fk_subject, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, 1, '实现', '实例', $3, $4, $5) RETURNING id"#,
    )
    .bind(notice)
    .bind(fk_subject)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .unwrap();
    if let Some(ev) = event_id {
        sqlx::query(
            r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
               VALUES (isahl.gen_next_uid(267), $1, $2, 1)"#,
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
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("JC", "FTA", "↓_NC"))
        .await
        .unwrap();
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_deta-opinion" (notice, fk_list, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, 1, $3, $4, $5) RETURNING id"#,
    )
    .bind(notice)
    .bind(fk_event)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 授予 actor admin UA（`scope=all` 管理视角门禁需要）：复用测试 common 的
/// `ensure_role_member`（幂等建 auth_users 行 + admin UA 绑定）。
async fn grant_admin(pool: &PgPool, user_id: i64) {
    common::ensure_role_member(pool, "admin", user_id)
        .await
        .expect("grant admin UA");
}

#[tokio::test]
async fn test_enriched_handler_default_list() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("hdlr-def");

    let flow = insert_flow(&pool, &format!("flow-{code}")).await;
    let emp = insert_employee(&pool, "测试用户").await;
    grant_admin(&pool, emp).await; // scope=all = 管理视角（admin UA 门禁）
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
        // `q=<唯一 code>`：断言面限定到本测试自己的行——共享测试库会积累历次运行/并行用例的行，
        // 否则本行可能落到第 1 页之外（历史 flake）。
        .uri(&format!(
            "/test/approval-instances/enriched?scope=all&q={code}"
        ))
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
    grant_admin(&pool, emp).await; // scope=all = 管理视角（admin UA 门禁）

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
        .uri(&format!(
            "/test/approval-instances/enriched?scope=all&status=approved&q={code}"
        ))
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
    grant_admin(&pool, emp).await; // scope=all = 管理视角（admin UA 门禁）

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
        .uri(&format!(
            "/test/approval-instances/enriched?scope=all&q={code}"
        ))
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
    // 7a81761c4a 申请人归用户 id 空间：fk_previous NULL → applicant=创建者 username。
    // 镜像 handler 解析优先级（enriched_instance.rs:155-171）：员工主体 notice → 账号显示名
    let sys_username: String = sqlx::query_scalar(
        r#"SELECT COALESCE(
             (SELECT ee.notice FROM isahl."zc_id_subj-employee" ee
              WHERE ee.deleted_at IS NULL AND (ee.fk_user = 1 OR ee.id = 1)
              ORDER BY (ee.fk_user = 1) DESC LIMIT 1),
             (SELECT COALESCE(u.display_name, u.username) FROM isahl_auth.auth_users u WHERE u.id = 1)
           )"#,
    )
    .fetch_one(&pool)
    .await
    .expect("system user");
    assert_eq!(my["applicant"], sys_username, "applicant=创建者 username");
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
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool.clone(), ("JC", "FTA", "↑_NA"))
            .await
            .unwrap();
    let op_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, 1, $2, $3, $4) RETURNING id"#,
    )
    .bind(format!("op-{code}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_uid(267), $1, $2, 1)"#,
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
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("TX", "FJA", "↓_GG"))
            .await
            .unwrap();
    let pos_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_subj-position" (notice, fk_user, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, 1, $3, $4, $5) RETURNING id"#,
    )
    .bind(format!("pos-{code}"))
    .bind(emp)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
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
    grant_admin(&pool, emp).await; // scope=all = 管理视角（admin UA 门禁）
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

/// 置顶契约（用户裁决 2026-09-27）：注册 / 认证 / 授权的相关审批在「我的审批」恒定居首，
/// 优先于**更晚创建**的普通实例 —— 即排序不是单纯的 `created_at DESC`。
#[tokio::test]
async fn test_enriched_list_pins_certification_flow_instances_first() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("hdlr-pin");

    // 置顶流程 = 认证域三流程之一（先建 ⇒ 时间上更早；仅靠 created_at DESC 不会居首）
    let pinned_flow = insert_flow(&pool, &format!("flow-pin-{code}")).await;
    sqlx::query(
        r#"UPDATE isahl."zc_id_proc-approve" SET code = 'FLOW-AUTHORIZATION' WHERE id = $1"#,
    )
    .bind(pinned_flow)
    .execute(&pool)
    .await
    .unwrap();
    let pinned_event = insert_event(&pool, &format!("ev-pin-{code}"), pinned_flow, None).await;
    let pinned =
        insert_instance(&pool, &format!("inst-pin-{code}"), Some(pinned_event), None).await;

    // 普通流程（后建 ⇒ 按 created_at DESC 本应居首）
    let plain_flow = insert_flow(&pool, &format!("flow-plain-{code}")).await;
    let plain_event = insert_event(&pool, &format!("ev-plain-{code}"), plain_flow, None).await;
    let plain = insert_instance(
        &pool,
        &format!("inst-plain-{code}"),
        Some(plain_event),
        None,
    )
    .await;

    grant_admin(&pool, plain).await; // scope=all = 管理视角（admin UA 门禁）
    let ctx = ::common::context::RequestContext::with_username(plain, "actor@test", "actor");
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

    // `q=<本测试唯一 code>`：把断言面限定到本测试自己创建的两行——
    // 否则 `scope=all` 会读到**同库并行用例**的其它置顶行（认证域注册审批），
    // 那条更新的行会把 pinned 挤到第 2 位 ⇒ 断言语义正确但结果依赖调度（历史 flake）。
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!(
                "/test/approval-instances/enriched?scope=all&pageSize=500&q={code}"
            ))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200, "enriched 应 200");
    let body: Value = test::read_body_json(resp).await;
    let ids: Vec<i64> = body["items"]
        .as_array()
        .expect("items array")
        .iter()
        .filter_map(|i| {
            i["id"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| i["id"].as_i64())
        })
        .collect();
    let pinned_pos = ids
        .iter()
        .position(|id| *id == pinned)
        .unwrap_or_else(|| panic!("pinned instance {pinned} in list: {}", ids.len()));
    let plain_pos = ids
        .iter()
        .position(|id| *id == plain)
        .unwrap_or_else(|| panic!("plain instance {plain} in list: {}", ids.len()));
    assert_eq!(pinned_pos, 0, "认证域流程实例必须居首");
    assert!(pinned_pos < plain_pos, "置顶优先于时间序");
}

/// `codes` 白名单过滤（管理端注册审批队列范围限定）：
/// 仅返回 code 命中白名单的实例，其它 code（如实名 `user-verify`）不得出现。
#[tokio::test]
async fn test_enriched_codes_filter_restricts_instances() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("hdlr-codes");

    let flow = insert_flow(&pool, &format!("flow-{code}")).await;
    let ev = insert_event(&pool, &format!("ev-{code}"), flow, None).await;
    let hit = insert_instance(&pool, &format!("inst-hit-{code}"), Some(ev), None).await;
    let miss = insert_instance(&pool, &format!("inst-miss-{code}"), Some(ev), None).await;
    sqlx::query(
        r#"UPDATE isahl."zc_id_oper-approve" SET code = 'user-register-approval' WHERE id = $1"#,
    )
    .bind(hit)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(r#"UPDATE isahl."zc_id_oper-approve" SET code = 'user-verify' WHERE id = $1"#)
        .bind(miss)
        .execute(&pool)
        .await
        .unwrap();

    let actor = insert_employee(&pool, "codes 管理员").await;
    grant_admin(&pool, actor).await;
    let ctx = ::common::context::RequestContext::with_username(actor, "actor@test", "actor");
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
            .uri("/test/approval-instances/enriched?scope=all&codes=user-register-approval")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200, "codes 过滤应 200");
    let body: Value = test::read_body_json(resp).await;
    let ids: Vec<i64> = body["items"]
        .as_array()
        .expect("items array")
        .iter()
        .filter_map(|i| {
            i["id"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| i["id"].as_i64())
        })
        .collect();
    assert!(ids.contains(&hit), "白名单 code 实例应在结果中: {ids:?}");
    assert!(
        !ids.contains(&miss),
        "非白名单 code（user-verify）不得出现: {ids:?}"
    );
}

/// `codes` 超过上限 → 400（MUST NOT 静默截断——否则调用方以为过滤生效）
#[tokio::test]
async fn test_enriched_codes_too_many_rejected() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let actor = insert_employee(&pool, "codes 超限").await;
    grant_admin(&pool, actor).await;
    let ctx = ::common::context::RequestContext::with_username(actor, "actor@test", "actor");
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

    let codes = (0..21)
        .map(|i| format!("c{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!(
                "/test/approval-instances/enriched?scope=all&codes={codes}"
            ))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 400, "codes 超上限应 400");
}

/// `scope=all`（管理视角全量）MUST 持 admin UA：非 admin 403，授予后 200。
#[tokio::test]
async fn test_enriched_scope_all_requires_admin() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("hdlr-gate");

    let flow = insert_flow(&pool, &format!("flow-{code}")).await;
    let ev = insert_event(&pool, &format!("ev-{code}"), flow, None).await;
    insert_instance(&pool, &format!("inst-{code}"), Some(ev), None).await;

    let actor = insert_employee(&pool, "非管理员").await;
    let ctx = ::common::context::RequestContext::with_username(actor, "actor@test", "actor");
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
            .uri("/test/approval-instances/enriched?scope=all")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 403, "非 admin 不得读取管理视角全量");

    // 授予 admin UA 后同一 actor 应放行
    grant_admin(&pool, actor).await;
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/test/approval-instances/enriched?scope=all")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200, "admin UA 持有者应放行");
}

/// 重提链（`original_id`）：创建入参双形态兼容 → 落 `meta.original_id` → enriched 回带字符串；
/// 无链实例 `meta` 保持 NULL 且回带 null。
#[tokio::test]
async fn test_reappeal_original_link_roundtrip() {
    use approval::models::{ApprovalInstance, CreateApprovalInstanceRequest};
    use approval::repositories::ApprovalInstanceRepository;
    use approval::services::ApprovalInstanceService;

    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("hdlr-link");

    let flow = insert_flow(&pool, &format!("flow-{code}")).await;
    let emp = insert_employee(&pool, "链接用户").await;
    grant_admin(&pool, emp).await;
    let event = insert_event(&pool, &format!("ev-{code}"), flow, None).await;
    // 来源实例（被驳回后「再申请」指向它）
    let source = insert_instance(&pool, &format!("src-{code}"), Some(event), Some(emp)).await;

    // 双形态反序列化：字符串 / 数字都必须接受（common::serde_zuid::opt）
    let by_number: CreateApprovalInstanceRequest = serde_json::from_value(serde_json::json!({
        "node_name": format!("再申请: src-{code}"),
        "original_id": source,
    }))
    .expect("数字形态 original_id 应被接受");
    assert_eq!(by_number.original_id, Some(source), "数字形态解析为同一 id");
    let by_text: CreateApprovalInstanceRequest = serde_json::from_value(serde_json::json!({
        "node_name": format!("再申请: src-{code}"),
        "original_id": source.to_string(),
    }))
    .expect("字符串形态 original_id 应被接受");
    assert_eq!(by_text.original_id, Some(source), "字符串形态解析为同一 id");

    let svc = ApprovalInstanceService::new(ApprovalInstanceRepository::new(pool.clone()));
    let reappeal: ApprovalInstance = svc.create(by_number, emp).await.expect("带链创建");
    let stored: Option<String> = sqlx::query_scalar(
        r#"SELECT meta->>'original_id' FROM isahl."zc_id_oper-approve" WHERE id = $1"#,
    )
    .bind(reappeal.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, Some(source.to_string()), "meta.original_id 落值");

    let plain: ApprovalInstance = svc
        .create(
            CreateApprovalInstanceRequest {
                node_name: format!("plain-{code}"),
                code: None,
                fk_approve: None,
                original_id: None,
                comments: None,
            },
            emp,
        )
        .await
        .expect("无链创建");
    let plain_meta: Option<serde_json::Value> =
        sqlx::query_scalar(r#"SELECT meta FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
            .bind(plain.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(plain_meta.is_none(), "无链实例 meta 保持 NULL");

    // update 路径（同样含 raw 字符串 SQL）：改标题后仍能读写（回归：行尾反斜杠字面量曾致语法错误）
    let updated = svc
        .update(
            reappeal.id,
            approval::models::UpdateApprovalInstanceRequest {
                node_name: Some(format!("再申请(改): src-{code}")),
                code: None,
                comments: None,
            },
            emp,
        )
        .await
        .expect("更新实例")
        .expect("实例存在");
    assert_eq!(updated.id, reappeal.id, "update 返回同一实例");

    // enriched 回带：带链 = 来源 id 字符串；无链 = null
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
            // `q=<唯一 code>`：只看本测试自己创建的行（共享库含历次运行/并行用例数据）
            .uri(&format!(
                "/test/approval-instances/enriched?scope=all&q={code}"
            ))
            .to_request(),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "enriched handler should return 200"
    );
    let body: Value = test::read_body_json(resp).await;
    let items = body["items"].as_array().expect("items array");
    let pick = |id: i64| {
        items
            .iter()
            .find(|i| {
                i["id"]
                    .as_str()
                    .and_then(|s| s.parse::<i64>().ok())
                    .or_else(|| i["id"].as_i64())
                    == Some(id)
            })
            .unwrap_or_else(|| panic!("instance {id} in response"))
    };
    assert_eq!(
        pick(reappeal.id)["original_id"],
        Value::String(source.to_string()),
        "带链实例回带字符串形态的来源 id"
    );
    assert!(pick(plain.id)["original_id"].is_null(), "无链实例回带 null");
    // 计数/页码为数字（本 change 去掉误加的 serde_zuid）
    assert!(body["total"].as_i64().is_some(), "total 为 JSON 数字");
    assert!(body["page"].as_i64().is_some(), "page 为 JSON 数字");
}
