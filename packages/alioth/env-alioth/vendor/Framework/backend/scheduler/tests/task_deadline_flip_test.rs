//! 任务到期引擎 + 切片翻转集成测试（时间对称模型双引擎验证）

use framework_scheduler::task_deadline::TaskDeadlineHandler;

async fn test_pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://isahl@localhost:5432/aliothstudio_test".to_string());
    let pool = sqlx::PgPool::connect(&url).await.expect("connect test db");
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("db name");
    assert!(db.contains("_test"), "REFUSED: {db}");
    pool
}

/// 任务到期：操作痕迹 + operation_rr_task 归属 + 站内信（幂等）
#[tokio::test]
async fn task_deadline_notifies_with_operation_trace() {
    let pool = test_pool().await;
    let handler = TaskDeadlineHandler::new(pool.clone());

    // 构造到期任务：qk_period → segm-date.date_ed 已过
    let segm_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_segm-date" (notice, date_ed, created_at)
           VALUES ('due-test', NOW() - INTERVAL '1 hour', NOW()) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("segm");

    let task_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-testing" (notice, code, qk_period, created_by_id)
           VALUES ('due-task', 't-due-1', $1, 1) RETURNING id"#,
    )
    .bind(segm_id)
    .fetch_one(&pool)
    .await
    .expect("task");

    let (total, notified) = handler.check_and_notify().await.expect("check");
    assert!(total >= 1, "due task detected");
    assert!(notified >= 1, "notification sent");

    // 操作痕迹存在且经正桥归属任务
    let op_cnt: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_operation_rr_task" rr
           JOIN isahl."zc_id_oper-planing" o ON o.id = rr.ref_left
           WHERE rr.ref_right = $1"#,
    )
    .bind(task_id)
    .fetch_one(&pool)
    .await
    .expect("op count");
    assert!(op_cnt >= 1, "operation trace linked via operation_rr_task");

    // 站内信 marker 存在
    let msg_cnt: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_msgs-system"
           WHERE comments LIKE '%task-deadline:' || $1::text || '%' AND deleted_at IS NULL"#,
    )
    .bind(task_id)
    .fetch_one(&pool)
    .await
    .expect("msg count");
    assert!(msg_cnt >= 1, "notification message with marker");

    // 幂等：二次调用不重复
    let (_, notified2) = handler.check_and_notify().await.expect("check 2");
    assert_eq!(notified2, 0, "idempotent re-run");

    // cleanup
    for sql in [
        format!(r#"DELETE FROM isahl."zc_id_operation_rr_task" WHERE ref_right = {task_id}"#),
        format!(
            r#"DELETE FROM isahl."zc_id_oper-planing" WHERE fk_subject = {task_id} AND code LIKE 'task-due-%'"#
        ),
        format!(
            r#"DELETE FROM isahl."zc_id_msgs-system" WHERE comments LIKE '%task-deadline:{task_id}%'"#
        ),
        format!(r#"DELETE FROM isahl."zc_id_task-testing" WHERE id = {task_id}"#),
        format!(r#"DELETE FROM isahl."zc_id_segm-date" WHERE id = {segm_id}"#),
    ] {
        let s = sql.as_str();
        sqlx::query(sqlx::AssertSqlSafe(s))
            .execute(&pool)
            .await
            .ok();
    }
}

/// 切片翻转：完成落 even-alert + plan_rr_event + operation 归属
#[tokio::test]
async fn slice_flip_records_event_and_links() {
    let pool = test_pool().await;

    let plan_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_plan-personal" (notice, code, created_by_id)
           VALUES ('flip-test', 't-flip', 1) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("plan");

    let task_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_task-testing" (notice, code, created_by_id)
           VALUES ('flip-task', 't-flip-task', 1) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("task");

    let (event_id, op_link) =
        common::plan_execution::record_slice_flip(&pool, plan_id, Some(task_id), "翻转测试", 1)
            .await
            .expect("flip");

    // 事件切片落 even-alert 叶表
    let ev_cnt: i64 =
        sqlx::query_scalar(r#"SELECT COUNT(*) FROM isahl."zc_id_even-alert" WHERE id = $1"#)
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .expect("ev");
    assert_eq!(ev_cnt, 1, "event slice on even-alert leaf");

    // plan_rr_event 关联
    let link: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_plan_rr_event"
           WHERE ref_left = $1 AND ref_right = $2"#,
    )
    .bind(plan_id)
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("link");
    assert_eq!(link, 1, "plan-event association");

    // operation 经正桥归属任务
    let op_id = op_link.expect("op link");
    let op_link_cnt: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_operation_rr_task"
           WHERE ref_left = $1 AND ref_right = $2"#,
    )
    .bind(op_id)
    .bind(task_id)
    .fetch_one(&pool)
    .await
    .expect("op link cnt");
    assert_eq!(op_link_cnt, 1, "operation attributed to task via rr_task");

    // cleanup
    for sql in [
        format!(r#"DELETE FROM isahl."zc_id_operation_rr_task" WHERE ref_right = {task_id}"#),
        format!(r#"DELETE FROM isahl."zc_id_oper-planing" WHERE id = {op_id}"#),
        format!(r#"DELETE FROM isahl."zc_id_plan_rr_event" WHERE ref_left = {plan_id}"#),
        format!(r#"DELETE FROM isahl."zc_id_even-alert" WHERE id = {event_id}"#),
        format!(r#"DELETE FROM isahl."zc_id_task-testing" WHERE id = {task_id}"#),
    ] {
        let s = sql.as_str();
        sqlx::query(sqlx::AssertSqlSafe(s))
            .execute(&pool)
            .await
            .ok();
    }
}
