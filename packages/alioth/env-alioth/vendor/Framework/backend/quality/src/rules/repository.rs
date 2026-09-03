use sqlx::{AssertSqlSafe, PgPool};

use crate::rules::types::{QualityRule, RuleExecutionResult, RuleSeverity, RuleStatus, RuleType};

pub struct RuleRepository {
    db_pool: PgPool,
}

impl RuleRepository {
    pub fn new(db_pool: PgPool) -> Self {
        Self { db_pool }
    }

    async fn generate_id(&self) -> Result<i64, sqlx::Error> {
        let id: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) + 1 FROM isahl_meta.quality_rules")
                .fetch_one(&self.db_pool)
                .await?;
        Ok(id)
    }

    /// 创建规则
    pub async fn create(&self, rule: &QualityRule) -> Result<i64, sqlx::Error> {
        let id = self.generate_id().await?;
        let sql = r#"
            INSERT INTO isahl_meta.quality_rules (
                id, name, description, rule_type, enabled, severity, parameters,
                collection_id, field_id, created_by, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING id
        "#;

        let result_id = sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql))
            .bind(id)
            .bind(&rule.name)
            .bind(&rule.description)
            .bind(rule.rule_type.as_str())
            .bind(rule.enabled)
            .bind(rule.severity.as_str())
            .bind(&rule.parameters)
            .bind(rule.collection_id)
            .bind(rule.field_id)
            .bind(rule.created_by)
            .bind(rule.created_at)
            .bind(rule.updated_at)
            .fetch_one(&self.db_pool)
            .await?;

        Ok(result_id)
    }

    /// 更新规则
    pub async fn update(
        &self,
        rule_id: i64,
        updates: &UpdateRuleRequest,
    ) -> Result<QualityRule, sqlx::Error> {
        let sql = r#"
            UPDATE isahl_meta.quality_rules SET
                name = COALESCE($1, name),
                description = COALESCE($2, description),
                rule_type = COALESCE($3, rule_type),
                enabled = COALESCE($4, enabled),
                severity = COALESCE($5, severity),
                parameters = COALESCE($6, parameters),
                collection_id = COALESCE($7, collection_id),
                field_id = COALESCE($8, field_id),
                updated_at = NOW()
            WHERE id = $9
            RETURNING id, name, description, rule_type, enabled, severity, parameters,
                      collection_id, field_id, created_by, created_at, updated_at
        "#;

        let row = sqlx::query_as::<_, QualityRuleRow>(AssertSqlSafe(sql))
            .bind(updates.name.as_ref())
            .bind(updates.description.as_ref())
            .bind(updates.rule_type.as_ref().map(|t| t.as_str()))
            .bind(updates.enabled)
            .bind(updates.severity.as_ref().map(|s| s.as_str()))
            .bind(updates.parameters.as_ref())
            .bind(updates.collection_id)
            .bind(updates.field_id)
            .bind(rule_id)
            .fetch_one(&self.db_pool)
            .await?;

        Ok(row.into())
    }

    /// 删除规则
    pub async fn delete(&self, id: i64) -> Result<(), sqlx::Error> {
        let sql = "DELETE FROM isahl_meta.quality_rules WHERE id = $1";

        sqlx::query(AssertSqlSafe(sql))
            .bind(id)
            .execute(&self.db_pool)
            .await?;

        Ok(())
    }

    /// 获取单个规则
    pub async fn get_by_id(&self, id: i64) -> Result<Option<QualityRule>, sqlx::Error> {
        let sql = r#"
            SELECT id, name, description, rule_type, enabled, severity, parameters,
                   collection_id, field_id, created_by, created_at, updated_at
            FROM isahl_meta.quality_rules
            WHERE id = $1
        "#;

        let row = sqlx::query_as::<_, QualityRuleRow>(AssertSqlSafe(sql))
            .bind(id)
            .fetch_optional(&self.db_pool)
            .await?;

        Ok(row.map(|r| r.into()))
    }

    /// 获取集合的所有规则
    pub async fn get_by_collection(
        &self,
        collection_id: i64,
    ) -> Result<Vec<QualityRule>, sqlx::Error> {
        let sql = r#"
            SELECT id, name, description, rule_type, enabled, severity, parameters,
                   collection_id, field_id, created_by, created_at, updated_at
            FROM isahl_meta.quality_rules
            WHERE collection_id = $1
            ORDER BY created_at DESC
        "#;

        let rows: Vec<QualityRuleRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(collection_id)
            .fetch_all(&self.db_pool)
            .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// 获取字段的所有规则
    pub async fn get_by_field(&self, field_id: i64) -> Result<Vec<QualityRule>, sqlx::Error> {
        let sql = r#"
            SELECT id, name, description, rule_type, enabled, severity, parameters,
                   collection_id, field_id, created_by, created_at, updated_at
            FROM isahl_meta.quality_rules
            WHERE field_id = $1
            ORDER BY created_at DESC
        "#;

        let rows: Vec<QualityRuleRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(field_id)
            .fetch_all(&self.db_pool)
            .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// 获取启用的规则
    pub async fn get_enabled_rules(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
    ) -> Result<Vec<QualityRule>, sqlx::Error> {
        let mut sql = String::from(
            "SELECT id, name, description, rule_type, enabled, severity, parameters, \
             collection_id, field_id, created_by, created_at, updated_at \
             FROM isahl_meta.quality_rules \
             WHERE enabled = true",
        );

        if collection_id.is_some() {
            sql.push_str(" AND (collection_id = $1 OR $1 IS NULL)");
        } else {
            sql.push_str(" AND $1 IS NULL");
        }

        if field_id.is_some() {
            sql.push_str(" AND (field_id = $2 OR $2 IS NULL)");
        } else {
            sql.push_str(" AND $2 IS NULL");
        }

        sql.push_str(" ORDER BY created_at DESC");

        let rows: Vec<QualityRuleRow> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(collection_id)
            .bind(field_id)
            .fetch_all(&self.db_pool)
            .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// 保存执行结果
    pub async fn save_execution(&self, result: &RuleExecutionResult) -> Result<i64, sqlx::Error> {
        let id: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) + 1 FROM quality_rule_executions")
                .fetch_one(&self.db_pool)
                .await?;

        let sql = r#"
            INSERT INTO quality_rule_executions (
                id, rule_id, collection_id, field_id, status, executed_at, duration_ms,
                total_rows, passed_rows, failed_rows, pass_percentage,
                failed_samples, error_message
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            RETURNING id
        "#;

        let result_id = sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql))
            .bind(id)
            .bind(result.rule_id)
            .bind(None::<i64>) // collection_id
            .bind(None::<i64>) // field_id
            .bind(result.status.as_str())
            .bind(result.executed_at)
            .bind(result.duration_ms)
            .bind(result.total_rows)
            .bind(result.passed_rows)
            .bind(result.failed_rows)
            .bind(result.pass_percentage)
            .bind(serde_json::to_value(&result.failed_samples).unwrap_or_default())
            .bind(result.error_message.as_ref())
            .fetch_one(&self.db_pool)
            .await?;

        Ok(result_id)
    }

    /// 获取规则执行历史
    pub async fn get_execution_history(
        &self,
        rule_id: i64,
        limit: i64,
    ) -> Result<Vec<RuleExecutionResult>, sqlx::Error> {
        let sql = r#"
            SELECT id, rule_id, status, executed_at, duration_ms,
                   total_rows, passed_rows, failed_rows, pass_percentage,
                   failed_samples, error_message
            FROM quality_rule_executions
            WHERE rule_id = $1
            ORDER BY executed_at DESC
            LIMIT $2
        "#;

        let rows: Vec<RuleExecutionRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(rule_id)
            .bind(limit)
            .fetch_all(&self.db_pool)
            .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// 获取最近执行结果
    pub async fn get_latest_execution(
        &self,
        rule_id: i64,
    ) -> Result<Option<RuleExecutionResult>, sqlx::Error> {
        let sql = r#"
            SELECT id, rule_id, status, executed_at, duration_ms,
                   total_rows, passed_rows, failed_rows, pass_percentage,
                   failed_samples, error_message
            FROM quality_rule_executions
            WHERE rule_id = $1
            ORDER BY executed_at DESC
            LIMIT 1
        "#;

        let row = sqlx::query_as::<_, RuleExecutionRow>(AssertSqlSafe(sql))
            .bind(rule_id)
            .fetch_optional(&self.db_pool)
            .await?;

        Ok(row.map(|r| r.into()))
    }

    /// 切换规则启用状态
    pub async fn toggle_enabled(&self, rule_id: i64, enabled: bool) -> Result<(), sqlx::Error> {
        let sql = r#"
            UPDATE isahl_meta.quality_rules
            SET enabled = $1, updated_at = NOW()
            WHERE id = $2
        "#;

        sqlx::query(AssertSqlSafe(sql))
            .bind(enabled)
            .bind(rule_id)
            .execute(&self.db_pool)
            .await?;

        Ok(())
    }
}

// 更新请求
#[derive(Debug, Clone, Default)]
pub struct UpdateRuleRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub rule_type: Option<RuleType>,
    pub enabled: Option<bool>,
    pub severity: Option<RuleSeverity>,
    pub parameters: Option<serde_json::Value>,
    pub collection_id: Option<Option<i64>>,
    pub field_id: Option<Option<i64>>,
}

// 数据库行结构
#[derive(sqlx::FromRow)]
struct QualityRuleRow {
    id: i64,
    name: String,
    description: Option<String>,
    rule_type: String,
    enabled: bool,
    severity: String,
    parameters: serde_json::Value,
    collection_id: Option<i64>,
    field_id: Option<i64>,
    created_by: Option<i64>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<QualityRuleRow> for QualityRule {
    fn from(row: QualityRuleRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            rule_type: row.rule_type.parse().unwrap_or(RuleType::NotNull),
            enabled: row.enabled,
            severity: row.severity.parse().unwrap_or(RuleSeverity::Error),
            parameters: row.parameters,
            collection_id: row.collection_id,
            field_id: row.field_id,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct RuleExecutionRow {
    id: i64,
    rule_id: i64,
    status: String,
    executed_at: chrono::DateTime<chrono::Utc>,
    duration_ms: i64,
    total_rows: i64,
    passed_rows: i64,
    failed_rows: i64,
    pass_percentage: f64,
    failed_samples: serde_json::Value,
    error_message: Option<String>,
}

impl From<RuleExecutionRow> for RuleExecutionResult {
    fn from(row: RuleExecutionRow) -> Self {
        Self {
            id: row.id,
            rule_id: row.rule_id,
            rule_name: String::new(),
            rule_type: RuleType::NotNull,  // 需要从规则表获取
            severity: RuleSeverity::Error, // 默认为 Error 级别
            status: match row.status.as_str() {
                "PASSED" => RuleStatus::Passed,
                "FAILED" => RuleStatus::Failed,
                "ERRORED" => RuleStatus::Errored,
                "SKIPPED" => RuleStatus::Skipped,
                _ => RuleStatus::Failed,
            },
            executed_at: row.executed_at,
            duration_ms: row.duration_ms,
            total_rows: row.total_rows,
            passed_rows: row.passed_rows,
            failed_rows: row.failed_rows,
            pass_percentage: row.pass_percentage,
            failed_samples: serde_json::from_value(row.failed_samples).unwrap_or_default(),
            error_message: row.error_message,
        }
    }
}
