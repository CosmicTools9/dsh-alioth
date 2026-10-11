//! Approval 集成测试 — T1.4
//!
//! 覆盖：
//! - T1.1 overview 审批按用户过滤（fk_operator / created_by_id）+ mine 维度 + counts
//! - T1.2 approve 鉴权（非处理人 403）+ 意见落库（同事务）+ 状态双真相源同步
//! - T1.3 GET /approvals/{id} 意见链
//!
//! 使用 aliothstudio_test 数据库，负数 ID fixture，自建自清。

use ::common::testing::connect_test_db;
use actix_web::{body::to_bytes, web, HttpMessage};
use sqlx::PgPool;

use alioth_gateway::api::approvals;
use alioth_gateway::api::global_overview;
use common::context::RequestContext;
use common::event_bus::DomainEventBus as _; // publish 方法解析（trait 需在作用域）
use framework_workspace_approval::{ApprovalActor, ApprovalService};

// ── Fixture IDs（负数段，避免与真实数据冲突）─────────────────────────────────

const USER_A: i64 = -9901; // 处理人 + 发起人
const USER_B: i64 = -9902; // 他人
const APPR_OPERATOR_A: i64 = -99011; // fk_operator = A，created_by = B
const APPR_CREATED_A: i64 = -99012; // fk_operator = NULL，created_by = A
const APPR_OTHER: i64 = -99013; // fk_operator = B，created_by = B（A 不可见）

/// 插入审批 fixture（幂等：先清后插）
async fn setup_approval_fixtures(pool: &PgPool) {
    cleanup_approval_fixtures(pool).await;

    // 叶表坐标（§6.12）：审批实例行 dk 经静态绑定解析（循环外一次求得）
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("JE", "FTA", "↓_EZ"))
        .await
        .expect("resolve oper-approve coords");

    for (id, operator, creator, notice) in [
        (APPR_OPERATOR_A, Some(USER_A), USER_B, "T1.4待我审批"),
        (APPR_CREATED_A, None, USER_A, "T1.4我发起的"),
        (APPR_OTHER, Some(USER_B), USER_B, "T1.4他人审批"),
    ] {
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_oper-approve"
               (id, notice, created_at, updated_at, created_by_id, fk_operator, dk_scene, dk_factor, dk_function)
               VALUES ($1, $4, NOW(), NOW(), $3, $2, $5, $6, $7)"#,
        )
        .bind(id)
        .bind(operator)
        .bind(creator)
        .bind(notice)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .execute(pool)
        .await
        .expect("insert oper-approve fixture");
    }
}

async fn cleanup_approval_fixtures(pool: &PgPool) {
    let ids = [APPR_OPERATOR_A, APPR_CREATED_A, APPR_OTHER];
    // 意见链
    sqlx::query(r#"DELETE FROM isahl."zc_id_deta-opinion" WHERE fk_list = ANY($1)"#)
        .bind(&ids[..])
        .execute(pool)
        .await
        .ok();
    // 主状态关系
    sqlx::query(r#"DELETE FROM isahl."zc_id_lifecycle_r_primary-status" WHERE ref_left = ANY($1)"#)
        .bind(&ids[..])
        .execute(pool)
        .await
        .ok();
    // 审批单
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = ANY($1)"#)
        .bind(&ids[..])
        .execute(pool)
        .await
        .ok();
}

fn req_with_user(user_id: i64) -> actix_web::HttpRequest {
    let req = actix_web::test::TestRequest::default().to_http_request();
    req.extensions_mut().insert(RequestContext::new(
        user_id,
        format!("user{}@test.local", user_id),
    ));
    req
}

/// 调 overview handler 并解析 data.approvals 的 (id, mine, status) 三元组
async fn overview_approvals(pool: &PgPool, user_id: i64) -> Vec<(i64, bool, String)> {
    let req = req_with_user(user_id);
    let resp = global_overview::get_global_overview(req, web::Data::new(pool.clone()))
        .await
        .expect("overview handler");
    let body = to_bytes(resp.into_body()).await.expect("body bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("overview json");

    json["data"]["approvals"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|item| {
            let id = item["id"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| item["id"].as_i64())
                .unwrap_or(0);
            (
                id,
                item["mine"].as_bool().unwrap_or(false),
                item["status"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

// ── T1.1: overview 用户过滤 + mine + counts ─────────────────────────────────

#[tokio::test]
async fn t1_4_overview_scoped_to_user_with_mine() {
    let pool = connect_test_db().await;
    setup_approval_fixtures(&pool).await;

    let items_a = overview_approvals(&pool, USER_A).await;
    let ids_a: Vec<i64> = items_a.iter().map(|(id, _, _)| *id).collect();

    assert!(
        ids_a.contains(&APPR_OPERATOR_A),
        "USER_A 应看到待我审批 APPR_OPERATOR_A"
    );
    assert!(
        ids_a.contains(&APPR_CREATED_A),
        "USER_A 应看到我发起的 APPR_CREATED_A"
    );
    assert!(
        !ids_a.contains(&APPR_OTHER),
        "USER_A 不应看到他人审批 APPR_OTHER"
    );

    // mine 标记：APPR_CREATED_A 由 A 创建 → mine=true；APPR_OPERATOR_A 由 B 创建 → mine=false
    let created = items_a.iter().find(|(id, _, _)| *id == APPR_CREATED_A);
    assert_eq!(
        created.map(|(_, mine, _)| *mine),
        Some(true),
        "APPR_CREATED_A 应标记 mine=true"
    );
    let operator = items_a.iter().find(|(id, _, _)| *id == APPR_OPERATOR_A);
    assert_eq!(
        operator.map(|(_, mine, _)| *mine),
        Some(false),
        "APPR_OPERATOR_A 应标记 mine=false"
    );

    // counts：pending_total 为独立 COUNT（不受列表 LIMIT 截断），至少覆盖 fixture
    let req = req_with_user(USER_A);
    let resp = global_overview::get_global_overview(req, web::Data::new(pool.clone()))
        .await
        .expect("overview handler");
    let body = to_bytes(resp.into_body()).await.expect("body bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("overview json");
    let pending_total = json["data"]["counts"]["pending_total"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| json["data"]["counts"]["pending_total"].as_i64())
        .expect("counts.pending_total");
    assert!(
        pending_total >= 2,
        "pending_total 应 >= 2（两个待办 fixture），got {}",
        pending_total
    );

    cleanup_approval_fixtures(&pool).await;
}

// ── T1.2: approve 鉴权 403 ──────────────────────────────────────────────────

#[tokio::test]
async fn t1_4_approve_forbidden_for_non_operator() {
    let pool = connect_test_db().await;
    setup_approval_fixtures(&pool).await;

    // USER_B 不是 APPR_OPERATOR_A 的处理人（fk_operator = USER_A）
    let resp = ApprovalService::execute(
        &pool,
        APPR_OPERATOR_A,
        "approved",
        Some(ApprovalActor {
            user_id: USER_B,
            opinion: None,
        }),
        None,
    )
    .await;

    assert!(!resp.success, "非处理人操作应失败");
    assert_eq!(
        resp.message, "APPROVAL_NOT_OPERATOR",
        "失败原因应为 APPROVAL_NOT_OPERATOR"
    );

    // 无意见落库、无主状态关系
    let opinion_count: i64 =
        sqlx::query_scalar(r#"SELECT COUNT(*) FROM isahl."zc_id_deta-opinion" WHERE fk_list = $1"#)
            .bind(APPR_OPERATOR_A)
            .fetch_one(&pool)
            .await
            .expect("count opinions");
    assert_eq!(opinion_count, 0, "鉴权失败不得写入意见");

    let status_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_lifecycle_r_primary-status" WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(APPR_OPERATOR_A)
    .fetch_one(&pool)
    .await
    .expect("count status");
    assert_eq!(status_count, 0, "鉴权失败不得写入主状态");

    cleanup_approval_fixtures(&pool).await;
}

// ── T1.2: approve 意见落库 + 状态同步（消除双真相源）─────────────────────────

#[tokio::test]
async fn t1_4_approve_persists_opinion_and_syncs_overview() {
    let pool = connect_test_db().await;
    setup_approval_fixtures(&pool).await;

    let resp = ApprovalService::execute(
        &pool,
        APPR_OPERATOR_A,
        "approved",
        Some(ApprovalActor {
            user_id: USER_A,
            opinion: Some("同意，T1.4测试".to_string()),
        }),
        None,
    )
    .await;
    assert!(resp.success, "处理人 approve 应成功, got: {}", resp.message);

    // 意见落库：notice='审批通过'、opinion、fk_list、created_by_id
    let (notice, opinion, created_by): (String, String, i64) = sqlx::query_as(
        r#"SELECT notice, COALESCE(opinion, ''), created_by_id FROM isahl."zc_id_deta-opinion"
           WHERE fk_list = $1 AND deleted_at IS NULL ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(APPR_OPERATOR_A)
    .fetch_one(&pool)
    .await
    .expect("opinion row");
    assert_eq!(notice, "审批通过");
    assert_eq!(opinion, "同意，T1.4测试");
    assert_eq!(created_by, USER_A);

    // 主状态关系落库（approved 状态）
    let status_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_lifecycle_r_primary-status" ps
           JOIN isahl."zc_id_stus-approve" s ON s.id = ps.ref_right
           WHERE ps.ref_left = $1 AND ps.deleted_at IS NULL AND s.code = 'approved'"#,
    )
    .bind(APPR_OPERATOR_A)
    .fetch_one(&pool)
    .await
    .expect("status row");
    assert_eq!(status_count, 1, "主状态应为 approved");

    // 双真相源同步：overview 状态立即从 pending → approved
    let items = overview_approvals(&pool, USER_A).await;
    let item = items.iter().find(|(id, _, _)| *id == APPR_OPERATOR_A);
    assert_eq!(
        item.map(|(_, _, status)| status.as_str()),
        Some("approved"),
        "approve 后 overview 状态应同步为 approved"
    );

    cleanup_approval_fixtures(&pool).await;
}

// ── T1.5: handler 层 opinion body 透传（fix-workspace-dock-contracts P1-3）────

#[tokio::test]
async fn t1_5_handler_approve_propagates_opinion_body() {
    let pool = connect_test_db().await;
    setup_approval_fixtures(&pool).await;

    // 带 opinion body 调用 handler（模拟前端 POST {opinion: "同意，走流程"}）
    let req = req_with_user(USER_A);
    let path = actix_web::web::Path::from(APPR_OPERATOR_A);
    let body = actix_web::web::Json(approvals::OpinionBody {
        opinion: Some("同意，走流程".to_string()),
    });
    let resp = approvals::approve_approval(
        req,
        web::Data::new(pool.clone()),
        web::Data::new(std::sync::Arc::new(
            alioth_gateway::notification::db_messaging::DbMessagingService::new(pool.clone()),
        )
            as std::sync::Arc<dyn common::messaging::MessagingService>),
        web::Data::new(
            std::sync::Arc::new(common::event_bus::InMemoryEventBus::new())
                as std::sync::Arc<dyn common::event_bus::DomainEventBus>,
        ),
        path,
        Some(body),
    )
    .await;
    let resp_body = to_bytes(resp.into_body()).await.expect("body bytes");
    let json: serde_json::Value = serde_json::from_slice(&resp_body).expect("approve json");
    println!("T1.11 approve resp: {:?}", json);
    assert!(
        json["success"].as_bool().unwrap_or(false),
        "approve should succeed"
    );

    // 意见链含透传的 opinion
    let (action, opinion, created_by): (String, String, i64) = sqlx::query_as(
        r#"SELECT notice, COALESCE(opinion, ''), created_by_id FROM isahl."zc_id_deta-opinion"
           WHERE fk_list = $1 AND deleted_at IS NULL ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(APPR_OPERATOR_A)
    .fetch_one(&pool)
    .await
    .expect("opinion row");
    assert_eq!(action, "审批通过");
    assert_eq!(opinion, "同意，走流程", "handler 应透传 opinion body");
    assert_eq!(created_by, USER_A);

    cleanup_approval_fixtures(&pool).await;
}

#[tokio::test]
async fn t1_5_handler_approve_without_body_ok() {
    let pool = connect_test_db().await;
    setup_approval_fixtures(&pool).await;

    // 无 body（老客户端）→ opinion=None，兼容
    let req = req_with_user(USER_A);
    let path = actix_web::web::Path::from(APPR_OPERATOR_A);
    let resp = approvals::approve_approval(
        req,
        web::Data::new(pool.clone()),
        web::Data::new(std::sync::Arc::new(
            alioth_gateway::notification::db_messaging::DbMessagingService::new(pool.clone()),
        )
            as std::sync::Arc<dyn common::messaging::MessagingService>),
        web::Data::new(
            std::sync::Arc::new(common::event_bus::InMemoryEventBus::new())
                as std::sync::Arc<dyn common::event_bus::DomainEventBus>,
        ),
        path,
        None,
    )
    .await;
    let resp_body = to_bytes(resp.into_body()).await.expect("body bytes");
    let json: serde_json::Value = serde_json::from_slice(&resp_body).expect("approve json");
    assert!(
        json["success"].as_bool().unwrap_or(false),
        "approve without body should succeed"
    );

    let opinion: Option<String> = sqlx::query_scalar(
        r#"SELECT opinion FROM isahl."zc_id_deta-opinion"
           WHERE fk_list = $1 AND deleted_at IS NULL ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(APPR_OPERATOR_A)
    .fetch_one(&pool)
    .await
    .expect("opinion row");
    assert!(
        opinion.is_none() || opinion.as_deref().unwrap_or("").is_empty(),
        "without body opinion should be empty, got {:?}",
        opinion
    );

    cleanup_approval_fixtures(&pool).await;
}

// ── T1.6: global_overview RLS visible_ids 过滤（P1-4 完成态）─────────────────

#[tokio::test]
async fn t1_6_overview_filters_by_visible_ids() {
    let pool = connect_test_db().await;
    setup_approval_fixtures(&pool).await;

    // 注入 visible_ids = [APPR_OPERATOR_A]（仅该审批可见）
    let req = req_with_user(USER_A);
    let mut ctx = RequestContext::new(USER_A, format!("user{}@test.local", USER_A));
    ctx.set_visible_resource_ids("global".to_string(), vec![APPR_OPERATOR_A]);
    req.extensions_mut().insert(ctx);

    let resp = global_overview::get_global_overview(req, web::Data::new(pool.clone()))
        .await
        .expect("overview handler");
    let body = to_bytes(resp.into_body()).await.expect("body bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("overview json");

    let approvals = json["data"]["approvals"].as_array().expect("approvals");
    let ids: Vec<i64> = approvals
        .iter()
        .filter_map(|a| {
            a["id"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| a["id"].as_i64())
        })
        .collect();
    assert!(
        ids.contains(&APPR_OPERATOR_A),
        "visible_ids 应包含 APPR_OPERATOR_A"
    );
    assert!(
        !ids.contains(&APPR_CREATED_A),
        "visible_ids 之外的审批（APPR_CREATED_A）应被过滤"
    );

    cleanup_approval_fixtures(&pool).await;
}

// ── T1.7: 审批流自检种子幂等（add-approval-flow-self-check）──────────────────

const STATUS_SEED_CODES: [&str; 3] = ["pending", "approved", "rejected"];
const REG_EVENT: i64 = -99021; // 注册审批事件（even-approve，缺实例）
const REG_OPER: i64 = -99022; // 注册审批实例（oper-approve，even 缺失 → oper→even 自愈）
const VERIFY_OPER: i64 = -99024; // user-verify 断链实例（自愈应绑 FLOW-USER-VERIFY 重建）
const INACTIVE_OPER: i64 = -99026; // 主体 is_active=false 的断链实例（不应被自愈重建）

async fn status_seed_count(pool: &PgPool, code: &str) -> i64 {
    sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_stus-approve"
           WHERE code = $1 AND deleted_at IS NULL"#,
    )
    .bind(code)
    .fetch_one(pool)
    .await
    .expect("count status seed")
}

#[allow(dead_code)] // 测试辅助：供后续流程种子计数用例接线（当前分支未调用）
async fn flow_seed_count(pool: &PgPool, code: &str) -> i64 {
    sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_process
           WHERE code = $1 AND deleted_at IS NULL"#,
    )
    .bind(code)
    .fetch_one(pool)
    .await
    .expect("count flow seed")
}

async fn backfill_count(pool: &PgPool, event_id: i64) -> i64 {
    sqlx::query_scalar(
        // fix-fk-approve-residual-consumers：实例↔事件经 rr_event 桥计数
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event rr
             ON rr.ref_left = oa.id AND rr.ref_right = $1 AND rr.deleted_at IS NULL
           WHERE oa.deleted_at IS NULL"#,
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .expect("count backfilled instances")
}

#[tokio::test]
async fn t1_7_self_check_seeds_idempotent() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;

    // 幂等断言：两次运行后各种子行数稳定（缺失即补，已存在不重复）
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;
    let mut first: Vec<i64> = Vec::with_capacity(STATUS_SEED_CODES.len());
    for code in STATUS_SEED_CODES {
        first.push(status_seed_count(&pool, code).await);
    }

    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;
    for (i, code) in STATUS_SEED_CODES.iter().enumerate() {
        let n = status_seed_count(&pool, code).await;
        assert!(n >= 1, "状态种子 {} 应存在，got {}", code, n);
        assert_eq!(n, first[i], "状态种子 {} 二次运行行数应稳定", code);
    }

    // FLOW-USER-REGISTER 已退役（运行时零消费方，register.rs 绑 FLOW-AUTHORIZATION）——
    // 不再种子、不断言行数；存量库惰性遗留行由人工通道清理
}

// ── T1.8: 注册审批实例补链自愈（even-approve 缺 oper-approve → 补建）─────────

#[tokio::test]
async fn t1_8_self_check_backfills_registration_instance() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;

    // 清理残留 fixture（先桥行后实例——桥行引用实例 id；桥行按 ref_right 无条件清，
    // 含 ref_left 实例已删的悬空桥——残留悬空桥会让自检「已有桥」短路，补建断言必假）
    sqlx::query(r#"DELETE FROM isahl.zc_id_operation_rr_event rr WHERE rr.ref_right = $1"#)
        .bind(REG_EVENT)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE code = 'user-register-approval' AND notice LIKE 'T1.8%'"#)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_even-approve" WHERE id = $1"#)
        .bind(REG_EVENT)
        .execute(&pool)
        .await
        .ok();

    // 构造注册审批事件（code=user-register-approval，comments 纯文本——comments-text-semantics
    // 契约：写侧禁止 JSON；申请人归属经 created_by_id → 补链 fk_subject）
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JC", "FTA", "↑_NA"))
            .await
            .expect("resolve even-approve coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_appr-authorization" (id, notice, code, comments, created_by_id, created_at, updated_at,
                                                    dk_scene, dk_factor, dk_function)
           VALUES ($1, 'T1.8注册审批', 'user-register-approval', $2, $3, NOW(), NOW(), $4, $5, $6)"#,
    )
    .bind(REG_EVENT)
    .bind("访问授权审批：申请人 t18")
    .bind(USER_A)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("insert registration event fixture");
    assert_eq!(backfill_count(&pool, REG_EVENT).await, 0);

    // 自检 → 补建实例（dock 可见）
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;
    assert_eq!(
        backfill_count(&pool, REG_EVENT).await,
        1,
        "应补建 1 个 oper-approve 实例"
    );

    // 再次自检 → 不重复补建
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;
    assert_eq!(
        backfill_count(&pool, REG_EVENT).await,
        1,
        "二次自检不得重复补建"
    );

    // 补建实例语义核对：notice/code 同事件，rr_event 桥指向事件
    let (code, fk_subject): (String, Option<i64>) = sqlx::query_as(
        r#"SELECT oa.code, oa.fk_subject FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event rr
             ON rr.ref_left = oa.id AND rr.ref_right = $1 AND rr.deleted_at IS NULL
           WHERE oa.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(REG_EVENT)
    .fetch_one(&pool)
    .await
    .expect("backfilled row");
    assert_eq!(code, "user-register-approval");
    assert_eq!(
        fk_subject,
        Some(USER_A),
        "fk_subject 应为事件 created_by_id（申请人）"
    );

    // 清理（先桥行后实例 + 事件）
    sqlx::query(r#"DELETE FROM isahl.zc_id_operation_rr_event WHERE ref_right = $1"#)
        .bind(REG_EVENT)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE code = 'user-register-approval' AND notice LIKE 'T1.8%'"#)
        .execute(&pool)
        .await
        .ok();

    sqlx::query(r#"DELETE FROM isahl."zc_id_even-approve" WHERE id = $1"#)
        .bind(REG_EVENT)
        .execute(&pool)
        .await
        .ok();
}

// ── T1.9: 驳回用户手动重新申请授权（refine-rejection-not-disabled）─────────

#[tokio::test]
async fn t1_9_rejected_user_reapply_creates_authorization_instance() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;

    // 前置：seed 模板（FLOW-AUTHORIZATION）+ 时长维度（72h）
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    // 清理前次运行残留（username 唯一锚点）
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username = 'reapply-user'")
        .execute(&pool)
        .await;

    // 构造 rejected 用户（username 唯一锚点）
    let uid: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.auth_users
           (id, username, user_type, status, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES (isahl.gen_next_zuid(), 'reapply-user',
                   'standard', 'rejected', TRUE,
                   NOW(), NOW(), 0, '{}'::jsonb)
           RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("insert rejected user");

    // 调用 apply handler（登录上下文 = 该用户）
    let req = req_with_user(uid);
    let resp = approvals::apply_approval(req, web::Data::new(pool.clone())).await;
    let body = to_bytes(resp.into_body()).await.expect("body bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("apply json");
    assert!(
        json["success"].as_bool().unwrap_or(false),
        "apply 应成功: {json}"
    );
    let instance_id: i64 = json["instance_id"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| json["instance_id"].as_i64())
        .expect("instance id");

    // 断言 1：状态回 pending_approval
    let status: String =
        sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(uid)
            .fetch_one(&pool)
            .await
            .expect("user status");
    assert_eq!(status, "pending_approval", "apply 后应回审批中状态");

    // 断言 2：事件落 authorization 叶表 + 绑模板 + SLA。
    // remove-comments-json-embedding 降级：定位键从 comments.applicant_id 改为
    // created_by_id（申请人即事件创建者）；`_t_` 断言保持不变。
    let (code, flow_id, _t): (String, Option<i64>, Option<String>) = sqlx::query_as(
        r#"SELECT a.code,
                  (SELECT rro.ref_left FROM isahl.zc_id_operation_rr_event oe
                   JOIN isahl.zc_id_process_rr_operation rro
                     ON rro.ref_right = oe.ref_left AND rro.deleted_at IS NULL
                   WHERE oe.ref_right = a.id AND oe.deleted_at IS NULL
                   ORDER BY oe.created_at LIMIT 1),
                  a._t_
           FROM isahl."zc_id_appr-authorization" a
           WHERE a.code = 'user-register-approval'
             AND a.created_by_id = $1
           ORDER BY a.id DESC LIMIT 1"#,
    )
    .bind(uid)
    .fetch_one(&pool)
    .await
    .expect("authorization leaf row");
    assert_eq!(code, "user-register-approval");
    assert!(
        flow_id.is_some(),
        "应经桥链绑定 FLOW-AUTHORIZATION 模板（fk_process 已移除）"
    );
    assert_eq!(_t, None, "`_t_` 是自动维度列，业务禁止写入值");

    // 断言 3：oper-approve 实例存在（rr_event 桥 → 叶表事件 id）
    let inst: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve"
           WHERE id = $1 AND code = 'user-register-approval' AND deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_one(&pool)
    .await
    .expect("instance count");
    assert_eq!(inst, 1);

    // 清理
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(instance_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_appr-authorization"
           WHERE created_by_id = $1"#,
    )
    .bind(uid)
    .execute(&pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(uid)
        .execute(&pool)
        .await
        .ok();
}

// ── T1.10: oper→even 自愈（oper-approve 断链，桥缺失 → 重建 even 并回填）───
//
// 与 T1.8 反向：T1.8 测 even 缺 oper（补建实例）；本测试测 oper 缺 even（重建事件并
// 回填 rr_event 桥行）。测试库有 zc_id_appr-authorization 叶表（seed-release-tables），
// 故自愈重建写入叶表。

#[tokio::test]
async fn t1_10_self_check_heals_broken_oper_to_event() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;

    // 前置：seed 模板（FLOW-AUTHORIZATION）+ 72h 时长维度
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    // 清理残留 fixture（桥行一并清——rr_event 唯一键含软删行，
    // 残留桥会导致重复键违例/断链断言失真）
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(REG_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_operation_rr_event WHERE ref_left = $1"#)
        .bind(REG_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_appr-authorization" WHERE notice = 'T1.10访问授权审批'"#,
    )
    .execute(&pool)
    .await
    .ok();

    // 创建真实主体用户（oper→even 自愈要求 fk_subject 存在于 auth_users）
    // 夹具幂等：上次失败中断的残留先行清理（username 唯一键 t110-user）
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username = 't110-user'")
        .execute(&pool)
        .await
        .ok();
    let uid: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.auth_users
           (id, username, user_type, status, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES (isahl.gen_next_zuid(), 't110-user',
                   'standard', 'active', TRUE,
                   NOW(), NOW(), 0, '{}'::jsonb)
           RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("insert subject user");

    // 构造断链 oper 实例（code=user-register-approval，无 rr_event 桥 = 断链）
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JE", "FTA", "↓_EZ"))
            .await
            .expect("resolve oper-approve coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (id, notice, code, fk_subject, created_by_id, created_at, updated_at,
            dk_scene, dk_factor, dk_function)
           VALUES ($1, 'T1.10访问授权审批', 'user-register-approval', $2, $2, NOW(), NOW(), $3, $4, $5)"#,
    )
    .bind(REG_OPER)
    .bind(uid)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("insert broken oper fixture");
    // 确认断链：无指向活跃 even-approve 事件的 rr_event 桥行
    let broken_before: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           WHERE oa.id = $1 AND oa.deleted_at IS NULL
             AND NOT EXISTS (
                 SELECT 1 FROM isahl.zc_id_operation_rr_event rr
                 JOIN isahl."zc_id_even-approve" e ON e.id = rr.ref_right AND e.deleted_at IS NULL
                 WHERE rr.ref_left = oa.id AND rr.deleted_at IS NULL)"#,
    )
    .bind(REG_OPER)
    .fetch_one(&pool)
    .await
    .expect("broken count before");
    assert_eq!(broken_before, 1, "前置：oper 应为断链");

    // 自检 → 重建 even 事件（写叶表）+ 回填 rr_event 桥行
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    // 断言 1：桥行已回填为真实存在的 even 事件
    let ev_id: i64 = sqlx::query_scalar(
        r#"SELECT rr.ref_right FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event rr
             ON rr.ref_left = oa.id AND rr.deleted_at IS NULL
           JOIN isahl."zc_id_even-approve" e ON e.id = rr.ref_right AND e.deleted_at IS NULL
           WHERE oa.id = $1 AND oa.deleted_at IS NULL"#,
    )
    .bind(REG_OPER)
    .fetch_one(&pool)
    .await
    .expect("oper bridge row");

    // 断言 2：事件落 authorization 叶表（测试库有叶表），code/notice 与 oper 一致
    let (ev_code, ev_notice): (String, String) = sqlx::query_as(
        r#"SELECT code, notice FROM isahl."zc_id_appr-authorization"
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(ev_id)
    .fetch_one(&pool)
    .await
    .expect("healed even row in leaf table");
    assert_eq!(ev_code, "user-register-approval");
    assert_eq!(ev_notice, "T1.10访问授权审批");

    // 断言 3（remove-comments-json-embedding 降级）：comments 文本化后申请人语义
    // 由 created_by_id 承载（seed 重建行同样写 created_by_id = 申请人）
    let applicant: i64 = sqlx::query_scalar(
        r#"SELECT created_by_id
           FROM isahl."zc_id_appr-authorization" WHERE id = $1"#,
    )
    .bind(ev_id)
    .fetch_one(&pool)
    .await
    .expect("created_by_id");
    assert_eq!(applicant, uid);

    // 断言 4：二次自检不重复重建（仍只 1 条该 notice 事件）
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;
    let ev_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_appr-authorization"
           WHERE notice = 'T1.10访问授权审批' AND deleted_at IS NULL"#,
    )
    .fetch_one(&pool)
    .await
    .expect("event count after second check");
    assert_eq!(ev_count, 1, "二次自检不得重复重建 even 事件");

    // 清理（含桥行）
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(REG_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_operation_rr_event WHERE ref_left = $1"#)
        .bind(REG_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_appr-authorization" WHERE notice = 'T1.10访问授权审批'"#,
    )
    .execute(&pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(uid)
        .execute(&pool)
        .await
        .ok();
}

// ── T1.11: 注册审批通过/驳回经 fk_subject 激活/禁用用户（fix-register-approval-activation-chain）───
//
// comments 文本化后申请人经 oper-approve.fk_subject 模型列承载；审批 handler
// 内联副作用：user-register-approval 实例通过 → 用户 active，驳回 → disabled。

#[tokio::test]
async fn t1_11_registration_approval_activates_user_via_fk_subject() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;

    // 前置：seed 模板（FLOW-AUTHORIZATION）+ 审批状态字典
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    const REG_USER: i64 = -99311;
    const REG_EVENT_11: i64 = -99321;
    const REG_OPER_11: i64 = -99331;

    // 清理残留（含桥行——rr_event 唯一键含软删行，残留桥导致 23505 重复键违例）
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_oper-approve\" WHERE id = -99331")
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_even-approve\" WHERE id = -99321")
        .execute(&pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM isahl.zc_id_operation_rr_event WHERE ref_left = -99331 OR ref_right = -99321",
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = -99311")
        .execute(&pool)
        .await;

    // 注册用户（pending_approval）
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, username, status, is_active, user_type, created_at, updated_at) \
         VALUES ($1, 't1_11_user', 'pending_approval', true, 'standard', NOW(), NOW())",
    )
    .bind(REG_USER)
    .execute(&pool)
    .await
    .expect("insert pending user");

    // 审批事件 + 实例（code=user-register-approval，fk_subject=REG_USER，comments 纯文本）
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (ev_dk_scene, ev_dk_factor, ev_dk_function) =
        ontology_binding::resolve(&pool, ("JC", "FTA", "↑_NA"))
            .await
            .expect("resolve even-approve coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_appr-authorization" (id, notice, code, comments, created_by_id, created_at, updated_at,
                                                    dk_scene, dk_factor, dk_function)
           VALUES ($1, 'T1.11注册审批', 'user-register-approval', $2, $3, NOW(), NOW(), $4, $5, $6)"#,
    )
    .bind(REG_EVENT_11)
    .bind("访问授权审批：申请人 t1_11_user")
    .bind(REG_USER)
    .bind(ev_dk_scene)
    .bind(ev_dk_factor)
    .bind(ev_dk_function)
    .execute(&pool)
    .await
    .expect("insert registration event");
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (op_dk_scene, op_dk_factor, op_dk_function) =
        ontology_binding::resolve(&pool, ("JE", "FTA", "↓_EZ"))
            .await
            .expect("resolve oper-approve coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_oper-approve" (id, notice, code, fk_subject, fk_operator, created_by_id, created_at, updated_at,
                                                    dk_scene, dk_factor, dk_function)
           VALUES ($1, 'T1.11注册审批', 'user-register-approval', $2, $3, $2, NOW(), NOW(), $4, $5, $6)"#,
    )
    .bind(REG_OPER_11)
    .bind(REG_USER)
    .bind(USER_A)
    .bind(op_dk_scene)
    .bind(op_dk_factor)
    .bind(op_dk_function)
    .execute(&pool)
    .await
    .expect("insert registration instance");
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_uid(267), $1, $2, 1)"#,
    )
    .bind(REG_OPER_11)
    .bind(REG_EVENT_11)
    .execute(&pool)
    .await
    .expect("insert instance-event bridge");

    // approve → 用户激活
    let req = req_with_user(USER_A);
    let path = actix_web::web::Path::from(REG_OPER_11);
    let resp = approvals::approve_approval(
        req,
        web::Data::new(pool.clone()),
        web::Data::new(std::sync::Arc::new(
            alioth_gateway::notification::db_messaging::DbMessagingService::new(pool.clone()),
        )
            as std::sync::Arc<dyn common::messaging::MessagingService>),
        web::Data::new(
            std::sync::Arc::new(common::event_bus::InMemoryEventBus::new())
                as std::sync::Arc<dyn common::event_bus::DomainEventBus>,
        ),
        path,
        None,
    )
    .await;
    let resp_body = to_bytes(resp.into_body()).await.expect("body bytes");
    let json: serde_json::Value = serde_json::from_slice(&resp_body).expect("approve json");
    eprintln!("T1.11 approve resp: {json:?}");
    assert!(
        json["success"].as_bool().unwrap_or(false),
        "approve should succeed"
    );
    let status: String =
        sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(REG_USER)
            .fetch_one(&pool)
            .await
            .expect("user status");
    assert_eq!(status, "active", "注册审批通过后用户 MUST 激活");

    // 幂等守卫：再次 approve（实例已 approved，handler 拒绝——但即使成功 status 已 active 不再变）
    // 实际验证：直接调用激活函数对 active 用户不降级
    let _ = approvals::apply_registration_activation(&pool, REG_OPER_11, "disabled", None).await;
    let status2: String =
        sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(REG_USER)
            .fetch_one(&pool)
            .await
            .expect("user status2");
    assert_eq!(
        status2, "active",
        "active 用户 MUST NOT 被 fk_subject 副作用降级（pending 守卫）"
    );

    // 清理（含桥行）
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_oper-approve\" WHERE id = -99331")
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_even-approve\" WHERE id = -99321")
        .execute(&pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM isahl.zc_id_operation_rr_event WHERE ref_left = -99331 OR ref_right = -99321",
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = -99311")
        .execute(&pool)
        .await;
}

// ── T1.12: 主体已删除的断链 oper 不重建（保持告警）────────────────────────
//
// oper→even 自愈仅对 fk_subject 仍存在于 auth_users 的 oper 生效；主体已删除
// （如 WZ 测试用户 regblock/regrej）时无法确定 applicant，重建会污染事件且不可
// 追溯——应跳过自愈、保持 broken_after 告警（fix-approval-event-adaptive-write）。

#[tokio::test]
async fn t1_12_broken_oper_with_missing_subject_not_reconstructed() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;

    // 前置：seed 模板
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    // 清理残留 fixture（含桥行——t1_10 自愈回填的残留桥会污染断链断言）
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(REG_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_operation_rr_event WHERE ref_left = $1"#)
        .bind(REG_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_appr-authorization" WHERE notice = 'T1.12访问授权审批'"#,
    )
    .execute(&pool)
    .await
    .ok();

    // 构造断链 oper：fk_subject 指向不存在的用户（-99099，auth_users 无此 id），无桥行
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JE", "FTA", "↓_EZ"))
            .await
            .expect("resolve oper-approve coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (id, notice, code, fk_subject, created_by_id, created_at, updated_at,
            dk_scene, dk_factor, dk_function)
           VALUES ($1, 'T1.12访问授权审批', 'user-register-approval', $2, $2, NOW(), NOW(), $3, $4, $5)"#,
    )
    .bind(REG_OPER)
    .bind(-99099) // 不存在的用户 id
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("insert broken oper with missing subject");

    // 自检
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    // 断言 1：桥行未被回填（实例仍无 rr_event 桥 = 主体缺失不自愈）
    let bridges: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_operation_rr_event rr
           WHERE rr.ref_left = $1 AND rr.deleted_at IS NULL"#,
    )
    .bind(REG_OPER)
    .fetch_one(&pool)
    .await
    .expect("oper bridge count");
    assert_eq!(bridges, 0, "主体缺失的断链 oper 不应被重建回填桥行");

    // 断言 2：未生成 even 事件（notice 匹配的叶表行不应存在）
    let ev_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_appr-authorization"
           WHERE notice = 'T1.12访问授权审批' AND deleted_at IS NULL"#,
    )
    .fetch_one(&pool)
    .await
    .expect("even count");
    assert_eq!(ev_count, 0, "主体缺失的断链 oper 不应生成 even 事件");

    // 清理（含桥行）
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(REG_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_operation_rr_event WHERE ref_left = $1"#)
        .bind(REG_OPER)
        .execute(&pool)
        .await
        .ok();
}
// ── T1.14: user-verify 断链自愈（绑 FLOW-USER-VERIFY，fix-user-verify-self-heal）──
//
// oper→even 自愈按事件 code 分派绑定：user-verify → FLOW-USER-VERIFY +
// zc_id_appr-user_verify 叶表（identity.rs 写入契约）；事件↔流程归属经叶表
// fk_process，tpl_id 绑该流程 approve 节点操作行（MUST NOT 指向 AUTHORIZATION）。

#[tokio::test]
async fn t1_14_user_verify_broken_heals_with_verify_flow_binding() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;

    // 前置：模型级种子重放（FLOW-USER-VERIFY 模板 + 72h 时长——组件零模板）
    alioth_gateway::seed::ensure_startup_seed_self_check(&pool).await;
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    // 清理残留 fixture（含桥行——rr_event 唯一键含软删行，残留桥致重复键违例/断链失真）
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(VERIFY_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_operation_rr_event WHERE ref_left = $1"#)
        .bind(VERIFY_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_appr-user_verify" WHERE notice = 'T1.14实名审核'"#)
        .execute(&pool)
        .await
        .ok();

    // 创建真实主体用户（自愈要求 fk_subject 存在于 auth_users 且活跃）
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username = 't114-user'")
        .execute(&pool)
        .await
        .ok();
    let uid: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.auth_users
           (id, username, user_type, status, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES (isahl.gen_next_zuid(), 't114-user',
                   'standard', 'active', TRUE,
                   NOW(), NOW(), 0, '{}'::jsonb)
           RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("insert subject user");

    // 构造 user-verify 断链 oper 实例（无 rr_event 桥 = 断链）
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JE", "FTA", "↓_EZ"))
            .await
            .expect("resolve oper-approve coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (id, notice, code, fk_subject, created_by_id, created_at, updated_at,
            dk_scene, dk_factor, dk_function)
           VALUES ($1, 'T1.14实名审核', 'user-verify', $2, $2, NOW(), NOW(), $3, $4, $5)"#,
    )
    .bind(VERIFY_OPER)
    .bind(uid)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("insert user-verify broken oper fixture");

    // 前置：确认断链（无指向活跃 even-approve 事件的 rr_event 桥行）
    let broken_before: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           WHERE oa.id = $1 AND oa.deleted_at IS NULL
             AND NOT EXISTS (
                 SELECT 1 FROM isahl.zc_id_operation_rr_event rr
                 JOIN isahl."zc_id_even-approve" e ON e.id = rr.ref_right AND e.deleted_at IS NULL
                 WHERE rr.ref_left = oa.id AND rr.deleted_at IS NULL)"#,
    )
    .bind(VERIFY_OPER)
    .fetch_one(&pool)
    .await
    .expect("broken count before");
    assert_eq!(broken_before, 1, "前置：user-verify oper 应为断链");

    // 自检 → 重建事件（落 user_verify 叶表）+ 回填 rr_event 桥行（绑 FLOW-USER-VERIFY）
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    // 断言 1：桥行已回填为真实存在的 even 事件
    let ev_id: i64 = sqlx::query_scalar(
        r#"SELECT rr.ref_right FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event rr
             ON rr.ref_left = oa.id AND rr.deleted_at IS NULL
           JOIN isahl."zc_id_even-approve" e ON e.id = rr.ref_right AND e.deleted_at IS NULL
           WHERE oa.id = $1 AND oa.deleted_at IS NULL"#,
    )
    .bind(VERIFY_OPER)
    .fetch_one(&pool)
    .await
    .expect("oper bridge row");

    // 断言 2：事件落点自适应（叶表存在 → user_verify 叶表；否则 even-approve 主表），code 保持 user-verify
    let leaf_exists: bool =
        sqlx::query_scalar(r#"SELECT to_regclass('isahl."zc_id_appr-user_verify"') IS NOT NULL"#)
            .fetch_one(&pool)
            .await
            .expect("leaf probe");
    let ev_code: String = if leaf_exists {
        sqlx::query_scalar::<_, String>(
            r#"SELECT code FROM isahl."zc_id_appr-user_verify" WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(ev_id)
        .fetch_one(&pool)
        .await
        .expect("healed event in user_verify leaf")
    } else {
        sqlx::query_scalar::<_, String>(
            r#"SELECT code FROM isahl."zc_id_even-approve" WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(ev_id)
        .fetch_one(&pool)
        .await
        .expect("healed event in even-approve main table")
    };
    assert_eq!(ev_code, "user-verify");

    // 断言 3：绑定 FLOW-USER-VERIFY（而非 FLOW-AUTHORIZATION）
    let verify_flow_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl.zc_id_process WHERE code = 'FLOW-USER-VERIFY' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .expect("verify flow query")
    .expect("FLOW-USER-VERIFY 模板应由模型级种子供给");
    // fk_process（叶表契约列）：identity.rs 同源写值 = 流程行 id
    let fk_process: Option<i64> = sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT fk_process FROM isahl."zc_id_appr-user_verify" WHERE id = $1"#,
    )
    .bind(ev_id)
    .fetch_optional(&pool)
    .await
    .expect("fk_process query")
    .flatten();
    assert_eq!(
        fk_process,
        Some(verify_flow_id),
        "事件 fk_process 应为 FLOW-USER-VERIFY 行 id"
    );
    // tpl_id：该流程 approve 节点操作行（process_rr_operation 桥反查）
    let tpl_id: Option<i64> =
        sqlx::query_scalar(r#"SELECT tpl_id FROM isahl."zc_id_even-approve" WHERE id = $1"#)
            .bind(ev_id)
            .fetch_one(&pool)
            .await
            .expect("tpl query");
    let tpl_in_verify: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (SELECT 1 FROM isahl.zc_id_process_rr_operation rro
                          WHERE rro.ref_left = $1 AND rro.ref_right = $2 AND rro.deleted_at IS NULL)"#,
    )
    .bind(verify_flow_id)
    .bind(tpl_id)
    .fetch_one(&pool)
    .await
    .expect("tpl binding query");
    assert!(
        tpl_in_verify,
        "事件 tpl_id 应绑 FLOW-USER-VERIFY 节点（而非 AUTHORIZATION），tpl={tpl_id:?}"
    );

    // 断言 4：二次自检不重复重建（仍只 1 条该 notice 事件）
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;
    let ev_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_appr-user_verify"
           WHERE notice = 'T1.14实名审核' AND deleted_at IS NULL"#,
    )
    .fetch_one(&pool)
    .await
    .expect("event count after second check");
    assert_eq!(ev_count, 1, "二次自检不得重复重建 even 事件");

    // 清理（含桥行）
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(VERIFY_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_operation_rr_event WHERE ref_left = $1"#)
        .bind(VERIFY_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_appr-user_verify" WHERE notice = 'T1.14实名审核'"#)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(uid)
        .execute(&pool)
        .await
        .ok();
}

// ── T1.13: 主体 is_active=false（封禁/停用）的断链 oper 不重建 ──────────────
//
// oper→even 自愈仅对主体存在且 is_active=TRUE 的 oper 生效；封禁/停用用户授权链路
// 已终止，重建审批事件无意义且不可追溯——应跳过自愈、保持 broken_after 告警
// （fix-approval-event-adaptive-write + advisory 复核四）。
// 使用独立 INACTIVE_OPER ID，避免与 t1_10/t1_12 并行互删。

#[tokio::test]
async fn t1_13_inactive_subject_not_reconstructed() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;

    // 前置：seed 模板
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    // 清理残留 fixture（独立 ID + 用户名，避免与并行测试互删；含桥行）
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(INACTIVE_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_operation_rr_event WHERE ref_left = $1"#)
        .bind(INACTIVE_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_appr-authorization" WHERE notice = 'T1.13访问授权审批'"#,
    )
    .execute(&pool)
    .await
    .ok();
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username = 't113-user'")
        .execute(&pool)
        .await;

    // 构造 is_active=false 的主体用户
    let uid: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.auth_users
           (id, username, user_type, status, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES (isahl.gen_next_zuid(), 't113-user',
                   'standard', 'pending_approval', FALSE,
                   NOW(), NOW(), 0, '{}'::jsonb)
           RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("insert inactive subject user");

    // 构造断链 oper：fk_subject = 该停用用户，无 rr_event 桥行
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JE", "FTA", "↓_EZ"))
            .await
            .expect("resolve oper-approve coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (id, notice, code, fk_subject, created_by_id, created_at, updated_at,
            dk_scene, dk_factor, dk_function)
           VALUES ($1, 'T1.13访问授权审批', 'user-register-approval', $2, $2, NOW(), NOW(), $3, $4, $5)"#,
    )
    .bind(INACTIVE_OPER)
    .bind(uid)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("insert broken oper with inactive subject");

    // 自检
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    // 断言 1：桥行未被回填（is_active=false 主体不自愈）
    let bridges: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_operation_rr_event rr
           WHERE rr.ref_left = $1 AND rr.deleted_at IS NULL"#,
    )
    .bind(INACTIVE_OPER)
    .fetch_one(&pool)
    .await
    .expect("oper bridge count");
    assert_eq!(
        bridges, 0,
        "is_active=false 主体的断链 oper 不应被重建回填桥行"
    );

    // 断言 2：未生成 even 事件
    let ev_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_appr-authorization"
           WHERE notice = 'T1.13访问授权审批' AND deleted_at IS NULL"#,
    )
    .fetch_one(&pool)
    .await
    .expect("even count");
    assert_eq!(
        ev_count, 0,
        "is_active=false 主体的断链 oper 不应生成 even 事件"
    );

    // 清理
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(INACTIVE_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(uid)
        .execute(&pool)
        .await
        .ok();
}

// ── 外部主体注册审批激活（add-dual-register-channels）────────────────────────

/// 外部入驻审批实例（code=external-subject-register-approval）审批通过/驳回
/// 应与内部注册审批同语义：经 fk_subject 激活/禁用申请人。
#[tokio::test]
async fn t_ext_subject_register_activation() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;

    // 清理前次运行残留
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username = 'ext-subj-act-user'")
        .execute(&pool)
        .await;
    let _ = sqlx::query(
        r#"DELETE FROM isahl."zc_id_oper-approve" WHERE notice = 'T-EXT 外部主体入驻审批'"#,
    )
    .execute(&pool)
    .await
    .ok();

    // 构造 pending_approval 外部用户
    let uid: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.auth_users
           (id, username, user_type, status, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES (isahl.gen_next_zuid(), 'ext-subj-act-user',
                   'external', 'pending_approval', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("insert external user");

    // 构造外部入驻审批实例（外部 code——激活链扩展匹配面）
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JE", "FTA", "↓_EZ"))
            .await
            .expect("resolve oper-approve coords");
    let instance_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (notice, code, fk_subject, created_by_id, created_at, updated_at,
            dk_scene, dk_factor, dk_function)
           VALUES ('T-EXT 外部主体入驻审批', 'external-subject-register-approval', $1, $1, NOW(), NOW(), $2, $3, $4)
           RETURNING id"#,
    )
    .bind(uid)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(&pool)
    .await
    .expect("insert external instance");

    // 通过 → 激活
    approvals::apply_registration_activation(&pool, instance_id, "active", None)
        .await
        .expect("activation");
    let status: String =
        sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(uid)
            .fetch_one(&pool)
            .await
            .expect("user status");
    assert_eq!(status, "active", "外部入驻审批通过应激活账号");

    // 驳回 → 禁用（先回 pending 态再验）
    sqlx::query("UPDATE isahl_auth.auth_users SET status = 'pending_approval' WHERE id = $1")
        .bind(uid)
        .execute(&pool)
        .await
        .ok();
    approvals::apply_registration_activation(&pool, instance_id, "disabled", None)
        .await
        .expect("disable");
    let status: String =
        sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(uid)
            .fetch_one(&pool)
            .await
            .expect("user status");
    assert_eq!(status, "disabled", "外部入驻审批驳回应禁用账号");

    // 清理
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(instance_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(uid)
        .execute(&pool)
        .await
        .ok();
}

// ── 队列批量审批与转办候选（change consolidate-admin-approval-queue D3）────────
//
// 语义：批量 = 逐实例走**单实例链路**（注册激活/禁用 + 意见 + 终态事件 + 鉴权），
// 逐实例独立提交，单实例失败不阻塞其余。**MUST NOT** 走 framework
// `/approval-instances/batch/*`——该链路不含 `isahl_auth.auth_users.status` 变更，注册实例
// 走它会退回「用户永卡 pending_approval」（框架写径的同族副作用由平台档位
// ApprovalCompleted 兜底订阅者承担）。

const QA_ACTOR: i64 = -99501; // 队列操作人（fk_operator）
const QA_OTHER: i64 = -99502; // 他人 operator（单实例鉴权失败夹具）
const QA_USER_A: i64 = -99511; // 申请人 A
const QA_USER_B: i64 = -99512; // 申请人 B

/// 清理本组 fixture（幂等，测试前后各一次）
async fn qa_cleanup(pool: &PgPool) {
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE notice LIKE 'QA-%'"#)
        .execute(pool)
        .await
        .ok();
    for uid in [QA_USER_A, QA_USER_B, QA_ACTOR, QA_OTHER] {
        sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
            .bind(uid)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
            .bind(uid)
            .execute(pool)
            .await
            .ok();
    }
}

/// 申请人账号（注册态 pending_approval）
async fn qa_insert_applicant(pool: &PgPool, uid: i64, name: &str) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, username, user_type, is_active, status, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, $2, 'standard', TRUE, 'pending_approval', NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO UPDATE SET status = 'pending_approval'"#,
    )
    .bind(uid)
    .bind(name)
    .execute(pool)
    .await
    .expect("insert applicant");
}

/// 注册审批实例（fk_subject = 申请人 id；fk_operator = 指定处理人）
async fn qa_insert_instance(
    pool: &PgPool,
    notice: &str,
    code: &str,
    applicant: i64,
    operator: i64,
) -> i64 {
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("JE", "FTA", "↓_EZ"))
        .await
        .expect("resolve oper-approve coords");
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (notice, code, fk_subject, fk_operator, created_by_id, created_at, updated_at,
            dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $3, NOW(), NOW(), $5, $6, $7)
           RETURNING id"#,
    )
    .bind(notice)
    .bind(code)
    .bind(applicant)
    .bind(operator)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("insert registration instance")
}

/// 账号 + admin UA 绑定（`is_admin_user` / 转办候选判据需要）
async fn qa_ensure_admin(pool: &PgPool, uid: i64) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, username, user_type, is_active, status, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, $2, 'standard', TRUE, 'active', NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(uid)
    .bind(format!("qa-admin-{uid}"))
    .execute(pool)
    .await
    .expect("insert admin user");

    let default_class: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.ngac_policy_class (o_name, description)
           VALUES ('default', 'QA 默认策略类') ON CONFLICT DO NOTHING RETURNING id"#,
    )
    .fetch_optional(pool)
    .await
    .expect("ensure default policy class")
    .unwrap_or(0);
    let class_id = if default_class > 0 {
        default_class
    } else {
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default'")
            .fetch_one(pool)
            .await
            .expect("default policy class")
    };
    let admin_ua: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class)
           VALUES ('admin', $1) ON CONFLICT DO NOTHING RETURNING id"#,
    )
    .bind(class_id)
    .fetch_optional(pool)
    .await
    .expect("ensure admin UA")
    .unwrap_or(0);
    let admin_ua = if admin_ua > 0 {
        admin_ua
    } else {
        sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' AND deleted_at IS NULL LIMIT 1",
        )
        .fetch_one(pool)
        .await
        .expect("admin UA")
    };
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute
           (id, o_name, fk_user, fk_user_attribute, assigned_at, created_at)
           SELECT isahl.gen_next_zuid(), $1, $2, $3, NOW(), NOW()
           WHERE NOT EXISTS (
               SELECT 1 FROM isahl_auth.ngac_user_rr_attribute
               WHERE fk_user = $2 AND fk_user_attribute = $3 AND deleted_at IS NULL
           ) ON CONFLICT DO NOTHING"#,
    )
    .bind(format!("qa-admin-bind-{uid}"))
    .bind(uid)
    .bind(admin_ua)
    .execute(pool)
    .await
    .expect("bind admin UA");
}

fn qa_bus() -> web::Data<std::sync::Arc<dyn common::event_bus::DomainEventBus>> {
    web::Data::new(
        std::sync::Arc::new(common::event_bus::InMemoryEventBus::new())
            as std::sync::Arc<dyn common::event_bus::DomainEventBus>,
    )
}

/// 通知捕获器（结果通知断言：次数 / 收件人 / 标题），不落库、不动真实消息面。
///
/// 用 mpsc 通道而非 `Mutex`：发送侧 async、接收侧由测试 drain——无线程锁、无 poisoning 路径
/// （仓库未引入 `parking_lot`，不为测试新增依赖）。
struct QaMessaging {
    tx: tokio::sync::mpsc::UnboundedSender<(u64, String, String)>,
}

impl QaMessaging {
    fn new() -> (
        std::sync::Arc<Self>,
        tokio::sync::mpsc::UnboundedReceiver<(u64, String, String)>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (std::sync::Arc::new(Self { tx }), rx)
    }
}

#[async_trait::async_trait]
impl common::messaging::MessagingService for QaMessaging {
    async fn send_direct(
        &self,
        _from: u64,
        _to: u64,
        _content: &str,
    ) -> Result<(), common::AliothError> {
        Ok(())
    }
    async fn send_group(
        &self,
        _from: u64,
        _conversation_id: u64,
        _content: &str,
    ) -> Result<(), common::AliothError> {
        Ok(())
    }
    async fn broadcast(&self, _from: u64, _content: &str) -> Result<(), common::AliothError> {
        Ok(())
    }
    async fn send_system_notification(
        &self,
        to: u64,
        title: &str,
        content: &str,
    ) -> Result<(), common::AliothError> {
        let _ = self.tx.send((to, title.to_string(), content.to_string()));
        Ok(())
    }
    async fn send_alert(
        &self,
        _level: common::messaging::AlertLevel,
        _title: &str,
        _content: &str,
    ) -> Result<(), common::AliothError> {
        Ok(())
    }
    async fn send_device_command(
        &self,
        _device_id: &str,
        _command: common::messaging::DeviceCommand,
    ) -> Result<(), common::AliothError> {
        Ok(())
    }
    async fn broadcast_device_command(
        &self,
        _command: common::messaging::DeviceCommand,
    ) -> Result<(), common::AliothError> {
        Ok(())
    }
    async fn send_raw(
        &self,
        _topic: &str,
        _payload: Vec<u8>,
        _qos: u8,
    ) -> Result<common::messaging::MessageDeliveryInfo, common::AliothError> {
        Ok(common::messaging::MessageDeliveryInfo {
            delivered_count: 0,
            offline_queued_count: 0,
        })
    }
}

fn qa_messaging() -> (
    std::sync::Arc<QaMessaging>,
    tokio::sync::mpsc::UnboundedReceiver<(u64, String, String)>,
) {
    QaMessaging::new()
}

fn qa_messaging_data(
    m: &std::sync::Arc<QaMessaging>,
) -> web::Data<std::sync::Arc<dyn common::messaging::MessagingService>> {
    web::Data::new(m.clone() as std::sync::Arc<dyn common::messaging::MessagingService>)
}

/// 只取 Data 形态（不关心通知断言时用；接收端随函数返回丢弃）
fn qa_messaging_sink() -> web::Data<std::sync::Arc<dyn common::messaging::MessagingService>> {
    let (m, _rx) = qa_messaging();
    qa_messaging_data(&m)
}

/// drain 捕获到的通知（调用方断言条数/收件人/标题）
fn qa_collect(
    inbox: &mut tokio::sync::mpsc::UnboundedReceiver<(u64, String, String)>,
) -> Vec<(u64, String, String)> {
    let mut out = Vec::new();
    while let Ok(msg) = inbox.try_recv() {
        out.push(msg);
    }
    out
}

/// 等后台订阅者副作用落库（上限 3s）；返回最终状态（调用方断言）。
async fn qa_wait_status(pool: &PgPool, uid: i64, want: &str) -> String {
    let mut status = String::new();
    for _ in 0..60 {
        status = sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(uid)
            .fetch_one(pool)
            .await
            .expect("user status");
        if status == want {
            return status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    status
}

/// 构造 `ApprovalCompleted` 域事件（发布端语义：`entity_id` = 审批实例 id）
fn qa_completed_event(instance_id: i64, result: &str) -> common::event_bus::DomainEvent {
    common::event_bus::DomainEvent::new(
        "ApprovalCompleted",
        "commitment",
        instance_id,
        serde_json::json!({
            "entity_type": "approval-instance",
            "entity_id": instance_id,
            "result": result,
            "comment": serde_json::Value::Null,
        }),
    )
    .expect("build ApprovalCompleted event")
}

/// 批量通过：两个注册审批实例同链审批 ⇒ 两个申请人均激活（激活语义不因批量丢失）
#[tokio::test]
async fn t_batch_approve_keeps_registration_activation() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;
    qa_cleanup(&pool).await;
    qa_insert_applicant(&pool, QA_USER_A, "qa-batch-a").await;
    qa_insert_applicant(&pool, QA_USER_B, "qa-batch-b").await;
    let inst_a = qa_insert_instance(
        &pool,
        "QA-批量A",
        "user-register-approval",
        QA_USER_A,
        QA_ACTOR,
    )
    .await;
    let inst_b = qa_insert_instance(
        &pool,
        "QA-批量B",
        "external-subject-register-approval",
        QA_USER_B,
        QA_ACTOR,
    )
    .await;

    let resp = approvals::batch_approve(
        req_with_user(QA_ACTOR),
        web::Data::new(pool.clone()),
        qa_bus(),
        qa_messaging_sink(),
        Some(web::Json(approvals::BatchBody {
            ids: vec![inst_a, inst_b],
            opinion: Some("批量同意".to_string()),
        })),
    )
    .await;
    let body = to_bytes(resp.into_body()).await.expect("body bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("batch json");
    assert_eq!(json["success"].as_bool(), Some(true), "批量端点应 200");
    assert_eq!(json["total"].as_u64(), Some(2));
    assert_eq!(json["succeeded"].as_u64(), Some(2));
    let results = json["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2);
    for r in results {
        assert_eq!(r["success"].as_bool(), Some(true));
        // id 语义字段 MUST 字符串化（ID_JSON_PRECISION）
        assert!(r["id"].is_string(), "逐实例结果 id 应为字符串: {r:?}");
    }

    for uid in [QA_USER_A, QA_USER_B] {
        let status: String =
            sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
                .bind(uid)
                .fetch_one(&pool)
                .await
                .expect("user status");
        assert_eq!(status, "active", "批量通过后用户 {uid} MUST 激活");
    }
    qa_cleanup(&pool).await;
}

/// 批量部分失败：单实例鉴权失败（APPROVAL_NOT_OPERATOR）不阻塞其余实例，
/// 且失败实例不产生激活副作用。
#[tokio::test]
async fn t_batch_approve_partial_failure_does_not_block_others() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;
    qa_cleanup(&pool).await;
    qa_insert_applicant(&pool, QA_USER_A, "qa-partial-a").await;
    qa_insert_applicant(&pool, QA_USER_B, "qa-partial-b").await;
    let inst_ok = qa_insert_instance(
        &pool,
        "QA-部分成功",
        "user-register-approval",
        QA_USER_A,
        QA_ACTOR,
    )
    .await;
    // fk_operator = 他人 ⇒ ApprovalService 鉴权失败（单实例失败夹具）
    let inst_denied = qa_insert_instance(
        &pool,
        "QA-部分失败",
        "user-register-approval",
        QA_USER_B,
        QA_OTHER,
    )
    .await;

    let resp = approvals::batch_approve(
        req_with_user(QA_ACTOR),
        web::Data::new(pool.clone()),
        qa_bus(),
        qa_messaging_sink(),
        Some(web::Json(approvals::BatchBody {
            ids: vec![inst_ok, inst_denied],
            opinion: None,
        })),
    )
    .await;
    let body = to_bytes(resp.into_body()).await.expect("body bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("batch json");
    assert_eq!(json["success"].as_bool(), Some(true), "整体请求仍应 200");
    assert_eq!(json["total"].as_u64(), Some(2));
    assert_eq!(json["succeeded"].as_u64(), Some(1));
    let results = json["results"].as_array().expect("results array");
    let denied = results
        .iter()
        .find(|r| r["id"].as_str() == Some(&inst_denied.to_string()))
        .expect("denied instance in results");
    assert_eq!(denied["success"].as_bool(), Some(false));
    assert!(
        denied["message"]
            .as_str()
            .is_some_and(|m| m.contains("APPROVAL_NOT_OPERATOR")),
        "失败项应带单实例错误信息: {denied:?}"
    );

    let status_a: String =
        sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(QA_USER_A)
            .fetch_one(&pool)
            .await
            .expect("status a");
    assert_eq!(status_a, "active", "成功项申请人应激活");
    let status_b: String =
        sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(QA_USER_B)
            .fetch_one(&pool)
            .await
            .expect("status b");
    assert_eq!(
        status_b, "pending_approval",
        "失败项申请人 MUST NOT 被激活（无副作用）"
    );
    qa_cleanup(&pool).await;
}

/// 转办候选：仅 admin 可读（非 admin 403）；候选 = 持 admin UA 的账号
#[tokio::test]
async fn t_approver_candidates_admin_only() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;
    qa_cleanup(&pool).await;
    qa_insert_applicant(&pool, QA_USER_A, "qa-cand-plain").await; // 非 admin
    qa_ensure_admin(&pool, QA_ACTOR).await;

    // 非 admin → 403
    let resp = approvals::approver_candidates(
        req_with_user(QA_USER_A),
        web::Data::new(pool.clone()),
        web::Query(approvals::ApproverCandidatesQuery::default()),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 403, "非 admin 不得读取管理面候选");

    // admin → 200 且含自身
    let resp = approvals::approver_candidates(
        req_with_user(QA_ACTOR),
        web::Data::new(pool.clone()),
        web::Query(approvals::ApproverCandidatesQuery::default()),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
    let body = to_bytes(resp.into_body()).await.expect("body bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("candidates json");
    let ids: Vec<String> = json["data"]
        .as_array()
        .expect("data array")
        .iter()
        .filter_map(|c| c["id"].as_str().map(str::to_string))
        .collect();
    assert!(
        ids.contains(&QA_ACTOR.to_string()),
        "候选应含 admin 账号: {ids:?}"
    );
    assert!(
        !ids.contains(&QA_USER_A.to_string()),
        "非 admin 账号不得出现在候选: {ids:?}"
    );
    qa_cleanup(&pool).await;
}

/// 转办 body 的 id 双形态契约：前端传字符串（zuid 可超 2^53）MUST 可反序列化，
/// 数字形态兼容（旧客户端）。
#[tokio::test]
async fn t_transfer_body_accepts_string_target_id() {
    let parsed: approvals::TransferBody = serde_json::from_value(serde_json::json!({
        "target_id": QA_ACTOR.to_string(),
        "opinion": "转交",
    }))
    .expect("字符串 target_id MUST 可反序列化（ID_JSON_PRECISION）");
    assert_eq!(parsed.target_id, QA_ACTOR);
    assert_eq!(parsed.opinion.as_deref(), Some("转交"));

    let legacy: approvals::TransferBody =
        serde_json::from_value(serde_json::json!({ "target_id": QA_OTHER })).expect("数字形态兼容");
    assert_eq!(legacy.target_id, QA_OTHER);
}

// ── 认证域终态副作用兜底订阅（change fix-registration-activation-on-framework-write-path）──
//
// 语义：framework 写径（ns 模块「我的审批」单条/批量）与 SLA 定时路径不经 Gateway 内联链路，
// 由平台侧 `ApprovalCompleted` 订阅者兜底产生 auth_users 副作用 + 申请人结果通知；
// 与内联路径共用 apply_registration_activation ⇒ 幂等、通知恰好一次。

#[tokio::test]
async fn t_registration_fallback_subscriber_activates_on_event() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;
    qa_cleanup(&pool).await;
    qa_insert_applicant(&pool, QA_USER_A, "qa-sub-active").await;
    let inst = qa_insert_instance(
        &pool,
        "QA-兜底通过",
        "user-register-approval",
        QA_USER_A,
        QA_ACTOR,
    )
    .await;

    let (messaging, mut inbox) = qa_messaging();
    // 直接驱动消费者（生产装配走 host；装配路径由下方 actix 运行时用例覆盖）
    approvals::handle_registration_activation_event(
        &pool,
        &(messaging.clone() as std::sync::Arc<dyn common::messaging::MessagingService>),
        &qa_completed_event(inst, "approved"),
    )
    .await;

    let status: String =
        sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(QA_USER_A)
            .fetch_one(&pool)
            .await
            .expect("user status");
    assert_eq!(
        status, "active",
        "framework 写径（事件驱动）通过后用户 MUST 激活（兜底订阅者）"
    );
    let notes = qa_collect(&mut inbox);
    assert_eq!(
        notes
            .iter()
            .filter(|(to, t, _)| *to == QA_USER_A as u64 && t == "访问授权审批已通过")
            .count(),
        1,
        "结果通知应恰好一次: {notes:?}"
    );
    qa_cleanup(&pool).await;
}

#[tokio::test]
async fn t_registration_fallback_subscriber_disables_on_reject_event() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;
    qa_cleanup(&pool).await;
    qa_insert_applicant(&pool, QA_USER_B, "qa-sub-disabled").await;
    let inst = qa_insert_instance(
        &pool,
        "QA-兜底驳回",
        "external-subject-register-approval",
        QA_USER_B,
        QA_ACTOR,
    )
    .await;

    let (messaging, mut inbox) = qa_messaging();
    approvals::handle_registration_activation_event(
        &pool,
        &(messaging.clone() as std::sync::Arc<dyn common::messaging::MessagingService>),
        &qa_completed_event(inst, "rejected"),
    )
    .await;

    let status: String =
        sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(QA_USER_B)
            .fetch_one(&pool)
            .await
            .expect("user status");
    assert_eq!(
        status, "disabled",
        "framework 写径（事件驱动）驳回后用户 MUST 禁用（兜底订阅者）"
    );
    let notes = qa_collect(&mut inbox);
    assert_eq!(
        notes
            .iter()
            .filter(|(to, t, _)| *to == QA_USER_B as u64 && t == "访问授权审批已驳回")
            .count(),
        1,
        "驳回结果通知应恰好一次: {notes:?}"
    );
    qa_cleanup(&pool).await;
}

/// 幂等：同一事件重复投递 ⇒ 状态不抖动、通知不重复。
#[tokio::test]
async fn t_registration_fallback_subscriber_is_idempotent() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;
    qa_cleanup(&pool).await;
    qa_insert_applicant(&pool, QA_USER_A, "qa-sub-idem").await;
    let inst = qa_insert_instance(
        &pool,
        "QA-兜底幂等",
        "user-register-approval",
        QA_USER_A,
        QA_ACTOR,
    )
    .await;

    let (messaging, mut inbox) = qa_messaging();
    let ms: std::sync::Arc<dyn common::messaging::MessagingService> = messaging.clone();
    let event = qa_completed_event(inst, "approved");
    for _ in 0..2 {
        approvals::handle_registration_activation_event(&pool, &ms, &event).await;
    }

    let status: String =
        sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(QA_USER_A)
            .fetch_one(&pool)
            .await
            .expect("user status");
    assert_eq!(status, "active");
    let notes = qa_collect(&mut inbox);
    assert_eq!(
        notes
            .iter()
            .filter(|(to, t, _)| *to == QA_USER_A as u64 && t == "访问授权审批已通过")
            .count(),
        1,
        "重复事件 MUST NOT 重复投递结果通知（迁移门控）: {notes:?}"
    );
    qa_cleanup(&pool).await;
}

/// 非认证域实例：事件不产生任何 auth_users 副作用与通知（code 收口）。
#[tokio::test]
async fn t_registration_fallback_subscriber_ignores_non_registration_instance() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;
    qa_cleanup(&pool).await;
    qa_insert_applicant(&pool, QA_USER_A, "qa-sub-nonreg").await;
    let inst =
        qa_insert_instance(&pool, "QA-非认证域", "some-other-flow", QA_USER_A, QA_ACTOR).await;

    let (messaging, mut inbox) = qa_messaging();
    approvals::handle_registration_activation_event(
        &pool,
        &(messaging.clone() as std::sync::Arc<dyn common::messaging::MessagingService>),
        &qa_completed_event(inst, "approved"),
    )
    .await;

    let status: String =
        sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(QA_USER_A)
            .fetch_one(&pool)
            .await
            .expect("user status");
    assert_eq!(
        status, "pending_approval",
        "非认证域实例 MUST NOT 触发用户状态变更"
    );
    assert!(
        qa_collect(&mut inbox).is_empty(),
        "非认证域实例 MUST NOT 产生结果通知"
    );
    qa_cleanup(&pool).await;
}

/// 装配路径（广播订阅 → 后台消费）在 actix 运行时下可达：
/// 发布 `ApprovalCompleted` → 订阅者落副作用（生产装配 = host 的同一函数）。
#[actix_web::test]
async fn t_registration_fallback_subscription_wiring_delivers() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;
    qa_cleanup(&pool).await;
    qa_insert_applicant(&pool, QA_USER_A, "qa-sub-wiring").await;
    let inst = qa_insert_instance(
        &pool,
        "QA-装配可达",
        "user-register-approval",
        QA_USER_A,
        QA_ACTOR,
    )
    .await;

    let (messaging, _inbox) = qa_messaging();
    let bus = std::sync::Arc::new(common::event_bus::InMemoryEventBus::new());
    approvals::subscribe_registration_activation_events(
        bus.clone() as std::sync::Arc<dyn common::event_bus::DomainEventBus>,
        pool.clone(),
        messaging.clone() as std::sync::Arc<dyn common::messaging::MessagingService>,
    );
    tokio::time::sleep(std::time::Duration::from_millis(100)).await; // 等订阅登记
    bus.publish("ApprovalCompleted", &qa_completed_event(inst, "approved"))
        .await
        .expect("publish");

    let status = qa_wait_status(&pool, QA_USER_A, "active").await;
    assert_eq!(
        status, "active",
        "订阅装配后发布事件 MUST 驱动副作用（广播消费可达）"
    );
    qa_cleanup(&pool).await;
}

// ── T1.15: 注册审批通过 ⇒ employee UA 授予（register-approval-closure :: employee-base-access）───
//
// `apply_registration_activation` 在**本次真正发生状态迁移**（applied=true）后同点调用
// `approval::ngac_ensure::ensure_employee_ua`：通过 ⇒ 用户持 employee UA；重复事件/重复调用
// MUST NOT 产生重复关联行；驳回（disabled）MUST NOT 授予（幂等由 ensure_employee_ua 自持）。

/// employee UA 有效关联行数（判据 = ngac_user_rr_attribute ⋈ o_name='employee'）。
async fn employee_ua_rows(pool: &PgPool, user_id: i64) -> i64 {
    sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl_auth.ngac_user_rr_attribute rr
           JOIN isahl_auth.ngac_user_attribute ua ON ua.id = rr.fk_user_attribute
           WHERE rr.fk_user = $1 AND ua.o_name = 'employee'
             AND rr.deleted_at IS NULL AND ua.deleted_at IS NULL"#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .expect("employee UA count")
}

#[tokio::test]
async fn t1_15_registration_approval_grants_employee_ua() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;

    // 前置：seed 自愈（employee UA 供给 + FLOW-AUTHORIZATION 模板 + 72h 维度）
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    const G_USER: i64 = -99411; // 通过样本
    const G_OPER: i64 = -99421;
    const G_USER_REJ: i64 = -99441; // 驳回样本
    const G_OPER_REJ: i64 = -99431;

    // 清理残留（关联行 → 用户 → 实例 → 桥行）
    for uid in [G_USER, G_USER_REJ] {
        let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
            .bind(uid)
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
            .bind(uid)
            .execute(&pool)
            .await;
    }
    for oid in [G_OPER, G_OPER_REJ] {
        let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
            .bind(oid)
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM isahl.zc_id_operation_rr_event WHERE ref_left = $1")
            .bind(oid)
            .execute(&pool)
            .await;
    }

    // 两个 pending_approval 样本 + 注册审批实例（code=user-register-approval，fk_subject=申请人）
    for (uid, name) in [(G_USER, "t1_15_pass"), (G_USER_REJ, "t1_15_rej")] {
        sqlx::query(
            "INSERT INTO isahl_auth.auth_users (id, username, status, is_active, user_type, created_at, updated_at) \
             VALUES ($1, $2, 'pending_approval', true, 'standard', NOW(), NOW())",
        )
        .bind(uid)
        .bind(name)
        .execute(&pool)
        .await
        .expect("insert pending user");
    }
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JE", "FTA", "↓_EZ"))
            .await
            .expect("resolve oper-approve coords");
    for (oper_id, uid, notice) in [
        (G_OPER, G_USER, "T1.15通过样本"),
        (G_OPER_REJ, G_USER_REJ, "T1.15驳回样本"),
    ] {
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_oper-approve"
               (id, notice, code, fk_subject, created_by_id, created_at, updated_at,
                dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, 'user-register-approval', $3, $3, NOW(), NOW(), $4, $5, $6)"#,
        )
        .bind(oper_id)
        .bind(notice)
        .bind(uid)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .execute(&pool)
        .await
        .expect("insert registration instance");
    }

    // ── 通过路径：状态迁移 + employee UA 授予 ──
    assert_eq!(
        employee_ua_rows(&pool, G_USER).await,
        0,
        "前置：未审批用户不持 employee UA"
    );
    let applied = approvals::apply_registration_activation(&pool, G_OPER, "active", None)
        .await
        .expect("apply active");
    assert!(applied, "首次激活 MUST 真正发生状态迁移");
    let status: String =
        sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(G_USER)
            .fetch_one(&pool)
            .await
            .expect("user status");
    assert_eq!(status, "active");
    assert_eq!(
        employee_ua_rows(&pool, G_USER).await,
        1,
        "审批通过 ⇒ 用户 MUST 持 employee UA（employee-base-access）"
    );

    // ── 幂等：重复调用（重复事件 / 内联+订阅双写径）不重复指派 ──
    let applied_again = approvals::apply_registration_activation(&pool, G_OPER, "active", None)
        .await
        .expect("re-apply active");
    assert!(
        !applied_again,
        "状态已迁移 ⇒ 幂等返回 false（不重复副作用，结果通知恰好一次）"
    );
    assert_eq!(
        employee_ua_rows(&pool, G_USER).await,
        1,
        "重复调用 MUST NOT 产生重复 employee UA 关联行"
    );

    // ── 驳回路径：MUST NOT 授予 ──
    let applied_rej = approvals::apply_registration_activation(&pool, G_OPER_REJ, "disabled", None)
        .await
        .expect("apply disabled");
    assert!(applied_rej, "首次驳回 MUST 真正发生状态迁移");
    let status_rej: String =
        sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(G_USER_REJ)
            .fetch_one(&pool)
            .await
            .expect("rejected user status");
    assert_eq!(status_rej, "disabled");
    assert_eq!(
        employee_ua_rows(&pool, G_USER_REJ).await,
        0,
        "驳回 ⇒ MUST NOT 授予 employee UA"
    );

    // 清理
    for uid in [G_USER, G_USER_REJ] {
        let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
            .bind(uid)
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
            .bind(uid)
            .execute(&pool)
            .await;
    }
    for oid in [G_OPER, G_OPER_REJ] {
        let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
            .bind(oid)
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM isahl.zc_id_operation_rr_event WHERE ref_left = $1")
            .bind(oid)
            .execute(&pool)
            .await;
    }
}

// ── T1.16: 外部入驻通道断链自愈（通道映射表单源：外部 code 进入 oper→even 重建覆盖）───
//
// `external-subject-register-approval` 原为 `_ => None` 缺省分支的排除面（断链静默跳过）。
// 自检现按 `common::registration_approval::APPROVAL_CHANNELS` 遍历（三通道同等待遇），
// 重建目标（流程模板 / 叶表 / 文案模板）取自映射表 ⇒ 外部通道断链 MUST 自愈：
// 事件落 zc_id_appr-authorization 叶表、tpl_id 绑 FLOW-EXTERNAL-SUBJECT 节点，
// notice/comments 按通道模板拆分/拼接（wire-registration-approval-closure 1.2）。

const EXT2_OPER: i64 = -99451; // 外部通道断链实例（oper-approve）

#[tokio::test]
async fn t1_16_external_subject_broken_oper_heals_via_channel_map() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;

    // 前置：模型级种子重放（FLOW-EXTERNAL-SUBJECT 模板 + 72h 时长——组件零模板）
    alioth_gateway::seed::ensure_startup_seed_self_check(&pool).await;
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    // 清理残留 fixture（桥行 → 实例 → 叶表事件 → 用户）
    sqlx::query(r#"DELETE FROM isahl.zc_id_operation_rr_event WHERE ref_left = $1"#)
        .bind(EXT2_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(EXT2_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_appr-authorization" WHERE notice = '外部主体 t116-ext 入驻审批'"#,
    )
    .execute(&pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username = 't116-ext'")
        .execute(&pool)
        .await
        .ok();

    // 真实主体用户（自愈要求 fk_subject 存在于 auth_users 且活跃）
    let uid: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.auth_users
           (id, username, user_type, status, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES (isahl.gen_next_zuid(), 't116-ext',
                   'external', 'pending_approval', TRUE,
                   NOW(), NOW(), 0, '{}'::jsonb)
           RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("insert external subject user");

    // 构造外部通道断链 oper 实例（notice 命中通道模板「外部主体 {} 入驻审批」）
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JE", "FTA", "↓_EZ"))
            .await
            .expect("resolve oper-approve coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (id, notice, code, fk_subject, created_by_id, created_at, updated_at,
            dk_scene, dk_factor, dk_function)
           VALUES ($1, '外部主体 t116-ext 入驻审批', 'external-subject-register-approval',
                   $2, $2, NOW(), NOW(), $3, $4, $5)"#,
    )
    .bind(EXT2_OPER)
    .bind(uid)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("insert external broken oper fixture");

    // 前置：确认断链（无指向活跃 even-approve 事件的 rr_event 桥行）
    let broken_before: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           WHERE oa.id = $1 AND oa.deleted_at IS NULL
             AND NOT EXISTS (
                 SELECT 1 FROM isahl.zc_id_operation_rr_event rr
                 JOIN isahl."zc_id_even-approve" e ON e.id = rr.ref_right AND e.deleted_at IS NULL
                 WHERE rr.ref_left = oa.id AND rr.deleted_at IS NULL)"#,
    )
    .bind(EXT2_OPER)
    .fetch_one(&pool)
    .await
    .expect("broken count before");
    assert_eq!(broken_before, 1, "前置：外部通道 oper 应为断链");

    // 自检 → 外部通道进入重建覆盖（映射表遍历）：重建事件 + 回填 rr_event 桥行
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    // 断言 1：桥行已回填为真实存在的 even 事件
    let ev_id: i64 = sqlx::query_scalar(
        r#"SELECT rr.ref_right FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event rr
             ON rr.ref_left = oa.id AND rr.deleted_at IS NULL
           JOIN isahl."zc_id_even-approve" e ON e.id = rr.ref_right AND e.deleted_at IS NULL
           WHERE oa.id = $1 AND oa.deleted_at IS NULL"#,
    )
    .bind(EXT2_OPER)
    .fetch_one(&pool)
    .await
    .expect("external oper bridge row");

    // 断言 2：事件落 authorization 叶表，code/notice 与 oper 一致
    let (ev_code, ev_notice, ev_comments): (String, String, String) = sqlx::query_as(
        r#"SELECT code, notice, comments FROM isahl."zc_id_appr-authorization"
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(ev_id)
    .fetch_one(&pool)
    .await
    .expect("healed external event in authorization leaf");
    assert_eq!(ev_code, "external-subject-register-approval");
    assert_eq!(ev_notice, "外部主体 t116-ext 入驻审批");
    // 断言 3：comments = 通道模板 + notice 前缀/后缀拆分出的申请人名（映射表单源消费）
    assert_eq!(
        ev_comments,
        format!("外部主体入驻审批：申请人 t116-ext（用户 id {uid}）"),
        "comments MUST 按通道模板拆分/拼接（前缀/后缀自 notice_tpl 反解）"
    );

    // 断言 4：tpl_id 绑 FLOW-EXTERNAL-SUBJECT 节点（而非 FLOW-AUTHORIZATION）
    let ext_flow_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl.zc_id_process WHERE code = 'FLOW-EXTERNAL-SUBJECT' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .expect("external flow query")
    .expect("FLOW-EXTERNAL-SUBJECT 模板应由模型级种子供给");
    let tpl_id: Option<i64> =
        sqlx::query_scalar(r#"SELECT tpl_id FROM isahl."zc_id_even-approve" WHERE id = $1"#)
            .bind(ev_id)
            .fetch_one(&pool)
            .await
            .expect("tpl query");
    let tpl_in_ext: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (SELECT 1 FROM isahl.zc_id_process_rr_operation rro
                          WHERE rro.ref_left = $1 AND rro.ref_right = $2 AND rro.deleted_at IS NULL)"#,
    )
    .bind(ext_flow_id)
    .bind(tpl_id)
    .fetch_one(&pool)
    .await
    .expect("tpl binding query");
    assert!(
        tpl_in_ext,
        "事件 tpl_id 应绑 FLOW-EXTERNAL-SUBJECT 节点（通道映射表 heal flow），tpl={tpl_id:?}"
    );

    // 断言 5：二次自检不重复重建（仍只 1 条该 notice 事件）
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;
    let ev_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_appr-authorization"
           WHERE notice = '外部主体 t116-ext 入驻审批' AND deleted_at IS NULL"#,
    )
    .fetch_one(&pool)
    .await
    .expect("event count after second check");
    assert_eq!(ev_count, 1, "二次自检不得重复重建 even 事件");

    // 清理（含桥行）
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(EXT2_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_operation_rr_event WHERE ref_left = $1"#)
        .bind(EXT2_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_appr-authorization" WHERE notice = '外部主体 t116-ext 入驻审批'"#,
    )
    .execute(&pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(uid)
        .execute(&pool)
        .await
        .ok();
}
