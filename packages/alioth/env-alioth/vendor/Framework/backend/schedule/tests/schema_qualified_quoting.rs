//! 回归测试：事件创建写入 `isahl."zc_id_even-alert"`（schema 限定表名引号）
//!
//! 背景：`service::create_event` 的 INSERT 曾写作 `INSERT INTO "isahl.zc_id_even-alert"` ——
//! PostgreSQL 中引号内是**单个**标识符，语句必然报「关系 "isahl.zc_id_even-alert" 不存在」。
//! 本测试走真实 `ScheduleService::create_event` 路径（修复前必失败）。
//!
//! 判据来源：openspec/specs/sql-schema-qualified-naming/spec.md
//! 依赖：test 库存在 `isahl."zc_id_even-alert"` + dk 维度行（JE / FBB / ↓_EE）。

use framework_schedule::models::CreateEventRequest;
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

#[tokio::test]
async fn create_event_inserts_into_even_alert() {
    let pool = test_pool().await;
    let svc = ScheduleService::new(ScheduleRepository::new(pool.clone()));

    let event = svc
        .create_event(CreateEventRequest {
            notice: Some("回归-事件".into()),
            fk_place: None,
            fk_subject: None,
            qk_date: Some(0),
        })
        .await
        .expect("事件创建（修复前：关系不存在）");

    assert_eq!(event.notice.as_deref(), Some("回归-事件"));

    let read_back: Option<String> =
        sqlx::query_scalar(r#"SELECT notice FROM isahl."zc_id_even-alert" WHERE id = $1"#)
            .bind(event.id)
            .fetch_one(&pool)
            .await
            .expect("读回");
    assert_eq!(read_back.as_deref(), Some("回归-事件"), "插入已落库");

    sqlx::query(r#"DELETE FROM isahl."zc_id_even-alert" WHERE id = $1"#)
        .bind(event.id)
        .execute(&pool)
        .await
        .expect("清理样本行");
}
