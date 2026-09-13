//! `find_item` 按 ID 精确取数集成测试（回归网）。
//!
//! 缺陷背景：`find_item` 曾以 `list_items(limit = 1)` + Rust 侧 `find` 实现——
//! 取数被限制在 `ORDER BY ds.date_st, ds.time_st, p.sort` 的**首行**，非首行计划
//! 恒返回 `None`；Gateway `GET /schedule/items/{id}`（`Gateway/backend/src/schedule/handlers.rs`）
//! 与 `POST /schedule/items/{id}/event` 前置校验因此把命中计划误判为 404/「Plan not found」。
//!
//! 契约：存在 ≥2 个计划时，对**非首行**计划调用 `find_item` MUST 命中，且返回 DTO
//! 与列表读（`list_items`）对同一计划的投影逐字段一致（单条读与列表读共用同一聚合 SELECT）。
//!
//! 依赖：test 库存在 isahl.zc_id_plan / zc_id_plan-personal / zc_id_segm-date。

use framework_schedule::models::{CreatePlanRequest, ScheduleListQuery};
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

async fn cleanup_plan(pool: &PgPool, plan_id: i64) {
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

fn create_req(code: &str, notice: &str, date_start: &str) -> CreatePlanRequest {
    CreatePlanRequest {
        notice: Some(notice.to_string()),
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
        reminder_offset_min: None,
    }
}

/// 同一 code 的计划列表（排序与 `find_item` 所属读路径一致）
fn batch_query(code: &str) -> ScheduleListQuery {
    ScheduleListQuery {
        qk_date_segm: None,
        start_date_segm: None,
        end_date_segm: None,
        _t_: Some(code.to_string()),
        done: None,
        limit: 10,
        offset: 0,
    }
}

#[tokio::test]
async fn find_item_hits_non_first_row_plan() {
    let pool = test_pool().await;
    let svc = ScheduleService::new(ScheduleRepository::new(pool.clone()));

    let code = test_code("t-finditem");
    // 日期递增 → 排序首行是 A；B / C 均为「非首行」
    let a = svc
        .create_plan(create_req(&code, "find-item-A", "2099-01-01"))
        .await
        .expect("create A");
    let b = svc
        .create_plan(create_req(&code, "find-item-B", "2099-06-01"))
        .await
        .expect("create B");
    let c = svc
        .create_plan(create_req(&code, "find-item-C", "2099-12-01"))
        .await
        .expect("create C");

    // 前置事实：B/C 确实不是列表首行（旧实现只取首行 → 必 None）
    let listed = svc
        .list_items(&batch_query(&code), None)
        .await
        .expect("list batch");
    assert_eq!(listed.len(), 3, "batch 应有 3 个计划");
    assert_eq!(listed[0].id, a.id, "排序首行应为 A");
    assert_eq!(listed[1].id, b.id, "B 是非首行");

    // ① 非首行计划必须命中（修复前恒 None）
    let found_b = svc.find_item(b.id).await.expect("find_item B");
    let found_b = found_b.expect("非首行计划 B MUST 命中（旧实现返回 None）");
    assert_eq!(found_b.id, b.id);
    assert_eq!(found_b.title, "find-item-B");
    assert_eq!(found_b.span.date_start.as_deref(), Some("2099-06-01"));

    // ② 尾行同样命中
    let found_c = svc
        .find_item(c.id)
        .await
        .expect("find_item C")
        .expect("非首行计划 C MUST 命中");
    assert_eq!(found_c.id, c.id);
    assert_eq!(found_c.span.date_start.as_deref(), Some("2099-12-01"));

    // ③ 单条读与列表读同一计划逐字段一致（共用同一聚合 SELECT）
    let listed_b = listed.iter().find(|i| i.id == b.id).expect("B 在列表读中");
    assert_eq!(found_b.title, listed_b.title);
    assert_eq!(found_b.item_type, listed_b.item_type);
    assert_eq!(found_b.done, listed_b.done);
    assert_eq!(found_b.span.date_start, listed_b.span.date_start);
    assert_eq!(found_b.span.time_start, listed_b.span.time_start);
    assert_eq!(found_b.participants.len(), listed_b.participants.len());

    // ④ 首行同样命中（新旧路径都成立的对照组）
    assert_eq!(
        svc.find_item(a.id)
            .await
            .expect("find_item A")
            .map(|i| i.id),
        Some(a.id)
    );

    for id in [a.id, b.id, c.id] {
        cleanup_plan(&pool, id).await;
    }
}

#[tokio::test]
async fn find_item_returns_none_for_missing_and_soft_deleted_plan() {
    let pool = test_pool().await;
    let svc = ScheduleService::new(ScheduleRepository::new(pool.clone()));

    let code = test_code("t-finditem-gone");
    let plan = svc
        .create_plan(create_req(&code, "find-item-gone", "2099-03-01"))
        .await
        .expect("create plan");

    assert!(
        svc.find_item(i64::MAX)
            .await
            .expect("find missing")
            .is_none(),
        "不存在的 id MUST 返回 None"
    );

    assert!(svc.delete_plan(plan.id).await.expect("soft delete"));
    assert!(
        svc.find_item(plan.id)
            .await
            .expect("find deleted")
            .is_none(),
        "软删除计划 MUST 返回 None"
    );

    cleanup_plan(&pool, plan.id).await;
}
