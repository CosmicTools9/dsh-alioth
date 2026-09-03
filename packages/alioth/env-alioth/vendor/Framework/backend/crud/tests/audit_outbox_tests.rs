//! 审计 Outbox 集成测试（ADR D-010 链路：enqueue → worker 转写 → replay）。
//!
//! 测试表名带 `test_audit_` 前缀与 nanos 后缀，与真实审计数据隔离；
//! 每个用例结束清理自身产生的 outbox / data_change_logs 行。

use chrono::Utc;
use common::testing::connect_test_db;
use crud::audit_outbox::{
    enqueue, lag_stats, replay, AuditAction, AuditScope, OutboxEvent, OutboxWorker, ReplayFilter,
};
use sqlx::{AssertSqlSafe, PgPool};

fn test_table() -> String {
    format!(
        "test_audit_{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    )
}

async fn cleanup(pool: &PgPool, table: &str) {
    sqlx::query("DELETE FROM isahl_audit.audit_outbox WHERE table_name = $1")
        .bind(table)
        .execute(pool)
        .await
        .expect("cleanup outbox");
    sqlx::query("DELETE FROM isahl_audit.data_change_logs WHERE table_name = $1")
        .bind(table)
        .execute(pool)
        .await
        .expect("cleanup change logs");
}

fn event(table: &str, record_id: i64, action: AuditAction) -> OutboxEvent {
    OutboxEvent::new(table, record_id, action)
        .with_user(1)
        .with_values(
            None::<&serde_json::Value>,
            Some(&serde_json::json!({"id": record_id, "notice": "test"})),
        )
}

#[tokio::test]
async fn enqueue_inserts_pending_row() {
    let pool = connect_test_db().await;
    let table = test_table();

    let id = enqueue(&pool, &event(&table, 1001, AuditAction::Insert))
        .await
        .expect("enqueue");
    assert!(id > 0);

    let (status, tx_id): (String, Option<String>) =
        sqlx::query_as("SELECT status, transaction_id FROM isahl_audit.audit_outbox WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("fetch outbox row");
    assert_eq!(status, "pending");
    assert!(tx_id.is_some(), "transaction_id 应由 DB 惰性生成");

    cleanup(&pool, &table).await;
}

#[tokio::test]
async fn audit_scope_groups_transaction_id() {
    let pool = connect_test_db().await;
    let table = test_table();

    let tx_id = AuditScope::new_tx_id(&pool).await.expect("new tx id");
    let (id1, id2) = AuditScope::scope(tx_id.clone(), async {
        let a = enqueue(&pool, &event(&table, 2001, AuditAction::Insert))
            .await
            .expect("enqueue 1");
        let b = enqueue(&pool, &event(&table, 2002, AuditAction::Update))
            .await
            .expect("enqueue 2");
        (a, b)
    })
    .await;

    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT transaction_id FROM isahl_audit.audit_outbox WHERE id = ANY($1) ORDER BY id",
    )
    .bind(vec![id1, id2])
    .fetch_all(&pool)
    .await
    .expect("fetch tx ids");
    assert_eq!(ids.len(), 2);
    assert!(
        ids.iter().all(|t| t == &tx_id),
        "scope 内两次写入应共享同一 transaction_id"
    );

    cleanup(&pool, &table).await;
}

#[tokio::test]
async fn worker_relays_to_change_logs() {
    let pool = connect_test_db().await;
    let table = test_table();

    let outbox_id = enqueue(&pool, &event(&table, 3001, AuditAction::Update))
        .await
        .expect("enqueue");
    let expected_ts: chrono::DateTime<Utc> =
        sqlx::query_scalar("SELECT action_timestamp FROM isahl_audit.audit_outbox WHERE id = $1")
            .bind(outbox_id)
            .fetch_one(&pool)
            .await
            .expect("fetch outbox ts");

    let worker = OutboxWorker::new(pool.clone());
    let n = worker.run_once().await.expect("run_once");
    assert!(n >= 1, "至少转写一条");

    // data_change_logs 有对应行，action_timestamp 透传业务事务时刻
    let (action, record_id, ts): (String, i64, chrono::DateTime<Utc>) = sqlx::query_as(
        "SELECT action, record_id, action_timestamp FROM isahl_audit.data_change_logs WHERE table_name = $1",
    )
    .bind(&table)
    .fetch_one(&pool)
    .await
    .expect("fetch data_change_logs row");
    assert_eq!(action, "UPDATE");
    assert_eq!(record_id, 3001);
    assert_eq!(
        ts, expected_ts,
        "action_timestamp 必须透传 outbox 行创建时刻（物理时间锚）"
    );

    // outbox 行已标记 done
    let status: String =
        sqlx::query_scalar("SELECT status FROM isahl_audit.audit_outbox WHERE id = $1")
            .bind(outbox_id)
            .fetch_one(&pool)
            .await
            .expect("fetch status");
    assert_eq!(status, "done");

    cleanup(&pool, &table).await;
}

#[tokio::test]
async fn replay_resets_failed_and_dead() {
    let pool = connect_test_db().await;
    let table = test_table();

    let id = enqueue(&pool, &event(&table, 4001, AuditAction::Delete))
        .await
        .expect("enqueue");
    // 模拟失败终态
    sqlx::query(
        "UPDATE isahl_audit.audit_outbox SET status = 'dead', attempts = 9, last_error = 'poison' WHERE id = $1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .expect("mark dead");

    let n = replay(&pool, &ReplayFilter::default())
        .await
        .expect("replay");
    assert!(n >= 1);

    let (status, err): (String, Option<String>) =
        sqlx::query_as("SELECT status, last_error FROM isahl_audit.audit_outbox WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("fetch after replay");
    assert_eq!(status, "pending");
    assert!(err.is_none(), "重放应清除 last_error");

    cleanup(&pool, &table).await;
}

#[tokio::test]
async fn lag_stats_reports_backlog() {
    let pool = connect_test_db().await;
    let table = test_table();

    enqueue(&pool, &event(&table, 5001, AuditAction::Insert))
        .await
        .expect("enqueue");

    let (n, age) = lag_stats(&pool).await.expect("lag stats");
    assert!(n >= 1, "应观测到至少一条积压");
    assert!(age.is_some(), "积压存在时最老年龄应可读");

    cleanup(&pool, &table).await;
}

/// slow path：poison 行（relay 时被临时 CHECK 约束拒绝）使整批原子转写失败 →
/// 降级逐条 → 正常行照常转写，poison 行退避重试，超阈值标 dead 且不阻塞同伴。
/// poison 表名与约束名带 nanos 唯一后缀，并行测试互不影响。
#[tokio::test]
async fn worker_degrades_and_marks_poison_dead() {
    let pool = connect_test_db().await;
    let table = test_table();
    let suffix = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let poison_table = format!("test_audit_poison_{}", suffix);
    let constraint = format!("tmp_reject_poison_{}", suffix);

    // 临时约束：relay 写 poison_table 必失败（NOT VALID 仅约束新写入）
    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE isahl_audit.data_change_logs ADD CONSTRAINT {} CHECK (table_name != '{}') NOT VALID",
        constraint, poison_table
    ).as_str()))
    .execute(&pool)
    .await
    .expect("add reject constraint");

    let result = async {
        // 一条正常 + 一条 poison
        let good_id = enqueue(&pool, &event(&table, 6001, AuditAction::Insert))
            .await
            .expect("enqueue good");
        let poison_id = enqueue(&pool, &event(&poison_table, 6002, AuditAction::Insert))
            .await
            .expect("enqueue poison");

        let mut worker = OutboxWorker::new(pool.clone());
        worker.max_attempts = 2;

        // 第 1 轮：fast path 批原子失败 → slow path 逐条：good done，poison failed
        let n = worker.run_once().await.expect("run_once 1");
        assert!(n >= 1, "正常行应被逐条转写");

        let good_status: String =
            sqlx::query_scalar("SELECT status FROM isahl_audit.audit_outbox WHERE id = $1")
                .bind(good_id)
                .fetch_one(&pool)
                .await
                .expect("good status");
        assert_eq!(good_status, "done", "poison 不得阻塞同伴");

        // 第 2 轮：拨回退避窗口（指数退避 next_retry_at 默认 5s 后），poison attempts 达阈值 → dead
        sqlx::query(
            "UPDATE isahl_audit.audit_outbox SET next_retry_at = now() - interval '1 second' WHERE id = $1",
        )
        .bind(poison_id)
        .execute(&pool)
        .await
        .expect("rewind backoff");
        let _ = worker.run_once().await.expect("run_once 2");
        let (poison_status, attempts): (String, i32) =
            sqlx::query_as("SELECT status, attempts FROM isahl_audit.audit_outbox WHERE id = $1")
                .bind(poison_id)
                .fetch_one(&pool)
                .await
                .expect("poison status");
        assert_eq!(poison_status, "dead", "超阈值 poison 应标 dead 死信");
        assert!(attempts >= 2);

        // data_change_logs 只有正常行的记录
        let log_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM isahl_audit.data_change_logs WHERE table_name = $1",
        )
        .bind(&table)
        .fetch_one(&pool)
        .await
        .expect("log count");
        assert_eq!(log_count, 1, "仅正常行转写成功");

        cleanup(&pool, &table).await;
        cleanup(&pool, &poison_table).await;
    }
    .await;

    sqlx::query(AssertSqlSafe(
        format!(
            "ALTER TABLE isahl_audit.data_change_logs DROP CONSTRAINT IF EXISTS {}",
            constraint
        )
        .as_str(),
    ))
    .execute(&pool)
    .await
    .expect("drop reject constraint");

    result
}
