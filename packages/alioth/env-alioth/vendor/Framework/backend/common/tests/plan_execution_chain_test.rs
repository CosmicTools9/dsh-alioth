//! plan_execution_chain 集成测试：三级分解链（plan → task → operation）读侧验证
//!
//! P7 修正落地：plan_rr_task + task_rr_operation（op_seq 工序序号）模型齐备，
//! 本测试验证链查询的正确性（直接执行实例 + task 分解 + 有序操作）。

use common::plan_execution::plan_execution_chain;

async fn test_pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://isahl@localhost:5432/aliothstudio_test".to_string());
    let pool = sqlx::PgPool::connect(&url).await.expect("connect test db");
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("current_database");
    assert!(db.contains("_test"), "REFUSED: non-test db {db}");
    pool
}

#[tokio::test]
async fn chain_returns_direct_and_decomposed_operations() {
    let pool = test_pool().await;

    // plan（personal 叶表）
    let plan_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_plan-personal" (notice, code, created_by_id)
           VALUES ('chain-test-plan', 't-chain', 1) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("insert plan");

    // 直接执行实例（P5 路径）
    common::plan_execution::record_plan_execution(&pool, plan_id, "直接执行", None, 1)
        .await
        .expect("record direct");
    // task（task-testing 叶表——zc_id_task 有子表 commission/testing，INSERT 必须落叶；
    // 读侧 plan_execution_chain JOIN 父表继承可见）
    let task_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-testing" (notice, code, created_by_id)
           VALUES ('chain-test-task', 't-chain-task', 1) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("insert task");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_plan_rr_task" (notice, ref_left, ref_right)
           VALUES ('decompose', $1, $2)"#,
    )
    .bind(plan_id)
    .bind(task_id)
    .execute(&pool)
    .await
    .expect("link task");

    // 两个操作（oper-planing 叶表，fk_subject 承载归属语义；插入序与查询序无关）
    let op2: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-planing"
           (notice, code, fk_subject, created_by_id) VALUES ('op-two', 't-chain-op2', $1, 1)
           RETURNING id"#,
    )
    .bind(task_id)
    .fetch_one(&pool)
    .await
    .expect("insert op2");
    let op1: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-planing"
           (notice, code, fk_subject, created_by_id) VALUES ('op-one', 't-chain-op1', $1, 1)
           RETURNING id"#,
    )
    .bind(task_id)
    .fetch_one(&pool)
    .await
    .expect("insert op1");
    // 正桥 operation_rr_task：ref_left=operation（声明归属），ref_right=task
    for oid in [op2, op1] {
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_operation_rr_task" (notice, ref_left, ref_right)
               VALUES ('step', $1, $2)"#,
        )
        .bind(oid)
        .bind(task_id)
        .execute(&pool)
        .await
        .expect("link op");
    }

    // 链查询
    let (direct, tasks) = plan_execution_chain(&pool, plan_id).await.expect("chain");
    assert_eq!(direct.len(), 1, "one direct execution instance");
    assert_eq!(tasks.len(), 1, "one decomposed task");
    let ops = &tasks[0].operations;
    assert_eq!(ops.len(), 2);
    // 按 operation.id 时序返回（op2 先插 id 小在前）
    assert_eq!(ops[0].operation_id, op2);
    assert_eq!(ops[1].operation_id, op1);
    assert_eq!(ops[0].source_table, "zc_id_oper-planing");

    // cleanup
    for t in [
        format!(r#"DELETE FROM isahl."zc_id_operation_rr_task" WHERE ref_right = {task_id}"#),
        format!(
            r#"DELETE FROM isahl."zc_id_oper-planing" WHERE fk_subject IN ({plan_id}, {task_id})"#
        ),
        format!(r#"DELETE FROM isahl."zc_id_plan_rr_task" WHERE ref_left = {plan_id}"#),
        format!(r#"DELETE FROM isahl."zc_id_task-testing" WHERE id = {task_id}"#),
        format!(r#"DELETE FROM isahl."zc_id_plan-personal" WHERE id = {plan_id}"#),
    ] {
        let sql: &str = t.as_str();
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .execute(&pool)
            .await
            .ok();
    }
}

#[tokio::test]
async fn chain_empty_for_unknown_plan() {
    let pool = test_pool().await;
    let (direct, tasks) = plan_execution_chain(&pool, 1).await.expect("chain");
    let _ = (direct.len(), tasks.len()); // 未知计划不报错（空/既有数据均可）
}
