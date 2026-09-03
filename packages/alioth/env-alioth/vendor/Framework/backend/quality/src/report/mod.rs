use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};

use crate::dashboard::DashboardService;

pub mod excel_exporter;
pub mod pdf_exporter;

/// 报告时间范围
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReportPeriod {
    Last24Hours,
    Last7Days,
    Last30Days,
    Custom {
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    },
}

/// 报告参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportParams {
    pub period: ReportPeriod,
    #[serde(with = "common::serde_zuid::opt_seq")]
    pub collection_ids: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt_seq")]
    pub rule_ids: Option<Vec<i64>>,
    pub include_details: bool,
    pub include_recommendations: bool,
}

/// 报告摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSummary {
    pub overall_score: i32,
    #[serde(with = "common::serde_zuid")]
    pub total_rules: i64,
    #[serde(with = "common::serde_zuid")]
    pub active_rules: i64,
    #[serde(with = "common::serde_zuid")]
    pub total_executions: i64,
    #[serde(with = "common::serde_zuid")]
    pub failed_executions: i64,
    pub pass_rate: f64,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
}

/// 规则执行详情
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleExecutionDetail {
    #[serde(with = "common::serde_zuid")]
    pub rule_id: i64,
    pub rule_name: String,
    pub rule_type: String,
    pub severity: String,
    #[serde(with = "common::serde_zuid")]
    pub execution_count: i64,
    #[serde(with = "common::serde_zuid")]
    pub passed_count: i64,
    #[serde(with = "common::serde_zuid")]
    pub failed_count: i64,
    pub pass_rate: f64,
    pub last_executed: Option<DateTime<Utc>>,
}

/// 改进建议
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub priority: RecommendationPriority,
    pub category: String,
    pub title: String,
    pub description: String,
    pub affected_rules: Vec<String>,
    pub estimated_impact: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum RecommendationPriority {
    High,
    Medium,
    Low,
}

/// 质量报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityReport {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub generated_at: DateTime<Utc>,
    pub params: ReportParams,
    pub summary: ReportSummary,
    pub details: Vec<RuleExecutionDetail>,
    pub recommendations: Vec<Recommendation>,
}

/// 报告错误
#[derive(Debug)]
pub enum ReportError {
    Database(sqlx::Error),
    GenerationFailed(String),
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReportError::Database(e) => write!(f, "Database error: {}", e),
            ReportError::GenerationFailed(msg) => write!(f, "Report generation failed: {}", msg),
        }
    }
}

impl From<sqlx::Error> for ReportError {
    fn from(e: sqlx::Error) -> Self {
        ReportError::Database(e)
    }
}

/// 报告生成服务
pub struct QualityReportService {
    pool: PgPool,
}

impl QualityReportService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 生成报告
    pub async fn generate_report(
        &self,
        params: ReportParams,
    ) -> Result<QualityReport, ReportError> {
        let (period_start, period_end) = self.calculate_period(&params.period);

        let summary = self
            .generate_summary(&params, period_start, period_end)
            .await?;

        let details = if params.include_details {
            self.generate_details(&params, period_start, period_end)
                .await?
        } else {
            vec![]
        };

        let recommendations = if params.include_recommendations {
            self.generate_recommendations(&params, &details).await?
        } else {
            vec![]
        };

        Ok(QualityReport {
            id: 0,
            generated_at: Utc::now(),
            params,
            summary,
            details,
            recommendations,
        })
    }

    /// 计算时间范围
    fn calculate_period(&self, period: &ReportPeriod) -> (DateTime<Utc>, DateTime<Utc>) {
        let now = Utc::now();
        let end = now;

        let start = match period {
            ReportPeriod::Last24Hours => now - chrono::Duration::hours(24),
            ReportPeriod::Last7Days => now - chrono::Duration::days(7),
            ReportPeriod::Last30Days => now - chrono::Duration::days(30),
            ReportPeriod::Custom { start, .. } => *start,
        };

        (start, end)
    }

    /// 生成报告摘要
    async fn generate_summary(
        &self,
        _params: &ReportParams,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Result<ReportSummary, ReportError> {
        let dashboard = DashboardService::new(self.pool.clone());
        let dashboard_data = dashboard.get_dashboard_data().await?;

        let total_executions: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) 
            FROM quality_rule_executions 
            WHERE executed_at >= $1 AND executed_at <= $2
            "#,
        )
        .bind(period_start)
        .bind(period_end)
        .fetch_one(&self.pool)
        .await?;

        let failed_executions: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) 
            FROM quality_rule_executions 
            WHERE executed_at >= $1 AND executed_at <= $2 
            AND status = 'failed'
            "#,
        )
        .bind(period_start)
        .bind(period_end)
        .fetch_one(&self.pool)
        .await?;

        let pass_rate = if total_executions > 0 {
            ((total_executions - failed_executions) as f64 / total_executions as f64) * 100.0
        } else {
            100.0
        };

        Ok(ReportSummary {
            overall_score: dashboard_data.score.overall,
            total_rules: dashboard_data.total_rules,
            active_rules: dashboard_data.active_rules,
            total_executions,
            failed_executions,
            pass_rate,
            period_start,
            period_end,
        })
    }

    /// 生成详细数据
    async fn generate_details(
        &self,
        params: &ReportParams,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Result<Vec<RuleExecutionDetail>, ReportError> {
        let mut conditions = vec![
            "qre.executed_at >= $1".to_string(),
            "qre.executed_at <= $2".to_string(),
        ];
        let mut query_params: Vec<Box<dyn std::any::Any + Send>> =
            vec![Box::new(period_start), Box::new(period_end)];

        if let Some(collection_ids) = &params.collection_ids {
            if !collection_ids.is_empty() {
                conditions.push(format!(
                    "qr.collection_id = ANY(${})",
                    query_params.len() + 1
                ));
                query_params.push(Box::new(collection_ids.clone()));
            }
        }

        if let Some(rule_ids) = &params.rule_ids {
            if !rule_ids.is_empty() {
                conditions.push(format!("qr.id = ANY(${})", query_params.len() + 1));
                query_params.push(Box::new(rule_ids.clone()));
            }
        }

        let where_clause = conditions.join(" AND ");

        let sql = format!(
            r#"
            SELECT 
                qr.id as rule_id,
                qr.name as rule_name,
                qr.rule_type as rule_type,
                qr.severity as severity,
                COUNT(qre.id) as execution_count,
                SUM(CASE WHEN qre.status = 'passed' THEN 1 ELSE 0 END) as passed_count,
                SUM(CASE WHEN qre.status = 'failed' THEN 1 ELSE 0 END) as failed_count,
                ROUND(
                    SUM(CASE WHEN qre.status = 'passed' THEN 1 ELSE 0 END) * 100.0 / COUNT(qre.id),
                    2
                ) as pass_rate,
                MAX(qre.executed_at) as last_executed
            FROM isahl_meta.quality_rules qr
            LEFT JOIN quality_rule_executions qre ON qr.id = qre.rule_id
            WHERE {}
            GROUP BY qr.id, qr.name, qr.rule_type, qr.severity
            ORDER BY execution_count DESC
            "#,
            where_clause
        );

        // 执行查询
        let details: Vec<RuleExecutionDetail> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(period_start)
            .bind(period_end)
            .fetch_all(&self.pool)
            .await?;

        Ok(details)
    }

    /// 生成改进建议
    async fn generate_recommendations(
        &self,
        _params: &ReportParams,
        details: &[RuleExecutionDetail],
    ) -> Result<Vec<Recommendation>, ReportError> {
        let mut recommendations = vec![];

        // 1. 检查低通过率的规则
        let low_pass_rules: Vec<&RuleExecutionDetail> = details
            .iter()
            .filter(|d| d.pass_rate < 50.0 && d.execution_count > 5)
            .collect();

        if !low_pass_rules.is_empty() {
            recommendations.push(Recommendation {
                priority: RecommendationPriority::High,
                category: "规则优化".to_string(),
                title: "修复低通过率规则".to_string(),
                description: format!(
                    "发现 {} 个规则通过率低于 50%，建议检查规则配置或数据质量",
                    low_pass_rules.len()
                ),
                affected_rules: low_pass_rules.iter().map(|r| r.rule_name.clone()).collect(),
                estimated_impact: "可提升整体质量评分 10-20 分".to_string(),
            });
        }

        // 2. 检查未使用的规则
        let unused_rules: Vec<&RuleExecutionDetail> =
            details.iter().filter(|d| d.execution_count == 0).collect();

        if !unused_rules.is_empty() {
            recommendations.push(Recommendation {
                priority: RecommendationPriority::Medium,
                category: "规则管理".to_string(),
                title: "清理未使用的规则".to_string(),
                description: format!(
                    "发现 {} 个规则在报告期内未执行，考虑删除或启用",
                    unused_rules.len()
                ),
                affected_rules: unused_rules.iter().map(|r| r.rule_name.clone()).collect(),
                estimated_impact: "简化规则库，提高管理效率".to_string(),
            });
        }

        // 3. 检查严重级别失败
        let critical_failures: Vec<&RuleExecutionDetail> = details
            .iter()
            .filter(|d| d.severity == "CRITICAL" && d.failed_count > 0)
            .collect();

        if !critical_failures.is_empty() {
            recommendations.push(Recommendation {
                priority: RecommendationPriority::High,
                category: "紧急修复".to_string(),
                title: "处理严重级别规则失败".to_string(),
                description: format!(
                    "发现 {} 个严重级别规则存在失败，需要立即处理",
                    critical_failures.len()
                ),
                affected_rules: critical_failures
                    .iter()
                    .map(|r| r.rule_name.clone())
                    .collect(),
                estimated_impact: "避免数据质量严重问题".to_string(),
            });
        }

        // 4. 通用建议
        recommendations.push(Recommendation {
            priority: RecommendationPriority::Low,
            category: "最佳实践".to_string(),
            title: "定期审查规则配置".to_string(),
            description: "建议每月审查一次规则配置，确保规则与业务需求保持一致".to_string(),
            affected_rules: vec![],
            estimated_impact: "保持数据质量持续改进".to_string(),
        });

        Ok(recommendations)
    }
}

// SQLx 结果映射
impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for RuleExecutionDetail {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;

        Ok(RuleExecutionDetail {
            rule_id: row.try_get("rule_id")?,
            rule_name: row.try_get("rule_name")?,
            rule_type: row.try_get("rule_type")?,
            severity: row.try_get("severity")?,
            execution_count: row.try_get("execution_count")?,
            passed_count: row.try_get("passed_count")?,
            failed_count: row.try_get("failed_count")?,
            pass_rate: row.try_get("pass_rate")?,
            last_executed: row.try_get("last_executed")?,
        })
    }
}
