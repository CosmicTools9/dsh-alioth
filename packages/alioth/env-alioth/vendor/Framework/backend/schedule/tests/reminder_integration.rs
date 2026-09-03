//! framework-schedule S1 集成测试：reminder 读写 + ScheduleReminderHandler 触发
//!
//! 依赖：test 库存在 isahl.zc_id_plan / zc_id_segm-date / zc_id_message。

use framework_schedule::models::CreatePlanRequest;
use framework_schedule::reminder::ScheduleReminderHandler;
use framework_schedule::service::ScheduleService;
use framework_schedule::ScheduleRepository;
use sqlx::PgPool;

async fn test_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://isahl@localhost:5432/aliothstudio_test".to_string());
    let pool = PgPool::connect(&url).await.expect("connect test db");
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("current_database");
    assert!(db.contains("_test"), "REFUSED: non-test db {db}");
    pool
}

fn test_code(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{}-{}", std::process::id(), nanos % 1_000_000_000)
}

#[tokio::test]
async fn create_plan_persists_reminder_and_reads_back() {
    let pool = test_pool().await;
    let repo = ScheduleRepository::new(pool.clone());
    let svc = ScheduleService::new(repo);

    let code = test_code("t-rem");
    let req = CreatePlanRequest {
        notice: Some("reminder-test".to_string()),
        code: Some(code.clone()),
        qk_date_segm: None,
        qk_time_segm: None,
        cron: None,
        exclude: None,
        sort: None,
        title: None,
        date_start: Some("2099-01-01".to_string()),
        date_end: None,
        time_start: Some("10:00".to_string()),
        time_end: None,
        r#type: None,
        reminder_offset_min: Some(30),
    };
    let plan = svc.create_plan(req).await.expect("create plan");

    // comments 含 reminder
    let comments: Option<String> =
        sqlx::query_scalar(r#"SELECT comments FROM isahl."zc_id_plan" WHERE id = $1"#)
            .bind(plan.id)
            .fetch_one(&pool)
            .await
            .expect("read comments");
    let cj = serde_json::from_str::<serde_json::Value>(&comments.unwrap()).unwrap();
    assert_eq!(cj["reminder_offset_min"], 30);

    // cleanup
    sqlx::query(r#"DELETE FROM isahl."zc_id_plan" WHERE id = $1"#)
        .bind(plan.id)
        .execute(&pool)
        .await
        .expect("cleanup plan");
}

#[tokio::test]
async fn handler_tolerates_plaintext_comments_rows() {
    // 回归：zc_id_plan.comments 被多业务共用（AVIC 等写纯文本备注），
    // 历史上 `comments::jsonb ?` cast 遇非 JSON 行使整个查询失败（类型json的输入语法无效）。
    // 现在 LIKE 粗筛 + Rust 侧 serde 容错，纯文本行必须被静默跳过。
    let pool = test_pool().await;
    let handler = ScheduleReminderHandler::new(pool.clone());

    let code = test_code("t-plain");
    let plan_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_plan-personal"
           (notice, code, comments, created_by_id, created_at)
           VALUES ($1, $2, $3, 1, NOW()) RETURNING id"#,
    )
    .bind("plaintext-comments-test")
    .bind(&code)
    .bind("方案评审→试制（多行\n备注文本）")
    .fetch_one(&pool)
    .await
    .expect("insert plan");

    // 必须不报错（坏行被跳过）
    let _ = handler.check_and_remind().await.expect("check_and_remind");

    // 纯文本行不产生提醒消息
    let cnt: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_msgs-system" m
           WHERE m.comments LIKE '%schedule-reminder:' || $1::text || '%' AND m.deleted_at IS NULL"#,
    )
    .bind(plan_id)
    .fetch_one(&pool)
    .await
    .expect("count msgs");
    assert_eq!(cnt, 0, "plaintext comments must not trigger reminder");

    // cleanup
    sqlx::query(r#"DELETE FROM isahl."zc_id_plan-personal" WHERE id = $1"#)
        .bind(plan_id)
        .execute(&pool)
        .await
        .expect("cleanup plan");
}

#[tokio::test]
async fn reminder_handler_sends_message_for_due_plan() {
    let pool = test_pool().await;
    let handler = ScheduleReminderHandler::new(pool.clone());

    // 造一个到点计划：开始时间 = now + 5 分钟，reminder_offset_min = 15 → 提醒窗口已到
    let code = test_code("t-due");
    let start_at = chrono::Utc::now() + chrono::Duration::minutes(5);
    let segm_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_segm-date" (notice, date_st, created_at)
           VALUES ('rem-test', $1, NOW()) RETURNING id"#,
    )
    .bind(start_at)
    .fetch_one(&pool)
    .await
    .expect("insert segm");

    let plan_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_plan-personal"
           (notice, code, comments, "qk_date-segm", created_by_id, created_at)
           VALUES ($1, $2, $3, $4, 1, NOW()) RETURNING id"#,
    )
    .bind("reminder-due-test")
    .bind(&code)
    .bind(serde_json::json!({"reminder_offset_min": 15}).to_string())
    .bind(segm_id)
    .fetch_one(&pool)
    .await
    .expect("insert plan");

    let sent = handler.check_and_remind().await.expect("check_and_remind");
    assert!(sent >= 1, "due plan should trigger reminder");

    // 幂等：再次调用不重复发送
    let sent2 = handler
        .check_and_remind()
        .await
        .expect("check_and_remind 2");
    assert_eq!(sent2, 0, "idempotent: no duplicate message");

    // cleanup
    sqlx::query(r#"DELETE FROM isahl."zc_id_plan-personal" WHERE id = $1"#)
        .bind(plan_id)
        .execute(&pool)
        .await
        .expect("cleanup plan");
    sqlx::query(r#"DELETE FROM isahl."zc_id_segm-date" WHERE id = $1"#)
        .bind(segm_id)
        .execute(&pool)
        .await
        .expect("cleanup segm");
}

#[tokio::test]
async fn toggle_plan_done_writes_execution_instance() {
    let pool = test_pool().await;
    let code = test_code("t-p5");

    let plan_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_plan-personal"
           (notice, code, created_by_id, created_at)
           VALUES ($1, $2, 1, NOW()) RETURNING id"#,
    )
    .bind("p5-toggle-test")
    .bind(&code)
    .fetch_one(&pool)
    .await
    .expect("insert plan");

    let repo = ScheduleRepository::new(pool.clone());
    // mark done → 执行实例
    repo.toggle_plan_done(plan_id).await.expect("mark done");
    let cnt: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-planing"
           WHERE fk_subject = $1 AND deleted_at IS NULL"#,
    )
    .bind(plan_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert!(cnt >= 1, "mark done should write oper-planing instance");

    // unmark → 第二条执行实例
    repo.toggle_plan_done(plan_id).await.expect("unmark");
    let cnt2: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-planing"
           WHERE fk_subject = $1 AND deleted_at IS NULL"#,
    )
    .bind(plan_id)
    .fetch_one(&pool)
    .await
    .expect("count 2");
    assert!(cnt2 >= 2, "unmark should write second instance");

    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-planing" WHERE fk_subject = $1"#)
        .bind(plan_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_plan-personal" WHERE id = $1"#)
        .bind(plan_id)
        .execute(&pool)
        .await
        .expect("cleanup plan");
}
