use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

use crate::rules::{RuleExecutionResult, RuleSeverity};

/// 质量评分
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityScore {
    pub overall: i32,
    pub by_category: Vec<CategoryScore>,
    pub trend: Vec<TrendPoint>,
}

/// 分类评分
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryScore {
    pub category: String,
    pub score: i32,
    pub weight: f64,
}

/// 趋势点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendPoint {
    pub date: DateTime<Utc>,
    pub score: i32,
    #[serde(with = "common::serde_zuid")]
    pub passed_count: i64,
    #[serde(with = "common::serde_zuid")]
    pub failed_count: i64,
}

/// 失败分布
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureDistribution {
    pub by_rule_type: Vec<RuleTypeFailure>,
    pub by_severity: Vec<SeverityFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleTypeFailure {
    pub rule_type: String,
    #[serde(with = "common::serde_zuid")]
    pub count: i64,
    pub percentage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeverityFailure {
    pub severity: String,
    #[serde(with = "common::serde_zuid")]
    pub count: i64,
    pub percentage: f64,
}

/// 热门问题
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopIssue {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub entity_type: String,
    pub entity_name: String,
    pub rule_name: String,
    pub severity: String,
    #[serde(with = "common::serde_zuid")]
    pub failure_count: i64,
    pub last_failed_at: Option<DateTime<Utc>>,
}

/// 仪表板数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardData {
    pub score: QualityScore,
    #[serde(with = "common::serde_zuid")]
    pub total_rules: i64,
    #[serde(with = "common::serde_zuid")]
    pub active_rules: i64,
    #[serde(with = "common::serde_zuid")]
    pub total_executions_24h: i64,
    #[serde(with = "common::serde_zuid")]
    pub failed_executions_24h: i64,
}

/// 评分计算器
pub struct QualityScoreCalculator;

impl QualityScoreCalculator {
    /// 计算整体评分
    pub fn calculate_overall_score(rule_results: &[RuleExecutionResult]) -> QualityScore {
        if rule_results.is_empty() {
            return QualityScore {
                overall: 100,
                by_category: vec![],
                trend: vec![],
            };
        }

        let total_weight: f64 = rule_results
            .iter()
            .map(|r| Self::severity_weight(&r.severity))
            .sum();

        let weighted_score: f64 = rule_results
            .iter()
            .map(|r| r.pass_percentage * Self::severity_weight(&r.severity))
            .sum();

        let overall = if total_weight > 0.0 {
            ((weighted_score / total_weight) as i32).clamp(0, 100)
        } else {
            100
        };

        QualityScore {
            overall,
            by_category: Self::calculate_by_category(rule_results),
            trend: vec![], // 趋势需要历史数据
        }
    }

    /// 根据严重程度获取权重
    fn severity_weight(severity: &RuleSeverity) -> f64 {
        match severity {
            RuleSeverity::Critical => 4.0,
            RuleSeverity::Error => 3.0,
            RuleSeverity::Warning => 2.0,
            RuleSeverity::Info => 1.0,
        }
    }

    /// 按分类计算评分
    fn calculate_by_category(rule_results: &[RuleExecutionResult]) -> Vec<CategoryScore> {
        use std::collections::HashMap;

        let mut category_scores: HashMap<String, (f64, f64)> = HashMap::new();

        for result in rule_results {
            let category = Self::categorize_rule(&result.rule_name);
            let weight = Self::severity_weight(&result.severity);
            let entry = category_scores.entry(category).or_insert((0.0, 0.0));
            entry.0 += result.pass_percentage * weight;
            entry.1 += weight;
        }

        category_scores
            .into_iter()
            .map(|(category, (weighted_sum, total_weight))| CategoryScore {
                category,
                score: if total_weight > 0.0 {
                    ((weighted_sum / total_weight) as i32).clamp(0, 100)
                } else {
                    100
                },
                weight: total_weight,
            })
            .collect()
    }

    /// 对规则分类
    fn categorize_rule(rule_name: &str) -> String {
        let name_lower = rule_name.to_lowercase();
        if name_lower.contains("null") || name_lower.contains("empty") {
            "完整性".to_string()
        } else if name_lower.contains("format") || name_lower.contains("regex") {
            "格式".to_string()
        } else if name_lower.contains("range")
            || name_lower.contains("min")
            || name_lower.contains("max")
        {
            "范围".to_string()
        } else if name_lower.contains("unique") || name_lower.contains("duplicate") {
            "唯一性".to_string()
        } else {
            "其他".to_string()
        }
    }
}

/// 仪表板服务
pub struct DashboardService {
    pool: PgPool,
}

impl DashboardService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 获取仪表板数据
    pub async fn get_dashboard_data(&self) -> Result<DashboardData, sqlx::Error> {
        let score = self.calculate_current_score().await?;
        let total_rules = self.count_total_rules().await?;
        let active_rules = self.count_active_rules().await?;
        let (total_exec, failed_exec) = self.count_executions_24h().await?;

        Ok(DashboardData {
            score,
            total_rules,
            active_rules,
            total_executions_24h: total_exec,
            failed_executions_24h: failed_exec,
        })
    }

    /// 计算当前评分
    async fn calculate_current_score(&self) -> Result<QualityScore, sqlx::Error> {
        // 获取最近的规则执行结果
        let results: Vec<RuleExecutionResult> = sqlx::query_as(
            r#"
            SELECT 
                qre.id,
                qr.name as rule_name,
                qr.rule_type,
                qr.severity as "severity: RuleSeverity",
                qre.status,
                qre.total_rows,
                qre.passed_rows,
                qre.failed_rows,
                qre.pass_percentage,
                qre.error_message,
                qre.executed_at,
                qre.duration_ms,
                qre.sample_failed_values
            FROM quality_rule_executions qre
            JOIN isahl_meta.quality_rules qr ON qre.rule_id = qr.id
            WHERE qre.executed_at > NOW() - INTERVAL '24 hours'
            ORDER BY qre.executed_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(QualityScoreCalculator::calculate_overall_score(&results))
    }

    /// 统计总规则数
    async fn count_total_rules(&self) -> Result<i64, sqlx::Error> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM isahl_meta.quality_rules")
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }

    /// 统计活跃规则数
    async fn count_active_rules(&self) -> Result<i64, sqlx::Error> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM isahl_meta.quality_rules WHERE enabled = true",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    /// 统计24小时内执行次数
    async fn count_executions_24h(&self) -> Result<(i64, i64), sqlx::Error> {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM quality_rule_executions WHERE executed_at > NOW() - INTERVAL '24 hours'"
        )
        .fetch_one(&self.pool)
        .await?;

        let failed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM quality_rule_executions WHERE executed_at > NOW() - INTERVAL '24 hours' AND status = 'failed'"
        )
        .fetch_one(&self.pool)
        .await?;

        Ok((total, failed))
    }

    /// 获取失败分布
    pub async fn get_failure_distribution(&self) -> Result<FailureDistribution, sqlx::Error> {
        let by_rule_type: Vec<RuleTypeFailure> = sqlx::query_as(
            r#"
            SELECT 
                qr.rule_type as "rule_type!: String",
                COUNT(*) as "count!: i64",
                ROUND(COUNT(*) * 100.0 / SUM(COUNT(*)) OVER (), 2) as "percentage!: f64"
            FROM quality_rule_executions qre
            JOIN isahl_meta.quality_rules qr ON qre.rule_id = qr.id
            WHERE qre.status = 'failed' 
            AND qre.executed_at > NOW() - INTERVAL '7 days'
            GROUP BY qr.rule_type
            ORDER BY COUNT(*) DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let by_severity: Vec<SeverityFailure> = sqlx::query_as(
            r#"
            SELECT 
                qr.severity as "severity!: String",
                COUNT(*) as "count!: i64",
                ROUND(COUNT(*) * 100.0 / SUM(COUNT(*)) OVER (), 2) as "percentage!: f64"
            FROM quality_rule_executions qre
            JOIN isahl_meta.quality_rules qr ON qre.rule_id = qr.id
            WHERE qre.status = 'failed' 
            AND qre.executed_at > NOW() - INTERVAL '7 days'
            GROUP BY qr.severity
            ORDER BY COUNT(*) DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(FailureDistribution {
            by_rule_type,
            by_severity,
        })
    }

    /// 获取质量趋势
    pub async fn get_quality_trend(&self, days: i32) -> Result<Vec<TrendPoint>, sqlx::Error> {
        let trend: Vec<TrendPoint> = sqlx::query_as(
            r#"
            SELECT 
                DATE(qre.executed_at) as "date!: DateTime<Utc>",
                ROUND(AVG(qre.pass_percentage), 0) as "score!: i32",
                SUM(CASE WHEN qre.status = 'passed' THEN 1 ELSE 0 END) as "passed_count!: i64",
                SUM(CASE WHEN qre.status = 'failed' THEN 1 ELSE 0 END) as "failed_count!: i64"
            FROM quality_rule_executions qre
            WHERE qre.executed_at > NOW() - INTERVAL '$1 days'
            GROUP BY DATE(qre.executed_at)
            ORDER BY DATE(qre.executed_at)
            "#,
        )
        .bind(days)
        .fetch_all(&self.pool)
        .await?;

        Ok(trend)
    }

    /// 获取热门问题
    pub async fn get_top_issues(&self, limit: i64) -> Result<Vec<TopIssue>, sqlx::Error> {
        let issues: Vec<TopIssue> = sqlx::query_as(
            r#"
            SELECT 
                qr.id,
                CASE 
                    WHEN qr.field_id IS NOT NULL THEN 'field'
                    WHEN qr.collection_id IS NOT NULL THEN 'collection'
                    ELSE 'global'
                END as "entity_type!: String",
                COALESCE(f.name, c.name, '全局规则') as "entity_name!: String",
                qr.name as "rule_name!: String",
                qr.severity as "severity!: String",
                COUNT(qre.id) as "failure_count!: i64",
                MAX(qre.executed_at) as "last_failed_at: DateTime<Utc>"
            FROM isahl_meta.quality_rules qr
            JOIN quality_rule_executions qre ON qr.id = qre.rule_id
            LEFT JOIN meta_field f ON qr.field_id = f.id
            LEFT JOIN meta_collections c ON qr.collection_id = c.table_name
            WHERE qre.status = 'failed'
            AND qre.executed_at > NOW() - INTERVAL '7 days'
            GROUP BY qr.id, f.name, c.name, qr.name, qr.severity
            ORDER BY COUNT(qre.id) DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(issues)
    }
}

// SQLx 查询结果映射
impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for RuleTypeFailure {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        Ok(RuleTypeFailure {
            rule_type: row.try_get("rule_type")?,
            count: row.try_get("count!")?,
            percentage: row.try_get("percentage!")?,
        })
    }
}

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for SeverityFailure {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        Ok(SeverityFailure {
            severity: row.try_get("severity!")?,
            count: row.try_get("count!")?,
            percentage: row.try_get("percentage!")?,
        })
    }
}

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for TopIssue {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        Ok(TopIssue {
            id: row.try_get("id")?,
            entity_type: row.try_get("entity_type!")?,
            entity_name: row.try_get("entity_name!")?,
            rule_name: row.try_get("rule_name!")?,
            severity: row.try_get("severity!")?,
            failure_count: row.try_get("failure_count!")?,
            last_failed_at: row.try_get("last_failed_at")?,
        })
    }
}

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for TrendPoint {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        Ok(TrendPoint {
            date: row.try_get("date!")?,
            score: row.try_get("score!")?,
            passed_count: row.try_get("passed_count!")?,
            failed_count: row.try_get("failed_count!")?,
        })
    }
}
