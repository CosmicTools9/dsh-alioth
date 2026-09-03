//! 审计 Outbox（ADR D-010：Rust 层写入机制，禁 DB 触发器）。
//!
//! ## 架构
//!
//! 1. **enqueue**：业务写路径在源数据写入后随即插入一条 `isahl_audit.audit_outbox`
//!    行——同事务场景用 [`enqueue_tx`]（严格零丢失），pool 直插场景用 [`enqueue`]
//!    （轻量、逐语句事务；严格化迁移路径见 change tasks 3.3 写路径盘点）。
//! 2. **worker**：[`OutboxWorker`] 独立事务批量转写 outbox → `data_change_logs`
//!    （业务事务不被审计阻塞/回滚）。竞争领取 `FOR UPDATE SKIP LOCKED`，
//!    fast path 整批原子，失败降级 slow path 逐条定位 poison 标 `dead`。
//! 3. **replay**：[`replay`] 将 `failed`/`dead` 行重置 `pending`——重放源 =
//!    outbox 持久行，进程崩溃不丢事件。
//!
//! ## 事务成组
//!
//! [`AuditScope::begin`] 在请求作用域生成 `transaction_id`（zuid 字符串），
//! scope 内所有 enqueue 共享同一 ID；无 scope 时退化为每写一个新 ID（单写成组）。

use chrono::{DateTime, Duration, Utc};
use common::AliothError;
use serde::Serialize;
use serde_json::Value as JsonValue;
use sqlx::{FromRow, PgPool};
use std::future::Future;

// ── 事务作用域（transaction_id 贯穿） ─────────────────────────────────────

tokio::task_local! {
    static AUDIT_TX_ID: String;
}

/// 审计事务作用域：`begin` 生成一次 zuid 作为 transaction_id，
/// scope 内全部 enqueue 共享（同事务多写成组）。
pub struct AuditScope;

impl AuditScope {
    /// 从 DB 取一个 zuid 作为本 scope 的 transaction_id。
    pub async fn new_tx_id(pool: &PgPool) -> Result<String, AliothError> {
        let id: i64 = sqlx::query_scalar("SELECT isahl.gen_next_zuid()")
            .fetch_one(pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(id.to_string())
    }

    /// 在指定 transaction_id 作用域内执行闭包。
    pub async fn scope<F, T>(tx_id: String, f: F) -> T
    where
        F: Future<Output = T>,
    {
        AUDIT_TX_ID.scope(tx_id, f).await
    }

    /// 当前作用域 transaction_id（未在 scope 内 → None，enqueue 时惰性生成）。
    pub fn current() -> Option<String> {
        AUDIT_TX_ID.try_with(|s| s.clone()).ok()
    }
}

// ── 事件载荷 ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditAction {
    Insert,
    Update,
    Delete,
}

impl AuditAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
        }
    }
}

/// 一条待转写的审计事件（对齐 `data_change_logs` 列子集）。
#[derive(Debug, Clone, Default)]
pub struct OutboxEvent {
    pub table_schema: Option<String>,
    pub table_name: String,
    pub record_id: i64,
    pub action: Option<AuditAction>,
    pub changed_fields: Option<JsonValue>,
    pub old_values: Option<JsonValue>,
    pub new_values: Option<JsonValue>,
    pub performed_by_id: Option<i64>,
    pub performed_by_email: Option<String>,
    /// None → 取 [`AuditScope::current`]；再 None → DB 惰性生成（单写成组）。
    pub transaction_id: Option<String>,
    pub session_id: Option<String>,
    pub request_path: Option<String>,
    pub request_method: Option<String>,
    pub context: Option<JsonValue>,
}

impl OutboxEvent {
    pub fn new(table_name: impl Into<String>, record_id: i64, action: AuditAction) -> Self {
        Self {
            table_name: table_name.into(),
            record_id,
            action: Some(action),
            ..Default::default()
        }
    }

    /// 从全限定表名（`isahl.zc_id_version` / `isahl."zc_id_file-document"`）构造，
    /// 自动拆分 schema/table（表名去引号）。
    pub fn for_table(full: &str, record_id: i64, action: AuditAction) -> Self {
        let (schema, table) = full.rsplit_once('.').unwrap_or(("isahl", full));
        Self {
            table_schema: Some(schema.trim_matches('"').to_string()),
            table_name: table.trim_matches('"').to_string(),
            record_id,
            action: Some(action),
            ..Default::default()
        }
    }

    pub fn with_user(mut self, user_id: i64) -> Self {
        self.performed_by_id = Some(user_id);
        self
    }

    /// 实体序列化为 new/old values（复用 trigger::to_record 的 map 形态）。
    pub fn with_values<T: Serialize>(mut self, old: Option<&T>, new: Option<&T>) -> Self {
        if let Some(o) = old {
            self.old_values = crate::trigger::to_record(o)
                .ok()
                .and_then(|m| serde_json::to_value(m).ok());
        }
        if let Some(n) = new {
            self.new_values = crate::trigger::to_record(n)
                .ok()
                .and_then(|m| serde_json::to_value(m).ok());
        }
        self
    }
}

// ── enqueue ──────────────────────────────────────────────────────────────

const ENQUEUE_SQL: &str = r#"
    INSERT INTO isahl_audit.audit_outbox
        (table_schema, table_name, record_id, action,
         changed_fields, old_values, new_values,
         performed_by_id, performed_by_email, transaction_id,
         session_id, request_path, request_method, context)
    VALUES (COALESCE($1, 'isahl'), $2, $3, $4, $5, $6, $7, $8, $9,
            COALESCE($10, isahl.gen_next_zuid()::text),
            $11, $12, $13, $14)
    RETURNING id
"#;

/// pool 直插（逐语句事务）——轻量路径；严格同事务用 [`enqueue_tx`]。
pub async fn enqueue(pool: &PgPool, event: &OutboxEvent) -> Result<i64, AliothError> {
    let tx_id = event.transaction_id.clone().or_else(AuditScope::current);
    sqlx::query_scalar::<_, i64>(ENQUEUE_SQL)
        .bind(event.table_schema.as_deref())
        .bind(&event.table_name)
        .bind(event.record_id)
        .bind(event.action.map(|a| a.as_str()))
        .bind(&event.changed_fields)
        .bind(&event.old_values)
        .bind(&event.new_values)
        .bind(event.performed_by_id)
        .bind(event.performed_by_email.as_deref())
        .bind(tx_id)
        .bind(event.session_id.as_deref())
        .bind(event.request_path.as_deref())
        .bind(event.request_method.as_deref())
        .bind(&event.context)
        .fetch_one(pool)
        .await
        .map_err(|e| AliothError::Database(e.to_string()))
}

/// 业务事务内插入（严格零丢失：与源数据写入同生共死）。
pub async fn enqueue_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &OutboxEvent,
) -> Result<i64, sqlx::Error> {
    let tx_id = event.transaction_id.clone().or_else(AuditScope::current);
    sqlx::query_scalar::<_, i64>(ENQUEUE_SQL)
        .bind(event.table_schema.as_deref())
        .bind(&event.table_name)
        .bind(event.record_id)
        .bind(event.action.map(|a| a.as_str()))
        .bind(&event.changed_fields)
        .bind(&event.old_values)
        .bind(&event.new_values)
        .bind(event.performed_by_id)
        .bind(event.performed_by_email.as_deref())
        .bind(tx_id)
        .bind(event.session_id.as_deref())
        .bind(event.request_path.as_deref())
        .bind(event.request_method.as_deref())
        .bind(&event.context)
        .fetch_one(&mut **tx)
        .await
}

// ── worker ───────────────────────────────────────────────────────────────

#[derive(Debug, FromRow)]
struct OutboxRow {
    id: i64,
    table_schema: String,
    table_name: String,
    record_id: i64,
    action: String,
    action_timestamp: DateTime<Utc>,
    changed_fields: Option<JsonValue>,
    old_values: Option<JsonValue>,
    new_values: Option<JsonValue>,
    performed_by_id: Option<i64>,
    performed_by_email: Option<String>,
    transaction_id: Option<String>,
    session_id: Option<String>,
    client_ip: Option<String>,
    user_agent: Option<String>,
    request_path: Option<String>,
    request_method: Option<String>,
    context: Option<JsonValue>,
}

const RELAY_SQL: &str = r#"
    INSERT INTO isahl_audit.data_change_logs
        (table_schema, table_name, record_id, action, action_timestamp,
         changed_fields, old_values, new_values,
         performed_by_id, performed_by_email, transaction_id,
         session_id, client_ip, user_agent, request_path, request_method, context)
    VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13::inet,$14,$15,$16,$17)
"#;

/// Outbox 转写 worker：每进程内嵌一个（actix 后台任务）。
pub struct OutboxWorker {
    pool: PgPool,
    /// 单批领取条数
    pub batch_size: i64,
    /// 轮询间隔（无任务时退避）
    pub poll_interval: std::time::Duration,
    /// 超此次数标 dead（slow path 逐条判定时生效）
    pub max_attempts: i32,
}

impl OutboxWorker {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            batch_size: 100,
            poll_interval: std::time::Duration::from_secs(2),
            max_attempts: 8,
        }
    }

    /// 转写一批。返回成功转写条数。
    ///
    /// fast path：claim + relay + mark done 单事务原子；任一条失败整批回滚，
    /// 降级 slow path 逐条小事务定位 poison（标 dead），其余正常转写。
    pub async fn run_once(&self) -> Result<usize, AliothError> {
        match self.run_batch_atomic().await {
            Ok(n) => Ok(n),
            Err(e) => {
                common::telemetry::warn!(
                    "audit_outbox batch atomic failed, degrade to per-row: {}",
                    e
                );
                self.run_batch_per_row().await
            }
        }
    }

    /// 持续运行（每进程一个后台任务）；`shutdown` 收到 true 时退出。
    pub async fn run_forever(self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        loop {
            if *shutdown.borrow() {
                break;
            }
            match self.run_once().await {
                Ok(0) => {
                    // 空转：退避等待（可被 shutdown 打断）
                    let _ = tokio::time::timeout(self.poll_interval, shutdown.changed()).await;
                }
                Ok(n) => {
                    common::telemetry::info!("audit_outbox relayed {} row(s)", n);
                }
                Err(e) => {
                    common::telemetry::error!("audit_outbox worker error: {}", e);
                    let _ = tokio::time::timeout(self.poll_interval, shutdown.changed()).await;
                }
            }
        }
    }

    async fn claim_rows(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<Vec<OutboxRow>, sqlx::Error> {
        sqlx::query_as::<_, OutboxRow>(
            r#"UPDATE isahl_audit.audit_outbox
               SET status = 'processing', attempts = attempts + 1
               WHERE id IN (
                   SELECT id FROM isahl_audit.audit_outbox
                   WHERE status IN ('pending', 'failed') AND next_retry_at <= now()
                   ORDER BY id
                   LIMIT $1
                   FOR UPDATE SKIP LOCKED
               )
               RETURNING id, table_schema, table_name, record_id, action, action_timestamp,
                         changed_fields, old_values, new_values,
                         performed_by_id, performed_by_email, transaction_id,
                         session_id, client_ip::text, user_agent,
                         request_path, request_method, context"#,
        )
        .bind(self.batch_size)
        .fetch_all(&mut **tx)
        .await
    }

    async fn relay_row(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        row: &OutboxRow,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(RELAY_SQL)
            .bind(&row.table_schema)
            .bind(&row.table_name)
            .bind(row.record_id)
            .bind(&row.action)
            .bind(row.action_timestamp) // 透传业务事务时刻，保物理时间锚
            .bind(&row.changed_fields)
            .bind(&row.old_values)
            .bind(&row.new_values)
            .bind(row.performed_by_id)
            .bind(row.performed_by_email.as_deref())
            .bind(row.transaction_id.as_deref())
            .bind(row.session_id.as_deref())
            .bind(row.client_ip.as_deref())
            .bind(row.user_agent.as_deref())
            .bind(row.request_path.as_deref())
            .bind(row.request_method.as_deref())
            .bind(&row.context)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    async fn mark_done(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE isahl_audit.audit_outbox SET status='done', processed_at=now() WHERE id=$1",
        )
        .bind(id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn run_batch_atomic(&self) -> Result<usize, AliothError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        let rows = self
            .claim_rows(&mut tx)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        if rows.is_empty() {
            tx.rollback()
                .await
                .map_err(|e| AliothError::Database(e.to_string()))?;
            return Ok(0);
        }
        for row in &rows {
            self.relay_row(&mut tx, row)
                .await
                .map_err(|e| AliothError::Database(e.to_string()))?;
            self.mark_done(&mut tx, row.id)
                .await
                .map_err(|e| AliothError::Database(e.to_string()))?;
        }
        let n = rows.len();
        tx.commit()
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(n)
    }

    /// 逐条降级：每条独立小事务；失败标记退避/dead，不阻塞同伴。
    async fn run_batch_per_row(&self) -> Result<usize, AliothError> {
        let mut ok = 0usize;
        loop {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| AliothError::Database(e.to_string()))?;
            let mut rows = {
                let mut one = self.clone_for_single();
                one.batch_size = 1;
                one.claim_rows(&mut tx)
                    .await
                    .map_err(|e| AliothError::Database(e.to_string()))?
            };
            let Some(row) = rows.pop() else {
                tx.rollback()
                    .await
                    .map_err(|e| AliothError::Database(e.to_string()))?;
                break;
            };
            let result = self.relay_row(&mut tx, &row).await;
            match result {
                Ok(()) => {
                    self.mark_done(&mut tx, row.id)
                        .await
                        .map_err(|e| AliothError::Database(e.to_string()))?;
                    tx.commit()
                        .await
                        .map_err(|e| AliothError::Database(e.to_string()))?;
                    ok += 1;
                }
                Err(e) => {
                    tx.rollback()
                        .await
                        .map_err(|e| AliothError::Database(e.to_string()))?;
                    self.mark_retry_or_dead(row.id, &e.to_string()).await?;
                }
            }
        }
        Ok(ok)
    }

    fn clone_for_single(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            batch_size: 1,
            poll_interval: self.poll_interval,
            max_attempts: self.max_attempts,
        }
    }

    /// 失败标记：attempts+1 并指数退避；超 max_attempts 标 dead（死信人工介入）。
    async fn mark_retry_or_dead(&self, id: i64, err: &str) -> Result<(), AliothError> {
        sqlx::query(
            r#"UPDATE isahl_audit.audit_outbox
               SET status = CASE WHEN attempts + 1 >= $2 THEN 'dead' ELSE 'failed' END,
                   attempts = attempts + 1,
                   next_retry_at = now() + (interval '1 second' * LEAST(power(2, attempts) * 5, 3600)),
                   last_error = $3
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(self.max_attempts)
        .bind(&err[..err.len().min(2000)])
        .execute(&self.pool)
        .await
        .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(())
    }
}

// ── replay ───────────────────────────────────────────────────────────────

/// 重放过滤：默认重置全部 failed/dead。
#[derive(Debug, Clone, Default)]
pub struct ReplayFilter {
    /// 目标状态集（默认 failed + dead）
    pub statuses: Option<Vec<String>>,
    pub table_name: Option<String>,
    pub before: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
}

/// 重放：把 failed/dead 行重置为 pending（重放源 = outbox 持久行）。
/// 返回重置条数。
pub async fn replay(pool: &PgPool, filter: &ReplayFilter) -> Result<u64, AliothError> {
    let statuses = filter
        .statuses
        .clone()
        .unwrap_or_else(|| vec!["failed".into(), "dead".into()]);
    let result = sqlx::query(
        r#"UPDATE isahl_audit.audit_outbox
           SET status = 'pending', next_retry_at = now(), last_error = NULL
           WHERE status = ANY($1)
             AND ($2::text IS NULL OR table_name = $2)
             AND ($3::timestamptz IS NULL OR created_at <= $3)
             AND id IN (
                 SELECT id FROM isahl_audit.audit_outbox
                 WHERE status = ANY($1)
                 ORDER BY id
                 LIMIT COALESCE($4, 10000)
             )"#,
    )
    .bind(&statuses)
    .bind(filter.table_name.as_deref())
    .bind(filter.before)
    .bind(filter.limit)
    .execute(pool)
    .await
    .map_err(|e| AliothError::Database(e.to_string()))?;
    Ok(result.rows_affected())
}

// ── 主状态（r_primary-status）审计 helper ────────────────────────────────
//
// 现状写点形态：upsert 对（UPDATE 原地迁移 ref_right + else INSERT 初始行）。
// UPDATE 原地覆盖旧值——old 必须先取后写（`fetch_primary_status` 迁移前调用）。

const PRIMARY_STATUS_TABLE: &str = "isahl.\"zc_id_lifecycle_r_primary-status\"";

/// 实体当前主状态（迁移前调用取 old；软删行除外）。
pub async fn fetch_primary_status(
    pool: &PgPool,
    entity_id: i64,
) -> Result<Option<i64>, AliothError> {
    sqlx::query_scalar::<_, i64>(
        r#"SELECT ref_right FROM isahl."zc_id_lifecycle_r_primary-status" WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(entity_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AliothError::Database(e.to_string()))
}

/// tx 版（迁移前同事务取 old，防并发漂移）。
pub async fn fetch_primary_status_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    entity_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        r#"SELECT ref_right FROM isahl."zc_id_lifecycle_r_primary-status" WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(entity_id)
    .fetch_optional(&mut **tx)
    .await
}

/// 主状态行三态查询（含软删行）：`Some((ref_right, is_active))`。
/// `ref_left` 全表唯一——软删行仍占位，UPSERT 必须走 restore 而非 INSERT，
/// 否则撞唯一约束（`fetch_primary_status*` 只查活跃行，不足以判 UPSERT 分支）。
pub async fn fetch_primary_status_row_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    entity_id: i64,
) -> Result<Option<(i64, bool)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, bool)>(
        r#"SELECT ref_right, (deleted_at IS NULL) FROM isahl."zc_id_lifecycle_r_primary-status" WHERE ref_left = $1"#,
    )
    .bind(entity_id)
    .fetch_optional(&mut **tx)
    .await
}

/// pool 版三态查询。
pub async fn fetch_primary_status_row(
    pool: &PgPool,
    entity_id: i64,
) -> Result<Option<(i64, bool)>, AliothError> {
    sqlx::query_as::<_, (i64, bool)>(
        r#"SELECT ref_right, (deleted_at IS NULL) FROM isahl."zc_id_lifecycle_r_primary-status" WHERE ref_left = $1"#,
    )
    .bind(entity_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AliothError::Database(e.to_string()))
}

fn primary_status_event(
    entity_id: i64,
    old: Option<i64>,
    new: i64,
    user_id: Option<i64>,
) -> OutboxEvent {
    let action = if old.is_some() {
        AuditAction::Update
    } else {
        AuditAction::Insert
    };
    let mut ev = OutboxEvent::for_table(PRIMARY_STATUS_TABLE, entity_id, action);
    ev.old_values = old.map(|s| serde_json::json!({ "ref_right": s }));
    ev.new_values = Some(serde_json::json!({ "ref_right": new }));
    ev.performed_by_id = user_id;
    ev
}

/// 主状态变更审计（迁移后调用）：record_id = 实体 id（ref_left），
/// old/new_values = {ref_right} 迁移前后——主状态时间线即按
/// (table_name, record_id) ORDER BY action_timestamp 成轴。
pub async fn audit_primary_status(
    pool: &PgPool,
    entity_id: i64,
    old: Option<i64>,
    new: i64,
    user_id: Option<i64>,
) -> Result<i64, AliothError> {
    enqueue(pool, &primary_status_event(entity_id, old, new, user_id)).await
}

/// tx 版（与迁移写同事务，严格零丢失）。
pub async fn audit_primary_status_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    entity_id: i64,
    old: Option<i64>,
    new: i64,
    user_id: Option<i64>,
) -> Result<i64, sqlx::Error> {
    enqueue_tx(tx, &primary_status_event(entity_id, old, new, user_id)).await
}

/// 主状态撤销审计（软删关系行，如 unmark done）：Delete 动作 + old={ref_right}。
pub async fn audit_primary_status_delete(
    pool: &PgPool,
    entity_id: i64,
    old: i64,
    user_id: Option<i64>,
) -> Result<i64, AliothError> {
    let mut ev = OutboxEvent::for_table(PRIMARY_STATUS_TABLE, entity_id, AuditAction::Delete);
    ev.old_values = Some(serde_json::json!({ "ref_right": old }));
    ev.performed_by_id = user_id;
    enqueue(pool, &ev).await
}

/// 装配入口：进程启动时内嵌 worker 后台任务。
/// 返回 shutdown 开关——进程优雅退出时 `send(true)`。
pub fn spawn_worker(pool: PgPool) -> tokio::sync::watch::Sender<bool> {
    let (tx, rx) = tokio::sync::watch::channel(false);
    let worker = OutboxWorker::new(pool);
    tokio::spawn(async move {
        worker.run_forever(rx).await;
        common::telemetry::info!("audit_outbox worker stopped");
    });
    common::telemetry::info!("audit_outbox worker spawned");
    tx
}

/// 滞后观测：pending/failed 行数与最老未处理行年龄（秒）。
pub async fn lag_stats(pool: &PgPool) -> Result<(i64, Option<f64>), AliothError> {
    let (n, age): (i64, Option<f64>) = sqlx::query_as(
        r#"SELECT count(*)::bigint,
                  EXTRACT(EPOCH FROM now() - min(created_at))::float8
           FROM isahl_audit.audit_outbox
           WHERE status IN ('pending', 'failed')"#,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| AliothError::Database(e.to_string()))?;
    Ok((n, age))
}

/// 保留 Duration 类型引用（退避计算文档化）；实际退避在 SQL 内完成。
#[allow(dead_code)]
fn backoff_hint(attempts: i32) -> Duration {
    Duration::seconds((2i64.pow(attempts.max(0) as u32) * 5).min(3600))
}
