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

    for (id, operator, creator, notice) in [
        (APPR_OPERATOR_A, Some(USER_A), USER_B, "T1.4待我审批"),
        (APPR_CREATED_A, None, USER_A, "T1.4我发起的"),
        (APPR_OTHER, Some(USER_B), USER_B, "T1.4他人审批"),
    ] {
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_oper-approve"
               (id, notice, created_at, updated_at, created_by_id, fk_operator)
               VALUES ($1, $4, NOW(), NOW(), $3, $2)"#,
        )
        .bind(id)
        .bind(operator)
        .bind(creator)
        .bind(notice)
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

// ── T1.3: GET /approvals/{id} 意见链 ────────────────────────────────────────

#[tokio::test]
async fn t1_4_detail_returns_opinion_chain() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;
    setup_approval_fixtures(&pool).await;

    // 先 approve 一次产生意见
    let resp = ApprovalService::execute(
        &pool,
        APPR_OPERATOR_A,
        "approved",
        Some(ApprovalActor {
            user_id: USER_A,
            opinion: Some("首签意见".to_string()),
        }),
        None,
    )
    .await;
    assert!(resp.success);

    let req = req_with_user(USER_A);
    let resp = approvals::get_approval_detail(
        req,
        web::Data::new(pool.clone()),
        web::Path::from(APPR_OPERATOR_A),
    )
    .await;
    let body = to_bytes(resp.into_body()).await.expect("body bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("detail json");

    assert_eq!(json["success"].as_bool(), Some(true));
    assert_eq!(json["data"]["id"].as_i64(), Some(APPR_OPERATOR_A));
    assert_eq!(json["data"]["operator_id"].as_i64(), Some(USER_A));
    assert_eq!(json["data"]["mine"].as_bool(), Some(false));
    // add-approval-flow-self-check：详情返回完整 ApprovalItem 契约
    assert_eq!(json["data"]["title"].as_str(), Some("T1.4待我审批"));
    assert_eq!(json["data"]["applicant"].as_str(), Some("未知用户")); // 无 fk_subject → fallback
    assert_eq!(json["data"]["code"].as_str(), Some(""));
    assert!(json["data"]["status"].as_str().is_some());
    assert!(json["data"]["time"].as_str().is_some());

    let opinions = json["data"]["opinions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(opinions.len(), 1, "应有一条意见记录");
    // action 为短码（审批通过 → 通过），对齐前端链节点严格相等判定
    assert_eq!(opinions[0]["action"].as_str(), Some("通过"));
    assert_eq!(opinions[0]["opinion"].as_str(), Some("首签意见"));
    // author：审批人（fixture 无 auth_users 行 → fallback 空串）
    assert_eq!(opinions[0]["author"].as_str(), Some(""));

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
const VERIFY_OPER: i64 = -99024; // user-verify 断链实例（不应被注册审批自愈误重建）
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

    // FLOW-USER-REGISTER 流程种子：恰好 1 行（唯一写入方）
    assert_eq!(flow_seed_count(&pool, "FLOW-USER-REGISTER").await, 1);
}

// ── T1.8: 注册审批实例补链自愈（even-approve 缺 oper-approve → 补建）─────────

#[tokio::test]
async fn t1_8_self_check_backfills_registration_instance() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;

    // 清理残留 fixture（先桥行后实例——桥行引用实例 id）
    sqlx::query(
        r#"DELETE FROM isahl.zc_id_operation_rr_event rr
           WHERE rr.ref_right = $1
             AND rr.ref_left IN (SELECT id FROM isahl."zc_id_oper-approve" WHERE code = 'user-register-approval')"#,
    )
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
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_even-approve" (id, notice, code, comments, created_by_id, created_at, updated_at)
           VALUES ($1, 'T1.8注册审批', 'user-register-approval', $2, $3, NOW(), NOW())"#,
    )
    .bind(REG_EVENT)
    .bind("访问授权审批：申请人 t18")
    .bind(USER_A)
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

    // 清理前次运行残留（username/email 唯一约束）
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username = 'reapply-user'")
        .execute(&pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.auth_users WHERE email LIKE 'reapply-user-%@test.local'",
    )
    .execute(&pool)
    .await;

    // 构造 rejected 用户（email 用时间戳保证唯一，防前次运行残留）
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let uid: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, status, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES (isahl.gen_next_zuid(), 'reapply-user', 'reapply-user',
                   $1, 'standard', 'rejected', TRUE,
                   NOW(), NOW(), 0, '{}'::jsonb)
           RETURNING id"#,
    )
    .bind(format!("reapply-user-{}@test.local", suffix))
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
    let instance_id = json["instance_id"].as_i64().expect("instance id");

    // 断言 1：状态回 pending_approval
    let status: String =
        sqlx::query_scalar("SELECT status FROM isahl_auth.auth_users WHERE id = $1")
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
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // 夹具幂等：上次失败中断的残留先行清理（name 唯一键 t110-user）
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE name = 't110-user'")
        .execute(&pool)
        .await
        .ok();
    let uid: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, status, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES (isahl.gen_next_zuid(), 't110-user', 't110-user',
                   $1, 'standard', 'active', TRUE,
                   NOW(), NOW(), 0, '{}'::jsonb)
           RETURNING id"#,
    )
    .bind(format!("t110-user-{}@test.local", suffix))
    .fetch_one(&pool)
    .await
    .expect("insert subject user");

    // 构造断链 oper 实例（code=user-register-approval，无 rr_event 桥 = 断链）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (id, notice, code, fk_subject, created_by_id, created_at, updated_at)
           VALUES ($1, 'T1.10访问授权审批', 'user-register-approval', $2, $2, NOW(), NOW())"#,
    )
    .bind(REG_OPER)
    .bind(uid)
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
        "INSERT INTO isahl_auth.auth_users (id, name, username, status, is_active, user_type, created_at, updated_at) \
         VALUES ($1, 'T1.11注册用户', 't1_11_user', 'pending_approval', true, 'standard', NOW(), NOW())",
    )
    .bind(REG_USER)
    .execute(&pool)
    .await
    .expect("insert pending user");

    // 审批事件 + 实例（code=user-register-approval，fk_subject=REG_USER，comments 纯文本）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_even-approve" (id, notice, code, comments, created_by_id, created_at, updated_at)
           VALUES ($1, 'T1.11注册审批', 'user-register-approval', $2, $3, NOW(), NOW())"#,
    )
    .bind(REG_EVENT_11)
    .bind("访问授权审批：申请人 t1_11_user")
    .bind(REG_USER)
    .execute(&pool)
    .await
    .expect("insert registration event");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_oper-approve" (id, notice, code, fk_subject, fk_operator, created_by_id, created_at, updated_at)
           VALUES ($1, 'T1.11注册审批', 'user-register-approval', $2, $3, $2, NOW(), NOW())"#,
    )
    .bind(REG_OPER_11)
    .bind(REG_USER)
    .bind(USER_A)
    .execute(&pool)
    .await
    .expect("insert registration instance");
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
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
        sqlx::query_scalar("SELECT status FROM isahl_auth.auth_users WHERE id = $1")
            .bind(REG_USER)
            .fetch_one(&pool)
            .await
            .expect("user status");
    assert_eq!(status, "active", "注册审批通过后用户 MUST 激活");

    // 幂等守卫：再次 approve（实例已 approved，handler 拒绝——但即使成功 status 已 active 不再变）
    // 实际验证：直接调用激活函数对 active 用户不降级
    let _ = approvals::apply_registration_activation(&pool, REG_OPER_11, "disabled").await;
    let status2: String =
        sqlx::query_scalar("SELECT status FROM isahl_auth.auth_users WHERE id = $1")
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
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (id, notice, code, fk_subject, created_by_id, created_at, updated_at)
           VALUES ($1, 'T1.12访问授权审批', 'user-register-approval', $2, $2, NOW(), NOW())"#,
    )
    .bind(REG_OPER)
    .bind(-99099) // 不存在的用户 id
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
//
// 自愈契约（fix-approval-event-adaptive-write）：oper→even 重建仅针对
// user-register-approval（绑 FLOW-AUTHORIZATION + 72h SLA，与写入契约一致）。
// user-verify 走独立流程模板 FLOW-USER-VERIFY，无对应自愈逻辑——若误按
// AUTHORIZATION_FLOW_CODE 重建会绑错流程，故断链维持告警人工处理，不触发重建。

#[tokio::test]
async fn t1_11_user_verify_broken_not_misreconstructed() {
    let _ = env_logger::builder().is_test(true).try_init();
    let pool = connect_test_db().await;

    // 前置：seed 模板
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    // 清理残留 fixture
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(VERIFY_OPER)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_even-approve" WHERE notice = 'T1.11实名审核'"#)
        .execute(&pool)
        .await
        .ok();

    // 构造 user-verify 断链 oper 实例（无 rr_event 桥 = 断链）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (id, notice, code, fk_subject, created_by_id, created_at, updated_at)
           VALUES ($1, 'T1.11实名审核', 'user-verify', $2, $2, NOW(), NOW())"#,
    )
    .bind(VERIFY_OPER)
    .bind(USER_A)
    .execute(&pool)
    .await
    .expect("insert user-verify broken oper fixture");

    // 自检
    alioth_gateway::seed::ensure_gateway_seed_self_check(&pool).await;

    // 断言 1：桥行未被回填（user-verify 断链不参与 oper→even 自愈）
    let bridges: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_operation_rr_event rr
           WHERE rr.ref_left = $1 AND rr.deleted_at IS NULL"#,
    )
    .bind(VERIFY_OPER)
    .fetch_one(&pool)
    .await
    .expect("oper bridge count");
    assert_eq!(bridges, 0, "user-verify 断链不应被重建回填桥行");

    // 断言 2：未产生 user-verify 事件（notice 匹配的 even 行不应存在）
    let ev_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_even-approve"
           WHERE notice = 'T1.11实名审核' AND deleted_at IS NULL"#,
    )
    .fetch_one(&pool)
    .await
    .expect("even count");
    assert_eq!(ev_count, 0, "user-verify 不应被误重建事件");

    // 清理
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(VERIFY_OPER)
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
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let uid: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, status, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES (isahl.gen_next_zuid(), 't113-user', 't113-user',
                   $1, 'standard', 'pending_approval', FALSE,
                   NOW(), NOW(), 0, '{}'::jsonb)
           RETURNING id"#,
    )
    .bind(format!("t113-user-{}@test.local", suffix))
    .fetch_one(&pool)
    .await
    .expect("insert inactive subject user");

    // 构造断链 oper：fk_subject = 该停用用户，无 rr_event 桥行
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (id, notice, code, fk_subject, created_by_id, created_at, updated_at)
           VALUES ($1, 'T1.13访问授权审批', 'user-register-approval', $2, $2, NOW(), NOW())"#,
    )
    .bind(INACTIVE_OPER)
    .bind(uid)
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
           (id, name, username, user_type, status, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES (isahl.gen_next_zuid(), 'ext-subj-act-user', 'ext-subj-act-user',
                   'external', 'pending_approval', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("insert external user");

    // 构造外部入驻审批实例（外部 code——激活链扩展匹配面）
    let instance_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (notice, code, fk_subject, created_by_id, created_at, updated_at)
           VALUES ('T-EXT 外部主体入驻审批', 'external-subject-register-approval', $1, $1, NOW(), NOW())
           RETURNING id"#,
    )
    .bind(uid)
    .fetch_one(&pool)
    .await
    .expect("insert external instance");

    // 通过 → 激活
    approvals::apply_registration_activation(&pool, instance_id, "active")
        .await
        .expect("activation");
    let status: String =
        sqlx::query_scalar("SELECT status FROM isahl_auth.auth_users WHERE id = $1")
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
    approvals::apply_registration_activation(&pool, instance_id, "disabled")
        .await
        .expect("disable");
    let status: String =
        sqlx::query_scalar("SELECT status FROM isahl_auth.auth_users WHERE id = $1")
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
