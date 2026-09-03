use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// 原有模型定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityRule {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub rule_type: RuleType,
    pub target_table: String,
    pub target_column: Option<String>,
    pub parameters: serde_json::Value,
    pub severity: Severity,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RuleType {
    NotNull,
    Unique,
    Range,
    Pattern,
    Length,
    DataType,
}

impl RuleType {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuleType::NotNull => "not_null",
            RuleType::Unique => "unique",
            RuleType::Range => "range",
            RuleType::Pattern => "pattern",
            RuleType::Length => "length",
            RuleType::DataType => "data_type",
        }
    }

    pub fn from_string(s: &str) -> Self {
        match s {
            "not_null" => RuleType::NotNull,
            "unique" => RuleType::Unique,
            "range" => RuleType::Range,
            "pattern" => RuleType::Pattern,
            "length" => RuleType::Length,
            "data_type" => RuleType::DataType,
            _ => RuleType::NotNull,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Critical => "critical",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }

    pub fn from_string(s: &str) -> Self {
        match s {
            "critical" => Severity::Critical,
            "warning" => Severity::Warning,
            "info" => Severity::Info,
            _ => Severity::Warning,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityCheck {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub rule_id: i64,
    pub status: CheckStatus,
    #[serde(with = "common::serde_zuid")]
    pub records_checked: i64,
    #[serde(with = "common::serde_zuid")]
    pub records_failed: i64,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pending,
    Running,
    Passed,
    Failed,
}

impl CheckStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckStatus::Pending => "pending",
            CheckStatus::Running => "running",
            CheckStatus::Passed => "passed",
            CheckStatus::Failed => "failed",
        }
    }

    pub fn from_string(s: &str) -> Self {
        match s {
            "pending" => CheckStatus::Pending,
            "running" => CheckStatus::Running,
            "passed" => CheckStatus::Passed,
            "failed" => CheckStatus::Failed,
            _ => CheckStatus::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityReport {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub check_id: i64,
    pub overall_score: f64,
    pub metrics: QualityMetrics,
    pub failed_records: Vec<FailedRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityMetrics {
    pub completeness: f64,
    pub uniqueness: f64,
    pub validity: f64,
    pub consistency: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedRecord {
    pub record_id: String,
    pub column: String,
    pub value: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableProfile {
    pub table_name: String,
    #[serde(with = "common::serde_zuid")]
    pub row_count: i64,
    pub metrics: QualityMetrics,
    pub column_profiles: Vec<ColumnProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnProfile {
    pub column_name: String,
    pub data_type: String,
    #[serde(with = "common::serde_zuid")]
    pub null_count: i64,
    pub null_percentage: f64,
    #[serde(with = "common::serde_zuid")]
    pub unique_count: i64,
    pub unique_percentage: f64,
}

// 响应类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityRuleResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub rule_type: String,
    pub target_table: String,
    pub target_column: Option<String>,
    pub parameters: serde_json::Value,
    pub severity: String,
    pub created_at: DateTime<Utc>,
}

impl From<QualityRule> for QualityRuleResponse {
    fn from(rule: QualityRule) -> Self {
        Self {
            id: rule.id,
            name: rule.name,
            rule_type: rule.rule_type.as_str().to_string(),
            target_table: rule.target_table,
            target_column: rule.target_column,
            parameters: rule.parameters,
            severity: rule.severity.as_str().to_string(),
            created_at: rule.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityCheckResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub rule_id: i64,
    pub status: String,
    #[serde(with = "common::serde_zuid")]
    pub records_checked: i64,
    #[serde(with = "common::serde_zuid")]
    pub records_failed: i64,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl From<QualityCheck> for QualityCheckResponse {
    fn from(check: QualityCheck) -> Self {
        Self {
            id: check.id,
            rule_id: check.rule_id,
            status: check.status.as_str().to_string(),
            records_checked: check.records_checked,
            records_failed: check.records_failed,
            started_at: check.started_at,
            completed_at: check.completed_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityReportResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub check_id: i64,
    pub overall_score: f64,
    pub completeness: f64,
    pub uniqueness: f64,
    pub validity: f64,
    pub consistency: f64,
    pub failed_records: Vec<FailedRecord>,
}

impl From<QualityReport> for QualityReportResponse {
    fn from(report: QualityReport) -> Self {
        Self {
            id: report.id,
            check_id: report.check_id,
            overall_score: report.overall_score,
            completeness: report.metrics.completeness,
            uniqueness: report.metrics.uniqueness,
            validity: report.metrics.validity,
            consistency: report.metrics.consistency,
            failed_records: report.failed_records,
        }
    }
}

// 请求类型
#[derive(Debug, Deserialize)]
pub struct CreateRuleRequest {
    pub name: String,
    pub rule_type: String,
    pub target_table: String,
    pub target_column: Option<String>,
    pub parameters: Option<serde_json::Value>,
    pub severity: String,
}

#[derive(Debug, Deserialize)]
pub struct RunCheckRequest {
    #[serde(with = "common::serde_zuid")]
    pub rule_id: i64,
}

// 错误类型
#[derive(Debug, thiserror::Error)]
pub enum QualityError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Rule not found")]
    RuleNotFound,
    #[error("Invalid rule type: {0}")]
    InvalidRuleType(String),
    #[error("Invalid parameters: {0}")]
    InvalidParameters(String),
    #[error("Execution error: {0}")]
    Execution(String),
}

impl actix_web::ResponseError for QualityError {
    fn error_response(&self) -> actix_web::HttpResponse {
        use actix_web::HttpResponse;
        match self {
            QualityError::RuleNotFound => {
                HttpResponse::NotFound().json(serde_json::json!({"error": self.to_string()}))
            }
            _ => HttpResponse::BadRequest().json(serde_json::json!({"error": self.to_string()})),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rule_type_as_str() {
        assert_eq!(RuleType::NotNull.as_str(), "not_null");
        assert_eq!(RuleType::Unique.as_str(), "unique");
        assert_eq!(RuleType::Range.as_str(), "range");
    }

    #[test]
    fn test_severity_as_str() {
        assert_eq!(Severity::Critical.as_str(), "critical");
        assert_eq!(Severity::Warning.as_str(), "warning");
    }

    #[test]
    fn test_check_status_as_str() {
        assert_eq!(CheckStatus::Passed.as_str(), "passed");
        assert_eq!(CheckStatus::Failed.as_str(), "failed");
    }
}
