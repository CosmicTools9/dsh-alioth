//! commitment factor — 完整性 E2E 集成测试
//!
//! 覆盖 approve_reject_test.rs 未触及的实体：
//! - ApprovalFlow（zc_id_process）
//! - FlowNode（zc_id_even-approve）
//! - DelegationRule（zc_id_operation）
//! - ApprovalAction（zc_id_deta-opinion — 非审批流程的 CRUD）
//! - Timeline 查询
//!
//! 数据依赖：
//! - zc_id_process（审批流程）
//! - zc_id_even-approve（审批事件，兼作流程节点）
//! - zc_id_operation（委托规则）
//! - zc_id_deta-opinion（审批意见）
//! - zc_id_subj-employee（审批人）
//! - zc_id_oper-approve（审批实例）

use ::common::testing::connect_test_db;
use sqlx::PgPool;
mod common;
use common::{setup_test_schema, test_code};

// ── Helpers ─────────────────────────────────────────────────────────────────

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

/// 节点构造（refactor-flow-node-operation-model）：even-approve 模板 +
/// operation 主体（oper-approve 子类）+ 模板桥（rr_event）。返回 operation id。
/// 节点在册锚（process_rr_operation）由调用方按需建立（见 flow_node_create_linked_to_flow）。
async fn insert_flow_node(pool: &PgPool, label: &str, _fk_flow: i64) -> i64 {
    let template: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(label)
    .fetch_one(pool)
    .await
    .unwrap();
    let op: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(label)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(op)
    .bind(template)
    .execute(pool)
    .await
    .unwrap();
    op
}

async fn insert_delegation_rule(pool: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl.zc_id_operation (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_approval_action(
    pool: &PgPool,
    summary: &str,
    fk_biller: i64,
    fk_list: i64,
    qk_date: Option<i64>,
) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_deta-opinion"
           (notice, opinion, fk_list, fk_biller, qk_date, created_at)
           VALUES ($1, $2, $3, $4, $5, NOW()) RETURNING id"#,
    )
    .bind(summary)
    .bind("comments placeholder")
    .bind(fk_list)
    .bind(fk_biller)
    .bind(qk_date)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_engineer(pool: &PgPool, notice: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_subj-employee" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(notice)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_approval_instance(
    pool: &PgPool,
    notice: &str,
    event_id: i64,
    fk_subject: i64,
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
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(instance_id)
    .bind(event_id)
    .execute(pool)
    .await
    .unwrap();
    instance_id
}

async fn count_flows(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*) FROM isahl.zc_id_process WHERE deleted_at IS NULL"#,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn approval_flow_create_read() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let flow_name = test_code("e2e-flow");
    let flow_id = insert_flow(&pool, &flow_name).await;

    let row: (i64, String) =
        sqlx::query_as(r#"SELECT id, notice FROM isahl.zc_id_process WHERE id = $1"#)
            .bind(flow_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(row.0, flow_id, "flow id must match");
    assert_eq!(row.1, flow_name, "flow notice must match inserted name");
}

#[tokio::test]
async fn flow_node_create_linked_to_flow() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    // 先创建流程
    let flow_name = test_code("e2e-flow-node-parent");
    let flow_id = insert_flow(&pool, &flow_name).await;

    // 创建流程节点（返回 operation 主体 id；模板经桥反查）
    let node_label = test_code("e2e-flow-node");
    let node_id = insert_flow_node(&pool, &node_label, flow_id).await;
    // 节点在册锚：process_rr_operation（ref_right=operation）——even 行自身无
    // fk_process，流程归属经桥链反查
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_process_rr_operation
           (id, code, ref_left, ref_right, comments, created_by_id)
           VALUES (isahl.gen_next_uid(791), 'approve', $1, $2, $3, 1)"#,
    )
    .bind(flow_id)
    .bind(node_id)
    .bind(&node_label)
    .execute(&pool)
    .await
    .unwrap();

    let template: i64 = sqlx::query_scalar(
        r#"SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
           WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(node_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let row: (i64, String, Option<i64>) = sqlx::query_as(
        r#"SELECT n.id, n.notice,
                  (SELECT rro.ref_left
                   FROM isahl.zc_id_operation_rr_event oe
                   JOIN isahl.zc_id_process_rr_operation rro
                     ON rro.ref_right = oe.ref_left AND rro.deleted_at IS NULL
                   WHERE oe.ref_right = n.id AND oe.deleted_at IS NULL
                   ORDER BY oe.created_at LIMIT 1)
           FROM isahl."zc_id_even-approve" n WHERE n.id = $1"#,
    )
    .bind(template)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.0, template, "node template id must match");

    assert_eq!(row.1, node_label, "node notice must match inserted label");
    assert_eq!(
        row.2,
        Some(flow_id),
        "node must link to flow via bridge chain"
    );
}

#[tokio::test]
async fn delegation_rule_create_read() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let rule_name = test_code("e2e-delegation");
    let rule_id = insert_delegation_rule(&pool, &rule_name).await;

    let row: (i64, String) =
        sqlx::query_as(r#"SELECT id, notice FROM isahl.zc_id_operation WHERE id = $1"#)
            .bind(rule_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(row.0, rule_id, "delegation rule id must match");
    assert_eq!(row.1, rule_name, "delegation rule notice must match");
}

#[tokio::test]
async fn delegation_rule_dates_roundtrip() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let rule_name = test_code("e2e-delegation-dates");
    let date_st = "2026-08-01T00:00:00Z";
    let date_ed = "2026-12-31T23:59:59Z";

    // 时间窗经 qk_period → zc_id_segm-date 标量引用承载（不写平铺列）
    let period_id = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_segm-date" (id, date_st, date_ed, notice, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1::timestamptz, $2::timestamptz, $3, 1)
           RETURNING id"#,
    )
    .bind(date_st)
    .bind(date_ed)
    .bind(format!("委托期测试 {}", rule_name))
    .fetch_one(&pool)
    .await
    .unwrap();

    let rule_id = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl.zc_id_operation (notice, qk_period, _t_, created_by_id)
           VALUES ($1, $2, 'delegation-rule', 1) RETURNING id"#,
    )
    .bind(&rule_name)
    .bind(period_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 读取路径：operation.qk_period JOIN segm-date 解析 date_st/date_ed（advance 同构查询）
    let row: (
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        r#"SELECT sd.date_st, sd.date_ed
               FROM isahl.zc_id_operation o
               LEFT JOIN isahl."zc_id_segm-date" sd ON sd.id = o.qk_period AND sd.deleted_at IS NULL
               WHERE o.id = $1"#,
    )
    .bind(rule_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let expect_st: chrono::DateTime<chrono::Utc> = date_st.parse().unwrap();
    let expect_ed: chrono::DateTime<chrono::Utc> = date_ed.parse().unwrap();
    assert_eq!(
        row.0,
        Some(expect_st),
        "qk_period → segm-date.date_st must roundtrip"
    );
    assert_eq!(
        row.1,
        Some(expect_ed),
        "qk_period → segm-date.date_ed must roundtrip"
    );

    // 兼容性：无时间窗委托规则（qk_period NULL，legacy comments JSON 兜底）读回 NULL，不失效
    let legacy_id = insert_delegation_rule(&pool, &test_code("e2e-delegation-legacy")).await;
    let legacy: (
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        r#"SELECT sd.date_st, sd.date_ed
               FROM isahl.zc_id_operation o
               LEFT JOIN isahl."zc_id_segm-date" sd ON sd.id = o.qk_period AND sd.deleted_at IS NULL
               WHERE o.id = $1"#,
    )
    .bind(legacy_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        legacy.0.is_none() && legacy.1.is_none(),
        "legacy rule must stay NULL"
    );
}

#[tokio::test]
async fn approval_action_crud() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    // 准备关联数据
    let event_notice = test_code("e2e-action-event");
    let event_id = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(&event_notice)
    .fetch_one(&pool)
    .await
    .unwrap();

    let eng_notice = test_code("e2e-action-eng");
    let eng_id = insert_engineer(&pool, &eng_notice).await;

    // 审批实例（意见 fk_list → 实例 id，fk_index 契约；实例↔事件经 rr_event 桥）
    let instance_id = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, fk_subject, created_by_id)
           VALUES ($1, $2, 1) RETURNING id"#,
    )
    .bind(&event_notice)
    .bind(eng_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(instance_id)
    .bind(event_id)
    .execute(&pool)
    .await
    .unwrap();

    // 创建审批意见（非 approve/reject 操作 — delegated）
    let action_summary = test_code("e2e-action");
    let action_id = insert_approval_action(&pool, &action_summary, eng_id, instance_id, None).await;

    let row: (i64, String, Option<i64>) = sqlx::query_as(
        r#"SELECT id, notice, fk_list FROM isahl."zc_id_deta-opinion" WHERE id = $1"#,
    )
    .bind(action_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.0, action_id, "action id must match");
    assert_eq!(row.1, action_summary, "action notice must match summary");
    assert_eq!(
        row.2,
        Some(instance_id),
        "action fk_list must link to approval instance"
    );
}

#[tokio::test]
async fn list_all_flows_count() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let count_before = count_flows(&pool).await;

    let f1 = test_code("e2e-list-f1");
    let f2 = test_code("e2e-list-f2");
    insert_flow(&pool, &f1).await;
    insert_flow(&pool, &f2).await;

    let count_after = count_flows(&pool).await;
    assert_eq!(
        count_after,
        count_before + 2,
        "flow count must increase by 2"
    );
}

#[tokio::test]
async fn soft_delete_flow_excludes_from_count() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let flow_name = test_code("e2e-softdel");
    let flow_id = insert_flow(&pool, &flow_name).await;

    let count_before = count_flows(&pool).await;

    // 软删除
    sqlx::query(r#"UPDATE isahl.zc_id_process SET deleted_at = NOW() WHERE id = $1"#)
        .bind(flow_id)
        .execute(&pool)
        .await
        .unwrap();

    // 确认 deleted_at 已设置
    let deleted_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar(r#"SELECT deleted_at FROM isahl.zc_id_process WHERE id = $1"#)
            .bind(flow_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        deleted_at.is_some(),
        "deleted_at must be set after soft delete"
    );

    // 确认 count 不再包含该记录
    let count_after = count_flows(&pool).await;
    assert_eq!(
        count_after,
        count_before - 1,
        "soft-deleted flow must be excluded from count"
    );
}

#[tokio::test]
async fn timeline_query_returns_ordered_nodes() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    // 1. 创建审批事件（作为 flow node / place）
    let place_notice = test_code("e2e-tl-place");
    let place_id = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(&place_notice)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 2. 创建工程师（审批人）
    let eng_notice = test_code("e2e-tl-eng");
    let eng_id = insert_engineer(&pool, &eng_notice).await;

    // 3. 创建审批实例
    let inst_notice = test_code("e2e-tl-inst");
    let _instance_id = insert_approval_instance(&pool, &inst_notice, place_id, eng_id).await;

    // 4. 创建两条审批意见（模拟时间线节点）
    insert_approval_action(&pool, &test_code("e2e-tl-act1"), eng_id, place_id, None).await;
    insert_approval_action(&pool, &test_code("e2e-tl-act2"), eng_id, place_id, None).await;

    // 5. 执行 timeline 查询（复用 handler 中的查询模式）
    let rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"SELECT a.notice, a.opinion AS opinion
           FROM isahl."zc_id_deta-opinion" a
           WHERE a.fk_list = $1 AND a.deleted_at IS NULL
           ORDER BY a.created_at ASC, a.id ASC"#,
    )
    .bind(place_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(rows.len(), 2, "timeline must return 2 action records");
}

#[tokio::test]
async fn approval_instance_comments_roundtrip() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    // 创建测试所需的 fk_approve（审批事件）和 fk_subject（员工）
    let approve_event_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind("test-event")
    .fetch_one(&pool)
    .await
    .unwrap();

    let eng_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_subj-employee" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind("test-engineer")
    .fetch_one(&pool)
    .await
    .unwrap();

    let payload = r#"{"amount":"50000"}"#;

    // 验证 INSERT 可写入 comments
    let instance_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, code, fk_subject, comments, created_by_id)
           VALUES ($1, $2, $3, $4, 1) RETURNING id"#,
    )
    .bind("test-instance")
    .bind("AF-TEST-001")
    .bind(eng_id)
    .bind(payload)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(instance_id)
    .bind(approve_event_id)
    .execute(&pool)
    .await
    .unwrap();

    // 验证 SELECT 可读取 comments
    let row: (Option<String>,) =
        sqlx::query_as(r#"SELECT comments FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
            .bind(instance_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row.0.as_deref(), Some(payload));

    // 验证 UPDATE 可覆盖 comments
    let payload2 = r#"{"v":2}"#;
    sqlx::query(r#"UPDATE isahl."zc_id_oper-approve" SET comments = $1 WHERE id = $2"#)
        .bind(payload2)
        .bind(instance_id)
        .execute(&pool)
        .await
        .unwrap();

    let row2: (Option<String>,) =
        sqlx::query_as(r#"SELECT comments FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
            .bind(instance_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row2.0.as_deref(), Some(payload2));
}

#[tokio::test]
async fn test_advance_flow() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    // 1. 创建流程
    let flow_id = insert_flow(&pool, "测试流程").await;

    // 2. 创建两个节点
    let n1 = insert_flow_node(&pool, "审批节点A", flow_id).await;
    let n2 = insert_flow_node(&pool, "审批节点B", flow_id).await;

    // 3. 创建 rr_operation 连接 + next-ops
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_process_rr_operation
       (id, code, ref_left, ref_right, comments, "next-ops", created_by_id)
       VALUES (isahl.gen_next_uid(791), 'oper-approve', $1, $2, '节点A',
               jsonb_build_array($3::bigint), 1)"#,
    )
    .bind(flow_id)
    .bind(n1)
    .bind(n2)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"INSERT INTO isahl.zc_id_process_rr_operation
       (id, code, ref_left, ref_right, comments, created_by_id)
       VALUES (isahl.gen_next_uid(791), 'oper-approve', $1, $2, '节点B', 1)"#,
    )
    .bind(flow_id)
    .bind(n2)
    .execute(&pool)
    .await
    .unwrap();

    // 4. 创建审批实例（实例挂节点事件模板：n1=operation → 模板桥反查）
    let n1_template: i64 = sqlx::query_scalar(
        r#"SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
           WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(n1)
    .fetch_one(&pool)
    .await
    .unwrap();
    let instance_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (id, notice, fk_subject, comments, created_by_id)
           VALUES (isahl.gen_next_zuid(), '实例A', 1, $1, 1) RETURNING id"#,
    )
    .bind(r#"{"entityType":"contract","contractId":42}"#)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(instance_id)
    .bind(n1_template)
    .execute(&pool)
    .await
    .unwrap();

    // 5. 调用 advance_flow
    approval::advance::advance_flow(&pool, instance_id, 1, None)
        .await
        .expect("advance_flow failed");

    // 6. 验证下一节点被创建（n2=operation → 反查模板）
    let n2_template: i64 = sqlx::query_scalar(
        r#"SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
           WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(n2)
    .fetch_one(&pool)
    .await
    .unwrap();
    let count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = oa.id AND oe.ref_right = $1
           WHERE oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL"#,
    )
    .bind(n2_template)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        count, 1,
        "advance_flow should create next approval instance"
    );

    // 7. 实体上下文透传：下一实例 comments 必须携带源实例的 contractId
    //    （wire-contract-approval-engine：断链则 ApprovalCompleted 无法回写业务状态）
    let next_comments: Option<String> = sqlx::query_scalar(
        r#"SELECT oa.comments FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = oa.id AND oe.ref_right = $1
           WHERE oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL LIMIT 1"#,
    )
    .bind(n2_template)
    .fetch_one(&pool)
    .await
    .unwrap();
    let cv: serde_json::Value =
        serde_json::from_str(next_comments.as_deref().unwrap_or("{}")).unwrap();
    assert_eq!(cv["contractId"], 42, "advance 必须透传实体上下文 comments");
    assert_eq!(cv["entityType"], "contract");
}

/// fix-approval-flow-chain-breaks F1：设计器词汇 `approval`（publish 物化值）
/// 必须与 `approve`/`oper-approve` 同等地创建审批实例——回归防护。
#[tokio::test]
async fn advance_flow_with_designer_approval_vocabulary() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let flow_id = insert_flow(&pool, "设计器词汇测试流程").await;
    let n1 = insert_flow_node(&pool, "审批节点A", flow_id).await;
    let n2 = insert_flow_node(&pool, "审批节点B", flow_id).await;

    // 边词汇 = 设计器 publish 物化的 "approval"（而非 "approve"）
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_process_rr_operation
           (id, code, ref_left, ref_right, comments, "next-ops", created_by_id)
           VALUES (isahl.gen_next_uid(791), 'approval', $1, $2, '节点A',
                   jsonb_build_array($3::bigint), 1)"#,
    )
    .bind(flow_id)
    .bind(n1)
    .bind(n2)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_process_rr_operation
           (id, code, ref_left, ref_right, comments, created_by_id)
           VALUES (isahl.gen_next_uid(791), 'approval', $1, $2, '节点B', 1)"#,
    )
    .bind(flow_id)
    .bind(n2)
    .execute(&pool)
    .await
    .unwrap();

    let n1_template: i64 = sqlx::query_scalar(
        r#"SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
           WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(n1)
    .fetch_one(&pool)
    .await
    .unwrap();
    let instance_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (id, notice, fk_subject, created_by_id)
           VALUES (isahl.gen_next_zuid(), '实例A', 1, 1) RETURNING id"#,
    )
    .bind(n1_template)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(instance_id)
    .bind(n1_template)
    .execute(&pool)
    .await
    .unwrap();

    approval::advance::advance_flow(&pool, instance_id, 1, None)
        .await
        .expect("advance_flow with 'approval' vocabulary failed");

    let n2_template: i64 = sqlx::query_scalar(
        r#"SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
           WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(n2)
    .fetch_one(&pool)
    .await
    .unwrap();
    let count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = oa.id AND oe.ref_right = $1
           WHERE oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL"#,
    )
    .bind(n2_template)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        count, 1,
        "'approval' 词汇节点必须创建下一审批实例（F1 回归防护）"
    );
}
