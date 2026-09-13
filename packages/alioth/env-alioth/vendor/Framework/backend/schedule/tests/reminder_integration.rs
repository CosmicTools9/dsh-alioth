//! framework-schedule 提醒集成测试：预警事件载体读写 + ScheduleReminderHandler 触发
//!
//! 提醒载体现状（零 DDL，全既有结构）：
//!   `zc_id_even-alert`（code='schedule-reminder'，qk_date → zc_id_scal-date）
//!   + `zc_id_plan_rr_event`（ref_left=计划）桥；`comments` 零承载。
//!
//! 依赖：test 库存在 isahl.zc_id_plan / zc_id_plan-personal / zc_id_segm-date /
//! zc_id_even-alert / zc_id_scal-date / zc_id_plan_rr_event / zc_id_msgs-system。

use framework_schedule::models::{CreatePlanRequest, UpdatePlanRequest};
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

/// 读取计划提醒事件（桥 → 叶表 → 标量）；返回 (event_id, scal_date_id)
async fn reminder_event(pool: &PgPool, plan_id: i64) -> Option<(i64, Option<i64>)> {
    let event_id: Option<i64> = sqlx::query_scalar::<_, i64>(
        r#"SELECT ref_right FROM isahl.zc_id_plan_rr_event
           WHERE ref_left = $1 AND deleted_at IS NULL
           ORDER BY id DESC LIMIT 1"#,
    )
    .bind(plan_id)
    .fetch_optional(pool)
    .await
    .expect("bridge row");
    let event_id = event_id?;
    let scal_id: Option<i64> = sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT qk_date FROM isahl."zc_id_even-alert" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .expect("alert row");
    Some((event_id, scal_id))
}

/// 清理提醒事件（桥 + 叶表 + 标量行）
async fn cleanup_reminder(pool: &PgPool, plan_id: i64) {
    let rows: Vec<(i64, i64, Option<i64>)> = sqlx::query_as(
        r#"SELECT rpe.id, re.id, re.qk_date FROM isahl.zc_id_plan_rr_event rpe
           JOIN isahl."zc_id_even-alert" re ON re.id = rpe.ref_right
           WHERE rpe.ref_left = $1"#,
    )
    .bind(plan_id)
    .fetch_all(pool)
    .await
    .expect("reminder rows");
    for (bridge_id, event_id, scal_id) in rows {
        sqlx::query(r#"DELETE FROM isahl.zc_id_plan_rr_event WHERE id = $1"#)
            .bind(bridge_id)
            .execute(pool)
            .await
            .ok();
        sqlx::query(r#"DELETE FROM isahl."zc_id_even-alert" WHERE id = $1"#)
            .bind(event_id)
            .execute(pool)
            .await
            .ok();
        if let Some(sid) = scal_id {
            sqlx::query(r#"DELETE FROM isahl."zc_id_scal-date" WHERE id = $1"#)
                .bind(sid)
                .execute(pool)
                .await
                .ok();
        }
    }
}

async fn cleanup_plan(pool: &PgPool, plan_id: i64) {
    cleanup_reminder(pool, plan_id).await;
    let segm: Option<Option<i64>> =
        sqlx::query_scalar(r#"SELECT "qk_date-segm" FROM isahl."zc_id_plan" WHERE id = $1"#)
            .bind(plan_id)
            .fetch_optional(pool)
            .await
            .expect("segm id");
    sqlx::query(r#"DELETE FROM isahl."zc_id_plan-personal" WHERE id = $1"#)
        .bind(plan_id)
        .execute(pool)
        .await
        .expect("cleanup plan");
    if let Some(Some(segm_id)) = segm {
        sqlx::query(r#"DELETE FROM isahl."zc_id_segm-date" WHERE id = $1"#)
            .bind(segm_id)
            .execute(pool)
            .await
            .ok();
    }
}

fn list_query(code: String) -> framework_schedule::models::ScheduleListQuery {
    framework_schedule::models::ScheduleListQuery {
        qk_date_segm: None,
        start_date_segm: None,
        end_date_segm: None,
        _t_: Some(code),
        done: None,
        limit: 20,
        offset: 0,
    }
}

fn create_req(code: &str, date_start: &str, reminder: Option<i32>) -> CreatePlanRequest {
    CreatePlanRequest {
        notice: Some("reminder-test".to_string()),
        code: Some(code.to_string()),
        qk_date_segm: None,
        qk_time_segm: None,
        cron: None,
        exclude: None,
        sort: None,
        title: None,
        date_start: Some(date_start.to_string()),
        date_end: None,
        time_start: Some("10:00".to_string()),
        time_end: None,
        r#type: None,
        reminder_offset_min: reminder,
    }
}

#[tokio::test]
async fn create_plan_persists_reminder_as_alert_event_and_reads_back() {
    let pool = test_pool().await;
    let repo = ScheduleRepository::new(pool.clone());
    let svc = ScheduleService::new(repo);

    let code = test_code("t-rem");
    let plan = svc
        .create_plan(create_req(&code, "2099-01-01", Some(30)))
        .await
        .expect("create plan");

    // ① comments 零承载（不再写 JSON）
    let comments: Option<String> =
        sqlx::query_scalar(r#"SELECT comments FROM isahl."zc_id_plan" WHERE id = $1"#)
            .bind(plan.id)
            .fetch_one(&pool)
            .await
            .expect("read comments");
    assert!(
        comments
            .as_deref()
            .unwrap_or("")
            .find("reminder_offset_min")
            .is_none(),
        "comments must not carry reminder JSON: {comments:?}"
    );

    // ② 提醒事件落既有结构：zc_id_even-alert(code=schedule-reminder) + 桥 + zc_id_scal-date
    let (event_id, scal_id) = reminder_event(&pool, plan.id)
        .await
        .expect("reminder event");
    assert!(
        scal_id.is_some(),
        "alert event must reference a scal-date row"
    );
    let (start_at, remind_at): (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) =
        sqlx::query_as(
            r#"SELECT ds.date_st, sd.date
               FROM isahl."zc_id_plan" p
               JOIN isahl."zc_id_segm-date" ds ON ds.id = p."qk_date-segm"
               JOIN isahl."zc_id_scal-date" sd ON sd.id = $2
               WHERE p.id = $1"#,
        )
        .bind(plan.id)
        .bind(scal_id)
        .fetch_one(&pool)
        .await
        .expect("start/remind times");
    assert_eq!(
        (start_at - remind_at).num_minutes(),
        30,
        "event time must equal plan start minus offset"
    );
    let event_code: String =
        sqlx::query_scalar(r#"SELECT code FROM isahl."zc_id_even-alert" WHERE id = $1"#)
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .expect("event code");
    assert_eq!(event_code, "schedule-reminder");

    // ③ DTO 读回 offset 一致（create 响应 + 列表/详情路径同源）
    assert_eq!(plan.reminder.as_ref().map(|r| r.offset), Some(30));
    // 列表读侧（into_item_response）：按 code 过滤，避免 find_item 的 limit=1 取首行怪癖
    let items = svc
        .list_items(&list_query(code.clone()), None)
        .await
        .expect("list items");
    let item = items
        .iter()
        .find(|i| i.id == plan.id)
        .expect("item in list");
    assert_eq!(item.reminder.as_ref().map(|r| r.offset), Some(30));

    cleanup_plan(&pool, plan.id).await;
}

#[tokio::test]
async fn clearing_reminder_soft_deletes_event_and_keeps_comments() {
    let pool = test_pool().await;
    let repo = ScheduleRepository::new(pool.clone());
    let svc = ScheduleService::new(repo);

    let code = test_code("t-clear");
    let plan = svc
        .create_plan(create_req(&code, "2099-01-02", Some(15)))
        .await
        .expect("create plan");
    let (old_event_id, _) = reminder_event(&pool, plan.id)
        .await
        .expect("reminder event");
    // 既有 comments 语义（纯文本备注）不受提醒写入影响
    sqlx::query(r#"UPDATE isahl."zc_id_plan" SET comments = $2 WHERE id = $1"#)
        .bind(plan.id)
        .bind("方案评审→试制")
        .execute(&pool)
        .await
        .expect("set comments");

    // 清除：reminder_offset_min = 0（前端 reminder.none 语义）
    let updated = svc
        .update_plan(
            plan.id,
            UpdatePlanRequest {
                notice: None,
                code: None,
                qk_date_segm: None,
                qk_time_segm: None,
                cron: None,
                exclude: None,
                sort: None,
                reminder_offset_min: Some(0),
            },
        )
        .await
        .expect("update plan")
        .expect("plan exists");
    assert!(
        updated.reminder.is_none(),
        "cleared reminder must read back as None"
    );

    // 事件与桥软删（deleted_at 置位），comments 原样保留
    let bridge_alive: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_plan_rr_event
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(plan.id)
    .fetch_one(&pool)
    .await
    .expect("bridge count");
    assert_eq!(bridge_alive, 0, "bridge row must be soft-deleted on clear");
    let event_deleted: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar(r#"SELECT deleted_at FROM isahl."zc_id_even-alert" WHERE id = $1"#)
            .bind(old_event_id)
            .fetch_one(&pool)
            .await
            .expect("event deleted_at");
    assert!(
        event_deleted.is_some(),
        "alert event must be soft-deleted on clear"
    );
    let comments: Option<String> =
        sqlx::query_scalar(r#"SELECT comments FROM isahl."zc_id_plan" WHERE id = $1"#)
            .bind(plan.id)
            .fetch_one(&pool)
            .await
            .expect("comments");
    assert_eq!(comments.as_deref(), Some("方案评审→试制"));

    // 清理：软删的事件行属本测试产物，连同计划一并物理删除
    let segm: Option<Option<i64>> =
        sqlx::query_scalar(r#"SELECT "qk_date-segm" FROM isahl."zc_id_plan" WHERE id = $1"#)
            .bind(plan.id)
            .fetch_optional(&pool)
            .await
            .expect("segm id");
    cleanup_reminder(&pool, plan.id).await;
    sqlx::query(r#"DELETE FROM isahl."zc_id_even-alert" WHERE id = $1"#)
        .bind(old_event_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_plan-personal" WHERE id = $1"#)
        .bind(plan.id)
        .execute(&pool)
        .await
        .expect("cleanup plan");
    if let Some(Some(segm_id)) = segm {
        sqlx::query(r#"DELETE FROM isahl."zc_id_segm-date" WHERE id = $1"#)
            .bind(segm_id)
            .execute(&pool)
            .await
            .ok();
    }
}

#[tokio::test]
async fn plaintext_comments_without_reminder_event_not_flagged() {
    // 回归：comments 为多业务共用纯文本备注（AVIC 等），不含提醒事件的行不得触发提醒；
    // 提醒扫描不再解析 comments（载体已迁预警事件），纯文本行天然安全。
    let pool = test_pool().await;
    let handler = ScheduleReminderHandler::new(pool.clone());

    let code = test_code("t-plain");
    // 叶表坐标（§6.12 声明即必须）：计划行 dk 经静态绑定解析（与 plan/task 族测试同源 JE/FMA/↓_CH）
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JE", "FMA", "↓_CH"))
            .await
            .expect("resolve plan coords");
    let plan_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_plan-personal"
           (notice, code, comments, created_by_id, created_at, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, 1, NOW(), $4, $5, $6) RETURNING id"#,
    )
    .bind("plaintext-comments-test")
    .bind(&code)
    .bind("方案评审→试制（多行\n备注文本）")
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(&pool)
    .await
    .expect("insert plan");

    let _ = handler.check_and_remind().await.expect("check_and_remind");

    let cnt: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_msgs-system" m
           WHERE m.comments LIKE '%schedule-reminder:' || $1::text || '%' AND m.deleted_at IS NULL"#,
    )
    .bind(plan_id)
    .fetch_one(&pool)
    .await
    .expect("count msgs");
    assert_eq!(cnt, 0, "plaintext comments must not trigger reminder");

    cleanup_plan(&pool, plan_id).await;
}

#[tokio::test]
async fn reminder_handler_sends_message_for_due_plan() {
    let pool = test_pool().await;
    let handler = ScheduleReminderHandler::new(pool.clone());
    let repo = ScheduleRepository::new(pool.clone());
    let svc = ScheduleService::new(repo);

    // 造一个到点计划：开始时间 = now + 5 分钟，提醒 offset = 5 → 提醒时刻 = now（窗口已到）
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

    // 叶表坐标（§6.12 声明即必须）：计划行 dk 经静态绑定解析（与 plan/task 族测试同源 JE/FMA/↓_CH）
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JE", "FMA", "↓_CH"))
            .await
            .expect("resolve plan coords");
    let plan_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_plan-personal"
           (notice, code, "qk_date-segm", created_by_id, created_at, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, 1, NOW(), $4, $5, $6) RETURNING id"#,
    )
    .bind("reminder-due-test")
    .bind(&code)
    .bind(segm_id)
    .fetch_one(&pool)
    .await
    .expect("insert plan");

    // 经服务层写提醒（预警事件载体）
    svc.update_plan(
        plan_id,
        UpdatePlanRequest {
            notice: None,
            code: None,
            qk_date_segm: None,
            qk_time_segm: None,
            cron: None,
            exclude: None,
            sort: None,
            reminder_offset_min: Some(5),
        },
    )
    .await
    .expect("update plan")
    .expect("plan exists");

    let sent = handler.check_and_remind().await.expect("check_and_remind");
    assert!(sent >= 1, "due plan should trigger reminder");

    // 幂等：再次调用不重复发送
    let sent2 = handler
        .check_and_remind()
        .await
        .expect("check_and_remind 2");
    assert_eq!(sent2, 0, "idempotent: no duplicate message");

    // cleanup（站内信 body 含 marker 文本，非 JSON）
    sqlx::query(r#"DELETE FROM isahl."zc_id_msgs-system" WHERE comments LIKE '%schedule-reminder:' || $1::text || '%'"#)
        .bind(plan_id)
        .execute(&pool)
        .await
        .ok();
    cleanup_reminder(&pool, plan_id).await;
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

    // 叶表坐标（§6.12 声明即必须）：计划行 dk 经静态绑定解析（与 plan/task 族测试同源 JE/FMA/↓_CH）
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JE", "FMA", "↓_CH"))
            .await
            .expect("resolve plan coords");
    let plan_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_plan-personal"
           (notice, code, created_by_id, created_at, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, 1, NOW(), $3, $4, $5) RETURNING id"#,
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
    cleanup_plan(&pool, plan_id).await;
}
