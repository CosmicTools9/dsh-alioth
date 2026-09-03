//! fix-approval-engine-gap-closure D8 委托闭环后端集成测试
//!
//! ① name-only 创建委托规则（前端只发受托人姓名）：
//!    fk_subject = 创建者归因；fk_operator 按 req.name 解析活跃 auth_users；
//!    解析不到 → Validation 400（fail-closed，无死规则）。
//! ② 规则落库后引擎推进创建实例自动转派命中：委托者节点实例通过后，
//!    下游实例创建时 apply_delegation 把 fk_operator 改派为受托人。

use ::common::testing::connect_test_db;
use actix_web::{dev::Service as _, test, web, App, HttpMessage};
use approval::models::CreateDelegationRuleRequest;
use approval::services::DelegationRuleService;
use serde_json::json;
use sqlx::PgPool;

mod common;
use common::{ensure_role_member, setup_test_schema, wire_approval_node};

const DELEGATOR: i64 = 444020;
const TRUSTEE: i64 = 444021;

async fn seed_user(pool: &PgPool, id: i64, username: &str) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, $2, $2, $3, 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(id)
    .bind(username)
    .bind(format!("{username}@test.local"))
    .execute(pool)
    .await
    .unwrap();
}

/// 节点 = 操作叶行（refactor-flow-node-operation-model，同 assignment_test）：
/// even-approve 模板 + oper-approve 主体 + 模板桥 + rro + 岗位接线。
async fn add_approve_node(
    pool: &PgPool,
    flow_id: i64,
    code: &str,
    assignees: &[i64],
    sign_mode: &str,
) -> i64 {
    let template_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, created_by_id, code, comments)
           VALUES ($1, 1, $2, $3) RETURNING id"#,
    )
    .bind(code)
    .bind(code)
    .bind(code)
    .fetch_one(pool)
    .await
    .unwrap();
    let op_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, code, created_by_id)
           VALUES ($1, $2, 1) RETURNING id"#,
    )
    .bind(code)
    .bind(code)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(op_id)
    .bind(template_id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_process_rr_operation
           (id, code, ref_left, ref_right, comments, "next-ops", created_by_id)
           VALUES (isahl.gen_next_uid(791), $1, $2, $3, $4, '[]'::jsonb, 1)"#,
    )
    .bind(code)
    .bind(flow_id)
    .bind(op_id)
    .bind("approve")
    .execute(pool)
    .await
    .unwrap();
    wire_approval_node(pool, op_id, assignees, sign_mode)
        .await
        .unwrap();
    op_id
}

async fn call_approve(
    pool: &PgPool,
    actor: i64,
    instance_id: i64,
) -> actix_web::dev::ServiceResponse {
    let ctx = ::common::context::RequestContext::with_username(actor, "actor@test", "actor");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(std::sync::Arc::new(
                ::common::event_bus::InMemoryEventBus::new(),
            )
                as std::sync::Arc<dyn ::common::event_bus::DomainEventBus>))
            .service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(approval::handlers::approve_reject::register),
            ),
    )
    .await;
    let req = test::TestRequest::post()
        .uri(&format!("/test/approval-instances/{}/approve", instance_id))
        .set_json(json!({"opinion": "同意"}))
        .to_request();
    test::call_service(&app, req).await
}

/// 首实例（发起人=审批人）：实例挂节点事件模板（tpl_id=node op）
async fn create_first_instance(pool: &PgPool, node_id: i64, actor: i64) -> i64 {
    let template: i64 = sqlx::query_scalar(
        r#"SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
           WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(node_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let instance_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (id, notice, code, fk_subject, fk_operator, created_by_id, tpl_id)
           VALUES (isahl.gen_next_zuid(), '发起', 'START', $1, $1, 1, $2)
           RETURNING id"#,
    )
    .bind(actor)
    .bind(node_id)
    .fetch_one(pool)
    .await
    .unwrap();
    // 实例 ↔ 节点事件模板桥（advance_flow 反查流程/节点依赖此桥）
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

/// 节点引擎实例（tpl_id 非空）：(id, fk_operator)
async fn node_instance_operators(pool: &PgPool, node_id: i64) -> Vec<(i64, Option<i64>)> {
    let template: i64 = sqlx::query_scalar(
        r#"SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
           WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(node_id)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_as(
        r#"SELECT oa.id, oa.fk_operator FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = oa.id AND oe.ref_right = $1
           WHERE oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL
           ORDER BY oa.id"#,
    )
    .bind(template)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn cleanup_dl_users(pool: &PgPool) {
    // 共享测试库防串扰：清除历次运行的专属委托测试身份（含曾用名变体）
    sqlx::query(
        r#"DELETE FROM isahl_auth.auth_users
           WHERE id IN ($1, $2, 444001, 444002, 444012)
              OR username IN ('dl-principal', 'dl-trustee', 'dlp-principal', 'dlp-trustee')"#,
    )
    .bind(DELEGATOR)
    .bind(TRUSTEE)
    .execute(pool)
    .await
    .unwrap();
}

/// D8-1：name-only 创建 → fk_subject 创建者归因 + fk_operator 按姓名解析；不可解析 → 400
#[tokio::test]
async fn delegation_name_only_create_resolves_operator_and_attrib_subject() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    cleanup_dl_users(&pool).await;
    seed_user(&pool, DELEGATOR, "dl-principal").await;
    seed_user(&pool, TRUSTEE, "dl-trustee").await;

    let repo = approval::repositories::DelegationRuleRepository::new(pool.clone());
    let svc = DelegationRuleService::new(repo);
    let rule = svc
        .create(
            CreateDelegationRuleRequest {
                name: "dl-trustee".to_string(),
                code: Some(format!("DL-RULE-{}", TRUSTEE)),
                fk_subject: None,
                fk_operator: None,
                comments: None,
                date_st: None,
                date_ed: None,
            },
            DELEGATOR,
        )
        .await
        .expect("name-only 创建应成功");
    assert_eq!(rule.fk_subject, Some(DELEGATOR), "fk_subject 应为创建者");
    assert_eq!(rule.fk_operator, Some(TRUSTEE), "fk_operator 应按姓名解析");

    // fail-closed：解析不到 → Validation 400
    let err = svc
        .create(
            CreateDelegationRuleRequest {
                name: "dl-no-such-user".to_string(),
                code: Some(format!("DL-RULE-{}-BAD", TRUSTEE)),
                fk_subject: None,
                fk_operator: None,
                comments: None,
                date_st: None,
                date_ed: None,
            },
            DELEGATOR,
        )
        .await
        .expect_err("不可解析姓名应 400");
    assert!(
        err.to_string().contains("受托人不可解析"),
        "错误信息应含受托人不可解析: {err}"
    );
}

/// D8-2：委托规则命中引擎实例创建 → apply_delegation 自动转派 fk_operator
#[tokio::test]
async fn delegation_rule_hits_on_downstream_instance_creation() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    cleanup_dl_users(&pool).await;
    seed_user(&pool, DELEGATOR, "dl-principal").await;
    seed_user(&pool, TRUSTEE, "dl-trustee").await;
    ensure_role_member(&pool, "dl-role", DELEGATOR)
        .await
        .unwrap();
    common::grant_user_access(&pool, DELEGATOR, "approval-instances", &["approve"])
        .await
        .expect("grant approve access");

    // 委托规则：委托者 = DELEGATOR（引擎解析出的当前审批人），受托人按姓名解析
    let repo = approval::repositories::DelegationRuleRepository::new(pool.clone());
    let svc = DelegationRuleService::new(repo);
    svc.create(
        CreateDelegationRuleRequest {
            name: "dl-trustee".to_string(),
            code: Some(format!("DL-RULE-HIT-{}", TRUSTEE)),
            fk_subject: Some(DELEGATOR),
            fk_operator: None,
            comments: None,
            date_st: Some(chrono::Utc::now() - chrono::Duration::days(1)),
            date_ed: Some(chrono::Utc::now() + chrono::Duration::days(1)),
        },
        DELEGATOR,
    )
    .await
    .expect("规则创建应成功");

    // n1 → n2 双审批节点，审批人均为 DELEGATOR（委托者）
    let flow_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve" (notice, code, comments, created_by_id, _f_, _t_)
           VALUES ('委托转派流程', 'DG-DL-FLOW', 'dl-test', 1, '实现', '范例')
           RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let n1 = add_approve_node(&pool, flow_id, "n1", &[DELEGATOR], "or_sign").await;
    let n2 = add_approve_node(&pool, flow_id, "n2", &[DELEGATOR], "or_sign").await;
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow_id)
    .bind(json!([n2]))
    .bind(n1)
    .execute(&pool)
    .await
    .unwrap();

    // n1 首实例（发起人=审批人=委托者）通过 → 推进创建下游实例
    let first = create_first_instance(&pool, n1, DELEGATOR).await;
    let _resp = call_approve(&pool, DELEGATOR, first).await;
    // 下游 n2 实例创建时 apply_delegation 命中 → fk_operator 转派 TRUSTEE
    let n2_insts = node_instance_operators(&pool, n2).await;
    assert_eq!(n2_insts.len(), 1, "n2 应创建一枚下游实例: {n2_insts:?}");
    assert_eq!(
        n2_insts[0].1,
        Some(TRUSTEE),
        "委托规则命中：下游实例 fk_operator 应转派为受托人（实际 {:?}）",
        n2_insts[0].1
    );
}
