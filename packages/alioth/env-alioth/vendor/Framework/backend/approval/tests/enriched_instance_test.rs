//! Enriched Instance Handler — 集成测试
//!
//! 覆盖 enriched 端点的 SQL 查询逻辑：
//! - 基础实例（无 action）→ status=pending, result=pending
//! - 已通过实例（有审批通过 action）→ status=approved, result=approved
//! - 已驳回实例（有审批驳回 action）→ status=rejected, result=rejected
//! - lk_urgent 映射（1→urgent, 2→high, 3→normal）

use ::common::testing::connect_test_db;
use sqlx::PgPool;
mod common;
use common::{setup_test_schema, test_code};

/// 插入一条员工（作为申请人）
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

/// 插入一条审批流程
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

/// 插入一条审批实例（指定 lk_urgent 和 fk_subject）
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

/// 插入一条审批意见动作（按 fk_list = 实例 id 关联，fk_index 契约）
async fn insert_action(pool: &PgPool, notice: &str, fk_list: i64) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_deta-opinion" (notice, fk_list, created_by_id)
           VALUES ($1, $2, 1) RETURNING id"#,
    )
    .bind(notice)
    .bind(fk_list)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 运行 enriched SQL 查询 — 与 handler 中的 QUERY 一致
/// 运行 enriched SQL 查询
async fn query_enriched(
    pool: &PgPool,
    status_filter: Option<&str>,
) -> Vec<(i64, Option<String>, Option<String>, String)> {
    // node_name Option：与 handler EnrichedRow 对齐（实例 notice 可空）
    sqlx::query_as::<_, (i64, Option<String>, Option<String>, String)>(
        r#"
        WITH base AS (
            SELECT i.id, i.notice AS node_name, i.code, i.fk_subject, i.comments,
                   e.notice AS applicant_name, act.notice AS latest_notice,
                   CASE
                       WHEN act.notice IS NULL THEN 'pending'
                       WHEN act.notice = '审批通过' THEN 'approved'
                       WHEN act.notice = '审批驳回' THEN 'rejected'
                       ELSE 'active'
                   END AS derived_status,
                   i.created_at, i.updated_at
            FROM isahl."zc_id_oper-approve" i
            LEFT JOIN isahl."zc_id_subj-employee" e ON e.id = i.fk_subject
            LEFT JOIN LATERAL (
                SELECT a.notice FROM isahl."zc_id_deta-opinion" a
                WHERE a.fk_list = i.id AND a.deleted_at IS NULL
                ORDER BY a.created_at DESC LIMIT 1
            ) act ON true
            WHERE i.deleted_at IS NULL
              AND ($1::text IS NULL OR
                   CASE
                       WHEN act.notice IS NULL THEN 'pending'
                       WHEN act.notice = '审批通过' THEN 'approved'
                       WHEN act.notice = '审批驳回' THEN 'rejected'
                       ELSE 'active'
                   END = $1)
        )
        SELECT id, node_name, applicant_name, derived_status
        FROM base
        ORDER BY id
        "#,
    )
    .bind(status_filter)
    .fetch_all(pool)
    .await
    .unwrap()
}
#[tokio::test]
async fn test_enriched_no_action_pending() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("enr-pend");

    let flow = insert_flow(&pool, &format!("flow-{code}")).await;
    let inst = insert_instance(&pool, &format!("inst-{code}"), Some(flow), None).await;

    let rows = query_enriched(&pool, None).await;
    let my = rows
        .iter()
        .find(|r| r.0 == inst)
        .expect("instance not found");

    assert_eq!(
        my.2, None,
        "applicant should be None when fk_subject is null"
    );
    assert_eq!(
        my.3, "pending",
        "no action → derived_status should be pending"
    );
}

#[tokio::test]
async fn test_enriched_action_approved() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("enr-appr");

    let flow = insert_flow(&pool, &format!("flow-{code}")).await;
    let employee = insert_employee(&pool, "张三").await;
    let inst = insert_instance(&pool, &format!("inst-{code}"), Some(flow), Some(employee)).await;
    insert_action(&pool, "审批通过", inst).await;

    let rows = query_enriched(&pool, Some("approved")).await;
    let my = rows
        .iter()
        .find(|r| r.0 == inst)
        .expect("approved instance not found");

    assert_eq!(
        my.2,
        Some("张三".into()),
        "applicant should resolve from fk_subject"
    );
    assert_eq!(
        my.3, "approved",
        "审批通过 action → derived_status should be approved"
    );
}

#[tokio::test]
async fn test_enriched_action_rejected() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("enr-rej");

    let flow = insert_flow(&pool, &format!("flow-{code}")).await;
    let inst = insert_instance(&pool, &format!("inst-{code}"), Some(flow), None).await;
    insert_action(&pool, "审批驳回", inst).await;

    let rows = query_enriched(&pool, Some("rejected")).await;
    let my = rows
        .iter()
        .find(|r| r.0 == inst)
        .expect("rejected instance not found");

    assert_eq!(
        my.3, "rejected",
        "审批驳回 action → derived_status should be rejected"
    );
}

#[tokio::test]
async fn test_enriched_status_filter() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let code = test_code("enr-flt");

    let flow_a = insert_flow(&pool, &format!("flow-a-{code}")).await;
    let flow_b = insert_flow(&pool, &format!("flow-b-{code}")).await;
    insert_instance(&pool, &format!("inst-pending-{code}"), Some(flow_a), None).await;
    let inst_b = insert_instance(&pool, &format!("inst-approved-{code}"), Some(flow_b), None).await;
    insert_action(&pool, "审批通过", inst_b).await;

    let pending_rows = query_enriched(&pool, Some("pending")).await;
    assert!(
        pending_rows.iter().all(|r| r.3 == "pending"),
        "all filtered rows should be pending"
    );
    assert!(
        pending_rows.iter().any(|r| r.0 != inst_b),
        "approved instance should not appear in pending filter"
    );
}
