//! framework-scheduler 集成测试：cron 到点触发 → oper-planing 写入 → 幂等防重复
//!
//! 依赖：test 库存在 isahl.zc_id_plan（含种子计划）与 isahl.zc_id_oper-planing。

use async_trait::async_trait;
use framework_scheduler::{
    CronSchedule, ScheduledHandler, SchedulerContext, SchedulerError, SchedulerResult,
    SchedulerService,
};
use sqlx::PgPool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// 测试库连接（DATABASE_URL 由 CI/dev 注入；守门：仅 _test 库）
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

/// 假 handler：计数调用
struct CountHandler {
    code: String,
    calls: Arc<AtomicU64>,
}

#[async_trait]
impl ScheduledHandler for CountHandler {
    fn plan_code(&self) -> &str {
        &self.code
    }
    async fn run(&self, _ctx: &SchedulerContext) -> Result<SchedulerResult, SchedulerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(SchedulerResult {
            summary: "fake run".to_string(),
            processed: 1,
        })
    }
}

/// 种子计划：插入测试计划行（幂等，code 唯一）
async fn seed_test_plan(pool: &PgPool, code: &str, cron: &str) -> i64 {
    let id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl."zc_id_plan-perform" (notice, code, cron, created_by_id)
        VALUES ($1, $2, $3, 1)
        RETURNING id
        "#,
    )
    .bind(format!("scheduler-test-{code}"))
    .bind(code)
    .bind(cron)
    .fetch_one(pool)
    .await
    .expect("insert test plan");
    id
}

#[tokio::test]
async fn cron_matches_current_minute() {
    let s = CronSchedule::EveryMinutes(1);
    let now = chrono::Utc::now().timestamp();
    assert!(s.matches(now), "*/1 should match any minute");
}

#[tokio::test]
async fn scan_once_triggers_handler_and_writes_oper_planing() {
    let pool = test_pool().await;
    let code = format!("t-scan-{}", std::process::id());
    let plan_id = seed_test_plan(&pool, &code, "*/1 * * * *").await;

    let calls = Arc::new(AtomicU64::new(0));
    let svc = Arc::new(SchedulerService::new(pool.clone()));
    svc.register(Arc::new(CountHandler {
        code: code.clone(),
        calls: calls.clone(),
    }))
    .await;

    // 触发一轮：cron=*/1 当前分钟必然命中
    svc.scan_once().await.expect("scan_once");

    // handler 被调用
    assert!(
        calls.load(Ordering::SeqCst) >= 1,
        "handler should be called"
    );

    // oper-planing 写入执行实例
    let cnt: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-planing" WHERE fk_subject = $1 AND deleted_at IS NULL"#,
    )
    .bind(plan_id)
    .fetch_one(&pool)
    .await
    .expect("count oper-planing");
    assert!(cnt >= 1, "oper-planing execution instance should exist");

    // 清理测试计划行
    sqlx::query(r#"DELETE FROM isahl."zc_id_plan-perform" WHERE id = $1"#)
        .bind(plan_id)
        .execute(&pool)
        .await
        .expect("cleanup plan");
}

#[tokio::test]
async fn scan_once_same_minute_idempotent() {
    let pool = test_pool().await;
    let code = format!("t-idem-{}", std::process::id());
    let plan_id = seed_test_plan(&pool, &code, "*/1 * * * *").await;

    let calls = Arc::new(AtomicU64::new(0));
    let svc = Arc::new(SchedulerService::new(pool.clone()));
    svc.register(Arc::new(CountHandler {
        code: code.clone(),
        calls: calls.clone(),
    }))
    .await;

    // 同一分钟内两次扫描：第二次不重复触发
    svc.scan_once().await.expect("first scan");
    let first_calls = calls.load(Ordering::SeqCst);
    svc.scan_once().await.expect("second scan");
    let second_calls = calls.load(Ordering::SeqCst);

    assert_eq!(
        first_calls, second_calls,
        "same-minute second scan must not re-trigger (idempotent)"
    );

    sqlx::query(r#"DELETE FROM isahl."zc_id_plan-perform" WHERE id = $1"#)
        .bind(plan_id)
        .execute(&pool)
        .await
        .expect("cleanup plan");
}

#[tokio::test]
async fn unregistered_plan_records_execution() {
    let pool = test_pool().await;
    let code = format!("t-skip-{}", std::process::id());
    let plan_id = seed_test_plan(&pool, &code, "*/1 * * * *").await;

    // 不注册 handler：scan_once 应写执行实例（cron 到点记录，非跳过）
    let svc = Arc::new(SchedulerService::new(pool.clone()));
    svc.scan_once().await.expect("scan with no handler");

    // oper-planing 有执行实例（无 handler 计划也记录到点执行）
    let cnt: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-planing" WHERE fk_subject = $1 AND deleted_at IS NULL"#,
    )
    .bind(plan_id)
    .fetch_one(&pool)
    .await
    .expect("count oper-planing");
    assert!(
        cnt >= 1,
        "execution instance should be recorded for unregistered plan"
    );

    sqlx::query(r#"DELETE FROM isahl."zc_id_plan-perform" WHERE id = $1"#)
        .bind(plan_id)
        .execute(&pool)
        .await
        .expect("cleanup plan");
}
