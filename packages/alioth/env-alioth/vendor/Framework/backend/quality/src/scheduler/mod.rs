use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};

/// 测试运行类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TestRunType {
    Scheduled, // 定时任务
    Manual,    // 手动触发
    Api,       // API触发
}

/// 测试运行状态
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TestRunStatus {
    Running,   // 运行中
    Completed, // 已完成
    Failed,    // 失败
    Cancelled, // 已取消
}

impl TestRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TestRunStatus::Running => "RUNNING",
            TestRunStatus::Completed => "COMPLETED",
            TestRunStatus::Failed => "FAILED",
            TestRunStatus::Cancelled => "CANCELLED",
        }
    }
}

/// 采样策略
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SamplingStrategy {
    Full,                                          // 全量检测
    Random { percentage: f64, seed: Option<u64> }, // 随机采样
    Stratified { column: String, buckets: i32 },   // 分层采样
    Recent { limit: i64, order_by: String },       // 最近N条
}

/// 测试运行记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRun {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub rule_id: i64,
    pub run_type: TestRunType,
    pub status: TestRunStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sample_size: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub total_rows: Option<i64>,
    pub execution_results: serde_json::Value,
    pub error_message: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub triggered_by: Option<i64>,
    pub sampling_strategy: SamplingStrategy,
}

/// 调度任务配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledJob {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub rule_id: i64,
    pub name: String,
    pub cron_expression: String,
    pub timezone: String,
    pub enabled: bool,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// 调度服务
pub struct SchedulerService {
    db_pool: PgPool,
}

impl SchedulerService {
    pub fn new(db_pool: PgPool) -> Self {
        Self { db_pool }
    }

    /// 创建定时任务
    async fn generate_id(&self) -> Result<i64, sqlx::Error> {
        let id: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(id), 0) + 1 FROM isahl_meta.quality_scheduled_jobs",
        )
        .fetch_one(&self.db_pool)
        .await?;
        Ok(id)
    }

    pub async fn schedule_job(
        &self,
        rule_id: i64,
        name: String,
        cron_expression: String,
        timezone: String,
    ) -> Result<i64, sqlx::Error> {
        let id = self.generate_id().await?;
        let now = Utc::now();

        // 计算下次执行时间 (简化实现)
        let next_run_at = self.calculate_next_run(&cron_expression, &timezone);

        let sql = r#"
            INSERT INTO isahl_meta.quality_scheduled_jobs (
                id, rule_id, name, cron_expression, timezone, enabled, next_run_at, created_at
            ) VALUES ($1, $2, $3, $4, $5, true, $6, $7)
            RETURNING id
        "#;

        let job_id: i64 = sqlx::query_scalar(AssertSqlSafe(sql))
            .bind(id)
            .bind(rule_id)
            .bind(name)
            .bind(cron_expression)
            .bind(timezone)
            .bind(next_run_at)
            .bind(now)
            .fetch_one(&self.db_pool)
            .await?;

        Ok(job_id)
    }

    /// 取消定时任务
    pub async fn unschedule_job(&self, job_id: i64) -> Result<(), sqlx::Error> {
        let sql = "UPDATE isahl_meta.quality_scheduled_jobs SET enabled = false WHERE id = $1";
        sqlx::query(AssertSqlSafe(sql))
            .bind(job_id)
            .execute(&self.db_pool)
            .await?;
        Ok(())
    }

    /// 获取待执行的任务
    pub async fn get_pending_jobs(&self, limit: i64) -> Result<Vec<ScheduledJob>, sqlx::Error> {
        let sql = r#"
            SELECT id, rule_id, name, cron_expression, timezone, enabled, last_run_at, next_run_at, created_at
            FROM isahl_meta.quality_scheduled_jobs
            WHERE enabled = true AND (next_run_at IS NULL OR next_run_at <= NOW())
            ORDER BY next_run_at ASC
            LIMIT $1
        "#;

        let rows: Vec<ScheduledJobRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(limit)
            .fetch_all(&self.db_pool)
            .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// 更新任务执行时间
    pub async fn update_job_run_time(&self, job_id: i64) -> Result<(), sqlx::Error> {
        let job = self.get_job_by_id(job_id).await?;
        if let Some(job) = job {
            let next_run_at = self.calculate_next_run(&job.cron_expression, &job.timezone);

            let sql = r#"
                UPDATE isahl_meta.quality_scheduled_jobs
                SET last_run_at = NOW(), next_run_at = $1
                WHERE id = $2
            "#;

            sqlx::query(AssertSqlSafe(sql))
                .bind(next_run_at)
                .bind(job_id)
                .execute(&self.db_pool)
                .await?;
        }
        Ok(())
    }

    /// 获取任务详情
    async fn get_job_by_id(&self, job_id: i64) -> Result<Option<ScheduledJob>, sqlx::Error> {
        let sql = r#"
            SELECT id, rule_id, name, cron_expression, timezone, enabled, last_run_at, next_run_at, created_at
            FROM isahl_meta.quality_scheduled_jobs
            WHERE id = $1
        "#;

        let row = sqlx::query_as::<_, ScheduledJobRow>(AssertSqlSafe(sql))
            .bind(job_id)
            .fetch_optional(&self.db_pool)
            .await?;

        Ok(row.map(|r| r.into()))
    }

    /// 计算下次执行时间 (简化实现)
    fn calculate_next_run(&self, _cron_expression: &str, _timezone: &str) -> Option<DateTime<Utc>> {
        // 简化实现: 返回当前时间 + 1小时
        Some(Utc::now() + chrono::Duration::hours(1))
    }
}

/// 测试运行服务
pub struct TestRunService {
    db_pool: PgPool,
}

impl TestRunService {
    pub fn new(db_pool: PgPool) -> Self {
        Self { db_pool }
    }

    /// 创建测试运行记录
    async fn generate_id(&self) -> Result<i64, sqlx::Error> {
        let id: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) + 1 FROM isahl_meta.quality_test_runs")
                .fetch_one(&self.db_pool)
                .await?;
        Ok(id)
    }

    pub async fn create_test_run(
        &self,
        rule_id: i64,
        run_type: TestRunType,
        sampling_strategy: SamplingStrategy,
        triggered_by: Option<i64>,
    ) -> Result<i64, sqlx::Error> {
        let id = self.generate_id().await?;
        let now = Utc::now();

        let sql = r#"
            INSERT INTO isahl_meta.quality_test_runs (
                id, rule_id, run_type, status, started_at, sampling_strategy, triggered_by
            ) VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id
        "#;

        let run_id: i64 = sqlx::query_scalar(AssertSqlSafe(sql))
            .bind(id)
            .bind(rule_id)
            .bind(match run_type {
                TestRunType::Scheduled => "SCHEDULED",
                TestRunType::Manual => "MANUAL",
                TestRunType::Api => "API",
            })
            .bind(TestRunStatus::Running.as_str())
            .bind(now)
            .bind(serde_json::to_value(sampling_strategy).unwrap_or_default())
            .bind(triggered_by)
            .fetch_one(&self.db_pool)
            .await?;

        Ok(run_id)
    }

    /// 完成测试运行
    pub async fn complete_test_run(
        &self,
        run_id: i64,
        status: TestRunStatus,
        total_rows: i64,
        execution_results: serde_json::Value,
        error_message: Option<String>,
    ) -> Result<(), sqlx::Error> {
        let sql = r#"
            UPDATE isahl_meta.quality_test_runs
            SET status = $1, completed_at = NOW(), total_rows = $2, 
                execution_results = $3, error_message = $4
            WHERE id = $5
        "#;

        sqlx::query(AssertSqlSafe(sql))
            .bind(status.as_str())
            .bind(total_rows)
            .bind(execution_results)
            .bind(error_message)
            .bind(run_id)
            .execute(&self.db_pool)
            .await?;

        Ok(())
    }

    /// 获取测试运行历史
    pub async fn get_test_run_history(
        &self,
        rule_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<TestRun>, sqlx::Error> {
        let mut sql = String::from(
            "SELECT id, rule_id, run_type, status, started_at, completed_at, \
             sample_size, total_rows, execution_results, error_message, triggered_by, sampling_strategy \
             FROM isahl_meta.quality_test_runs"
        );

        if rule_id.is_some() {
            sql.push_str(" WHERE rule_id = $1");
        }
        sql.push_str(" ORDER BY started_at DESC LIMIT $2");

        let query = sqlx::query_as::<_, TestRunRow>(AssertSqlSafe(sql.as_str()));

        let rows: Vec<TestRunRow> = if let Some(rid) = rule_id {
            query.bind(rid).bind(limit).fetch_all(&self.db_pool).await?
        } else {
            query.bind(limit).fetch_all(&self.db_pool).await?
        };

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }
}

// 数据库行结构
#[derive(sqlx::FromRow)]
struct ScheduledJobRow {
    id: i64,
    rule_id: i64,
    name: String,
    cron_expression: String,
    timezone: String,
    enabled: bool,
    last_run_at: Option<DateTime<Utc>>,
    next_run_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

impl From<ScheduledJobRow> for ScheduledJob {
    fn from(row: ScheduledJobRow) -> Self {
        Self {
            id: row.id,
            rule_id: row.rule_id,
            name: row.name,
            cron_expression: row.cron_expression,
            timezone: row.timezone,
            enabled: row.enabled,
            last_run_at: row.last_run_at,
            next_run_at: row.next_run_at,
            created_at: row.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct TestRunRow {
    id: i64,
    rule_id: i64,
    run_type: String,
    status: String,
    started_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
    sample_size: Option<i64>,
    total_rows: Option<i64>,
    execution_results: serde_json::Value,
    error_message: Option<String>,
    triggered_by: Option<i64>,
    sampling_strategy: serde_json::Value,
}

impl From<TestRunRow> for TestRun {
    fn from(row: TestRunRow) -> Self {
        Self {
            id: row.id,
            rule_id: row.rule_id,
            run_type: match row.run_type.as_str() {
                "SCHEDULED" => TestRunType::Scheduled,
                "MANUAL" => TestRunType::Manual,
                "API" => TestRunType::Api,
                _ => TestRunType::Manual,
            },
            status: match row.status.as_str() {
                "RUNNING" => TestRunStatus::Running,
                "COMPLETED" => TestRunStatus::Completed,
                "FAILED" => TestRunStatus::Failed,
                "CANCELLED" => TestRunStatus::Cancelled,
                _ => TestRunStatus::Failed,
            },
            started_at: row.started_at,
            completed_at: row.completed_at,
            sample_size: row.sample_size,
            total_rows: row.total_rows,
            execution_results: row.execution_results,
            error_message: row.error_message,
            triggered_by: row.triggered_by,
            sampling_strategy: serde_json::from_value(row.sampling_strategy)
                .unwrap_or(SamplingStrategy::Full),
        }
    }
}
