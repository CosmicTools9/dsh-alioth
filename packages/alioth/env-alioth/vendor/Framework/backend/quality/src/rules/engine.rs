use regex::Regex;
use sqlx::{AssertSqlSafe, PgPool};

use crate::rules::types::{
    FailedRowSample, RuleExecutionResult, RuleSeverity, RuleStatus, RuleType,
};

pub struct RuleEngine {
    db_pool: PgPool,
}

impl RuleEngine {
    pub fn new(db_pool: PgPool) -> Self {
        Self { db_pool }
    }

    /// 执行单个规则
    pub async fn execute_rule(
        &self,
        rule_id: i64,
        sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        use std::time::Instant;

        let start_time = Instant::now();

        // 获取规则详情
        let rule = self.get_rule(rule_id).await?;

        let result = match rule.rule_type {
            RuleType::NotNull => {
                self.check_not_null(
                    rule.collection_id,
                    rule.field_id,
                    &rule.parameters,
                    sample_limit,
                )
                .await
            }
            RuleType::Unique => {
                self.check_unique(
                    rule.collection_id,
                    rule.field_id,
                    &rule.parameters,
                    sample_limit,
                )
                .await
            }
            RuleType::Range | RuleType::RangeInclusive => {
                let inclusive = matches!(rule.rule_type, RuleType::RangeInclusive);
                self.check_range(
                    rule.collection_id,
                    rule.field_id,
                    &rule.parameters,
                    inclusive,
                    sample_limit,
                )
                .await
            }
            RuleType::GreaterThan => {
                self.check_greater_than(
                    rule.collection_id,
                    rule.field_id,
                    &rule.parameters,
                    sample_limit,
                )
                .await
            }
            RuleType::LessThan => {
                self.check_less_than(
                    rule.collection_id,
                    rule.field_id,
                    &rule.parameters,
                    sample_limit,
                )
                .await
            }
            RuleType::EqualTo => {
                self.check_equal_to(
                    rule.collection_id,
                    rule.field_id,
                    &rule.parameters,
                    sample_limit,
                )
                .await
            }
            RuleType::LengthMin => {
                self.check_length_min(
                    rule.collection_id,
                    rule.field_id,
                    &rule.parameters,
                    sample_limit,
                )
                .await
            }
            RuleType::LengthMax => {
                self.check_length_max(
                    rule.collection_id,
                    rule.field_id,
                    &rule.parameters,
                    sample_limit,
                )
                .await
            }
            RuleType::LengthRange => {
                self.check_length_range(
                    rule.collection_id,
                    rule.field_id,
                    &rule.parameters,
                    sample_limit,
                )
                .await
            }
            RuleType::RegexMatch => {
                self.check_regex(
                    rule.collection_id,
                    rule.field_id,
                    &rule.parameters,
                    sample_limit,
                )
                .await
            }
            RuleType::EmailFormat => {
                self.check_email_format(rule.collection_id, rule.field_id, sample_limit)
                    .await
            }
            RuleType::UrlFormat => {
                self.check_url_format(rule.collection_id, rule.field_id, sample_limit)
                    .await
            }
            RuleType::UuidFormat => {
                self.check_uuid_format(rule.collection_id, rule.field_id, sample_limit)
                    .await
            }
            RuleType::Enum => {
                self.check_enum(
                    rule.collection_id,
                    rule.field_id,
                    &rule.parameters,
                    sample_limit,
                )
                .await
            }
            RuleType::NotIn => {
                self.check_not_in(
                    rule.collection_id,
                    rule.field_id,
                    &rule.parameters,
                    sample_limit,
                )
                .await
            }
            RuleType::IsNumeric => {
                self.check_is_numeric(rule.collection_id, rule.field_id, sample_limit)
                    .await
            }
            RuleType::IsInteger => {
                self.check_is_integer(rule.collection_id, rule.field_id, sample_limit)
                    .await
            }
            RuleType::IsBoolean => {
                self.check_is_boolean(rule.collection_id, rule.field_id, sample_limit)
                    .await
            }
            RuleType::IsDate => {
                self.check_is_date(rule.collection_id, rule.field_id, sample_limit)
                    .await
            }
            RuleType::IsTimestamp => {
                self.check_is_timestamp(rule.collection_id, rule.field_id, sample_limit)
                    .await
            }
            RuleType::NullPercentage => {
                self.check_null_percentage(rule.collection_id, rule.field_id, &rule.parameters)
                    .await
            }
            RuleType::CardinalityRatio => {
                self.check_cardinality_ratio(rule.collection_id, rule.field_id, &rule.parameters)
                    .await
            }
            RuleType::CustomSql => {
                self.check_custom_sql(
                    rule.collection_id,
                    rule.field_id,
                    &rule.parameters,
                    sample_limit,
                )
                .await
            }
        };

        let duration = start_time.elapsed().as_millis() as i64;

        match result {
            Ok(mut exec_result) => {
                exec_result.id = 0;
                exec_result.rule_id = rule.id;
                exec_result.rule_name = rule.name;
                exec_result.rule_type = rule.rule_type;
                exec_result.executed_at = chrono::Utc::now();
                exec_result.duration_ms = duration;
                Ok(exec_result)
            }
            Err(e) => {
                // 返回错误状态的结果
                Ok(RuleExecutionResult {
                    id: 0,
                    rule_id: rule.id,
                    rule_name: rule.name.clone(),
                    rule_type: rule.rule_type,
                    severity: rule.severity,
                    status: RuleStatus::Errored,
                    executed_at: chrono::Utc::now(),
                    duration_ms: duration,
                    total_rows: 0,
                    passed_rows: 0,
                    failed_rows: 0,
                    pass_percentage: 0.0,
                    failed_samples: vec![],
                    error_message: Some(e.to_string()),
                })
            }
        }
    }

    /// 获取规则详情（简化实现）
    async fn get_rule(
        &self,
        rule_id: i64,
    ) -> Result<crate::rules::types::QualityRule, crate::models::QualityError> {
        let sql = r#"
            SELECT id, name, description, rule_type, enabled, severity, parameters,
                   collection_id, field_id, created_by, created_at, updated_at
            FROM isahl_meta.quality_rules
            WHERE id = $1
        "#;

        let row = sqlx::query_as::<
            _,
            (
                i64,
                String,
                Option<String>,
                String,
                bool,
                String,
                serde_json::Value,
                Option<i64>,
                Option<i64>,
                Option<i64>,
                chrono::DateTime<chrono::Utc>,
                chrono::DateTime<chrono::Utc>,
            ),
        >(sql)
        .bind(rule_id)
        .fetch_one(&self.db_pool)
        .await
        .map_err(crate::models::QualityError::Database)?;

        Ok(crate::rules::types::QualityRule {
            id: row.0,
            name: row.1,
            description: row.2,
            rule_type: row
                .3
                .parse()
                .map_err(|_| crate::models::QualityError::InvalidRuleType(row.3.clone()))?,
            enabled: row.4,
            severity: row
                .5
                .parse()
                .map_err(|_| crate::models::QualityError::InvalidParameters(row.5.clone()))?,
            parameters: row.6,
            collection_id: row.7,
            field_id: row.8,
            created_by: row.9,
            created_at: row.10,
            updated_at: row.11,
        })
    }

    /// 解析字段信息
    async fn resolve_field_info(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
    ) -> Result<(String, String, String), crate::models::QualityError> {
        if let Some(fid) = field_id {
            // 通过 field_id 获取字段信息
            let row: (String, String, String) = sqlx::query_as(
                "SELECT schema_name, table_name, field_name FROM fields_view WHERE id = $1",
            )
            .bind(fid)
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;
            Ok(row)
        } else if let Some(cid) = collection_id {
            // 通过 collection_id 获取集合信息（用于表级规则）
            let row: (String, String) = sqlx::query_as(
                "SELECT schema_name, name FROM meta_collections WHERE table_name = $1",
            )
            .bind(cid)
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;
            Ok((row.0, row.1, String::new()))
        } else {
            Err(crate::models::QualityError::InvalidParameters(
                "Either collection_id or field_id must be provided".into(),
            ))
        }
    }

    // ==================== 规则检查实现 ====================

    /// 非空检查
    async fn check_not_null(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        _parameters: &serde_json::Value,
        sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let (schema, table, field) = self.resolve_field_info(collection_id, field_id).await?;

        let sql = format!(
            "SELECT COUNT(*) as total, \
             COUNT(\"{}\") as non_null \
             FROM \"{}\".\"{}\"",
            field, schema, table
        );

        let row: (i64, i64) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        let total_rows = row.0;
        let passed_rows = row.1;
        let failed_rows = total_rows - passed_rows;
        let pass_percentage = if total_rows > 0 {
            (passed_rows as f64 / total_rows as f64) * 100.0
        } else {
            100.0
        };

        // 获取失败样本
        let failed_samples = if failed_rows > 0 {
            self.get_failed_samples(
                &schema,
                &table,
                &field,
                format!("\"{}\" IS NULL", field),
                "Value is NULL",
                sample_limit,
            )
            .await?
        } else {
            vec![]
        };

        Ok(RuleExecutionResult {
            id: 0,
            rule_id: 0,
            rule_name: String::new(),
            rule_type: RuleType::NotNull,
            severity: RuleSeverity::Error,
            status: if failed_rows == 0 {
                RuleStatus::Passed
            } else {
                RuleStatus::Failed
            },
            executed_at: chrono::Utc::now(),
            duration_ms: 0,
            total_rows,
            passed_rows,
            failed_rows,
            pass_percentage,
            failed_samples,
            error_message: None,
        })
    }

    /// 唯一性检查
    async fn check_unique(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        _parameters: &serde_json::Value,
        sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let (schema, table, field) = self.resolve_field_info(collection_id, field_id).await?;

        let sql = format!(
            "SELECT COUNT(*) as total, COUNT(DISTINCT \"{}\") as unique_count \
             FROM \"{}\".\"{}\" WHERE \"{}\" IS NOT NULL",
            field, schema, table, field
        );

        let row: (i64, i64) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        let total_rows = row.0;
        let unique_count = row.1;
        let failed_rows = total_rows - unique_count;
        let pass_percentage = if total_rows > 0 {
            (unique_count as f64 / total_rows as f64) * 100.0
        } else {
            100.0
        };

        // 获取重复值样本
        let failed_samples = if failed_rows > 0 {
            let sample_sql = format!(
                "SELECT ctid::text as row_id, \"{}\"::text as value \
                 FROM \"{}\".\"{}\" \
                 WHERE \"{}\" IN ( \
                     SELECT \"{}\" \
                     FROM \"{}\".\"{}\" \
                     WHERE \"{}\" IS NOT NULL \
                     GROUP BY \"{}\" \
                     HAVING COUNT(*) > 1 \
                 ) \
                 LIMIT {}",
                field, schema, table, field, field, schema, table, field, field, sample_limit
            );

            let rows: Vec<(String, String)> = sqlx::query_as(AssertSqlSafe(sample_sql.as_str()))
                .fetch_all(&self.db_pool)
                .await
                .map_err(crate::models::QualityError::Database)?;

            rows.into_iter()
                .map(|(row_id, value)| FailedRowSample {
                    row_id: Some(row_id),
                    field_value: value,
                    reason: "Duplicate value".to_string(),
                })
                .collect()
        } else {
            vec![]
        };

        Ok(RuleExecutionResult {
            id: 0,
            rule_id: 0,
            rule_name: String::new(),
            rule_type: RuleType::Unique,
            severity: RuleSeverity::Error,
            status: if failed_rows == 0 {
                RuleStatus::Passed
            } else {
                RuleStatus::Failed
            },
            executed_at: chrono::Utc::now(),
            duration_ms: 0,
            total_rows,
            passed_rows: unique_count,
            failed_rows,
            pass_percentage,
            failed_samples,
            error_message: None,
        })
    }

    /// 范围检查
    async fn check_range(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        parameters: &serde_json::Value,
        inclusive: bool,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let (schema, table, field) = self.resolve_field_info(collection_id, field_id).await?;

        let min: Option<f64> = parameters.get("min").and_then(|v| v.as_f64());
        let max: Option<f64> = parameters.get("max").and_then(|v| v.as_f64());

        let (op_min, op_max) = if inclusive { (">=", "<=") } else { (">", "<") };

        let mut conditions = vec![];
        if let Some(min_val) = min {
            conditions.push(format!("\"{}\"::numeric {} {}", field, op_min, min_val));
        }
        if let Some(max_val) = max {
            conditions.push(format!("\"{}\"::numeric {} {}", field, op_max, max_val));
        }

        let where_clause = if conditions.is_empty() {
            "TRUE".to_string()
        } else {
            conditions.join(" AND ")
        };

        let sql = format!(
            "SELECT COUNT(*) as total, \
             COUNT(CASE WHEN \"{}\" IS NOT NULL AND {} THEN 1 END) as passed \
             FROM \"{}\".\"{}\"",
            field, where_clause, schema, table
        );

        let row: (i64, Option<i64>) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        let total_rows = row.0;
        let passed_rows = row.1.unwrap_or(0);
        let failed_rows = total_rows - passed_rows;
        let pass_percentage = if total_rows > 0 {
            (passed_rows as f64 / total_rows as f64) * 100.0
        } else {
            100.0
        };

        Ok(RuleExecutionResult {
            id: 0,
            rule_id: 0,
            rule_name: String::new(),
            rule_type: RuleType::Range,
            severity: RuleSeverity::Error,
            status: if failed_rows == 0 {
                RuleStatus::Passed
            } else {
                RuleStatus::Failed
            },
            executed_at: chrono::Utc::now(),
            duration_ms: 0,
            total_rows,
            passed_rows,
            failed_rows,
            pass_percentage,
            failed_samples: vec![],
            error_message: None,
        })
    }

    /// 大于检查
    async fn check_greater_than(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        parameters: &serde_json::Value,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let threshold = parameters
            .get("value")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| {
                crate::models::QualityError::InvalidParameters("value required".into())
            })?;

        let mut params = parameters.clone();
        params["min"] = serde_json::json!(threshold);
        self.check_range(collection_id, field_id, &params, false, 0)
            .await
    }

    /// 小于检查
    async fn check_less_than(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        parameters: &serde_json::Value,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let threshold = parameters
            .get("value")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| {
                crate::models::QualityError::InvalidParameters("value required".into())
            })?;

        let mut params = parameters.clone();
        params["max"] = serde_json::json!(threshold);
        self.check_range(collection_id, field_id, &params, false, 0)
            .await
    }

    /// 等于检查
    async fn check_equal_to(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        parameters: &serde_json::Value,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let target = parameters
            .get("value")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| {
                crate::models::QualityError::InvalidParameters("value required".into())
            })?;

        let mut params = parameters.clone();
        params["min"] = serde_json::json!(target);
        params["max"] = serde_json::json!(target);
        self.check_range(collection_id, field_id, &params, true, 0)
            .await
    }

    /// 最小长度检查
    async fn check_length_min(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        parameters: &serde_json::Value,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let min_len = parameters
            .get("min")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| crate::models::QualityError::InvalidParameters("min required".into()))?;

        let mut params = parameters.clone();
        params["min"] = serde_json::json!(min_len);
        params["max"] = serde_json::json!(i64::MAX);
        self.check_length_range(collection_id, field_id, &params, 0)
            .await
    }

    /// 最大长度检查
    async fn check_length_max(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        parameters: &serde_json::Value,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let max_len = parameters
            .get("max")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| crate::models::QualityError::InvalidParameters("max required".into()))?;

        let mut params = parameters.clone();
        params["min"] = serde_json::json!(0i64);
        params["max"] = serde_json::json!(max_len);
        self.check_length_range(collection_id, field_id, &params, 0)
            .await
    }

    /// 长度范围检查
    async fn check_length_range(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        parameters: &serde_json::Value,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let (schema, table, field) = self.resolve_field_info(collection_id, field_id).await?;

        let min_len = parameters.get("min").and_then(|v| v.as_i64()).unwrap_or(0);
        let max_len = parameters
            .get("max")
            .and_then(|v| v.as_i64())
            .unwrap_or(i64::MAX);

        let sql = format!(
            "SELECT COUNT(*) as total, \
             COUNT(CASE WHEN \"{}\" IS NOT NULL AND LENGTH(\"{}\"::text) BETWEEN {} AND {} THEN 1 END) as passed \
             FROM \"{}\".\"{}\"",
            field, field, min_len, max_len, schema, table
        );

        let row: (i64, Option<i64>) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        let total_rows = row.0;
        let passed_rows = row.1.unwrap_or(0);
        let failed_rows = total_rows - passed_rows;

        Ok(self.build_result(total_rows, passed_rows, failed_rows, RuleType::LengthRange))
    }

    /// 正则匹配检查
    async fn check_regex(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        parameters: &serde_json::Value,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let (schema, table, field) = self.resolve_field_info(collection_id, field_id).await?;

        let pattern = parameters
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::models::QualityError::InvalidParameters("pattern required".into())
            })?;

        // 验证正则表达式
        Regex::new(pattern).map_err(|e| {
            crate::models::QualityError::InvalidParameters(format!("Invalid regex: {}", e))
        })?;

        let sql = format!(
            "SELECT COUNT(*) as total, \
             COUNT(CASE WHEN \"{}\" IS NOT NULL AND \"{}\"::text ~ '{}' THEN 1 END) as passed \
             FROM \"{}\".\"{}\"",
            field, field, pattern, schema, table
        );

        let row: (i64, Option<i64>) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        let total_rows = row.0;
        let passed_rows = row.1.unwrap_or(0);
        let failed_rows = total_rows - passed_rows;

        Ok(self.build_result(total_rows, passed_rows, failed_rows, RuleType::RegexMatch))
    }

    /// 邮箱格式检查
    async fn check_email_format(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let params = serde_json::json!({
            "pattern": r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$"
        });
        self.check_regex(collection_id, field_id, &params, 0).await
    }

    /// URL格式检查
    async fn check_url_format(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let params = serde_json::json!({
            "pattern": r"^https?://[^\s/$.?#].[^\s]*$"
        });
        self.check_regex(collection_id, field_id, &params, 0).await
    }

    /// UUID格式检查
    async fn check_uuid_format(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let params = serde_json::json!({
            "pattern": r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
        });
        self.check_regex(collection_id, field_id, &params, 0).await
    }

    /// 枚举值检查
    async fn check_enum(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        parameters: &serde_json::Value,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let (schema, table, field) = self.resolve_field_info(collection_id, field_id).await?;

        let values: Vec<String> = parameters
            .get("values")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .ok_or_else(|| {
                crate::models::QualityError::InvalidParameters("values required".into())
            })?;

        let placeholders: Vec<String> = values
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", i + 1))
            .collect();

        let sql = format!(
            "SELECT COUNT(*) as total, \
             COUNT(CASE WHEN \"{}\" IS NOT NULL AND \"{}\"::text IN ({}) THEN 1 END) as passed \
             FROM \"{}\".\"{}\"",
            field,
            field,
            placeholders.join(", "),
            schema,
            table
        );

        let mut query = sqlx::query_as::<_, (i64, Option<i64>)>(AssertSqlSafe(sql.as_str()));
        for value in &values {
            query = query.bind(value);
        }

        let row = query
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        let total_rows = row.0;
        let passed_rows = row.1.unwrap_or(0);
        let failed_rows = total_rows - passed_rows;

        Ok(self.build_result(total_rows, passed_rows, failed_rows, RuleType::Enum))
    }

    /// 排除值检查
    async fn check_not_in(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        parameters: &serde_json::Value,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let (schema, table, field) = self.resolve_field_info(collection_id, field_id).await?;

        let values: Vec<String> = parameters
            .get("values")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .ok_or_else(|| {
                crate::models::QualityError::InvalidParameters("values required".into())
            })?;

        let placeholders: Vec<String> = values
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", i + 1))
            .collect();

        let sql = format!(
            "SELECT COUNT(*) as total, \
             COUNT(CASE WHEN \"{}\" IS NOT NULL AND \"{}\"::text NOT IN ({}) THEN 1 END) as passed \
             FROM \"{}\".\"{}\"",
            field,
            field,
            placeholders.join(", "),
            schema,
            table
        );

        let mut query = sqlx::query_as::<_, (i64, Option<i64>)>(AssertSqlSafe(sql.as_str()));
        for value in &values {
            query = query.bind(value);
        }

        let row = query
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        let total_rows = row.0;
        let passed_rows = row.1.unwrap_or(0);
        let failed_rows = total_rows - passed_rows;

        Ok(self.build_result(total_rows, passed_rows, failed_rows, RuleType::NotIn))
    }

    /// 数字类型检查
    async fn check_is_numeric(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let (schema, table, field) = self.resolve_field_info(collection_id, field_id).await?;

        let sql = format!(
            "SELECT COUNT(*) as total, \
             COUNT(CASE WHEN \"{}\" IS NOT NULL AND \"{}\"::text ~ '^-?[0-9]+\\.?[0-9]*$' THEN 1 END) as passed \
             FROM \"{}\".\"{}\"",
            field, field, schema, table
        );

        let row: (i64, Option<i64>) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        let total_rows = row.0;
        let passed_rows = row.1.unwrap_or(0);
        let failed_rows = total_rows - passed_rows;

        Ok(self.build_result(total_rows, passed_rows, failed_rows, RuleType::IsNumeric))
    }

    /// 整数类型检查
    async fn check_is_integer(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let (schema, table, field) = self.resolve_field_info(collection_id, field_id).await?;

        let sql = format!(
            "SELECT COUNT(*) as total, \
             COUNT(CASE WHEN \"{}\" IS NOT NULL AND \"{}\"::text ~ '^-?[0-9]+$' THEN 1 END) as passed \
             FROM \"{}\".\"{}\"",
            field, field, schema, table
        );

        let row: (i64, Option<i64>) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        let total_rows = row.0;
        let passed_rows = row.1.unwrap_or(0);
        let failed_rows = total_rows - passed_rows;

        Ok(self.build_result(total_rows, passed_rows, failed_rows, RuleType::IsInteger))
    }

    /// 布尔类型检查
    async fn check_is_boolean(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let (schema, table, field) = self.resolve_field_info(collection_id, field_id).await?;

        let sql = format!(
            "SELECT COUNT(*) as total, \
             COUNT(CASE WHEN \"{}\" IS NOT NULL AND \\
             LOWER(\"{}\"::text) IN ('true', 'false', '0', '1', 'yes', 'no') THEN 1 END) as passed \
             FROM \"{}\".\"{}\"",
            field, field, schema, table
        );

        let row: (i64, Option<i64>) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        let total_rows = row.0;
        let passed_rows = row.1.unwrap_or(0);
        let failed_rows = total_rows - passed_rows;

        Ok(self.build_result(total_rows, passed_rows, failed_rows, RuleType::IsBoolean))
    }

    /// 日期类型检查
    async fn check_is_date(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let (schema, table, field) = self.resolve_field_info(collection_id, field_id).await?;

        let sql = format!(
            "SELECT COUNT(*) as total, \
             COUNT(CASE WHEN \"{}\" IS NOT NULL AND \\
             \"{}\"::text ~ '^[0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}$' THEN 1 END) as passed \
             FROM \"{}\".\"{}\"",
            field, field, schema, table
        );

        let row: (i64, Option<i64>) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        let total_rows = row.0;
        let passed_rows = row.1.unwrap_or(0);
        let failed_rows = total_rows - passed_rows;

        Ok(self.build_result(total_rows, passed_rows, failed_rows, RuleType::IsDate))
    }

    /// 时间戳类型检查
    async fn check_is_timestamp(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let (schema, table, field) = self.resolve_field_info(collection_id, field_id).await?;

        let sql = format!(
            "SELECT COUNT(*) as total, \
             COUNT(CASE WHEN \"{}\" IS NOT NULL AND \\
             \"{}\"::text ~ '^[0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}[ T][0-9]{{2}}:[0-9]{{2}}:[0-9]{{2}}' THEN 1 END) as passed \
             FROM \"{}\".\"{}\"",
            field, field, schema, table
        );

        let row: (i64, Option<i64>) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        let total_rows = row.0;
        let passed_rows = row.1.unwrap_or(0);
        let failed_rows = total_rows - passed_rows;

        Ok(self.build_result(total_rows, passed_rows, failed_rows, RuleType::IsTimestamp))
    }

    /// 空值比例检查
    async fn check_null_percentage(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        parameters: &serde_json::Value,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let (schema, table, field) = self.resolve_field_info(collection_id, field_id).await?;

        let max_pct = parameters
            .get("max_percentage")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| {
                crate::models::QualityError::InvalidParameters("max_percentage required".into())
            })?;

        let sql = format!(
            "SELECT COUNT(*) as total, \
             COUNT(*) FILTER (WHERE \"{}\" IS NULL) as null_count \
             FROM \"{}\".\"{}\"",
            field, schema, table
        );

        let row: (i64, i64) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        let total_rows = row.0;
        let null_count = row.1;
        let null_pct = if total_rows > 0 {
            (null_count as f64 / total_rows as f64) * 100.0
        } else {
            0.0
        };

        let passed = null_pct <= max_pct;
        let passed_rows = if passed { total_rows } else { 0 };
        let failed_rows = if passed { 0 } else { total_rows };

        Ok(RuleExecutionResult {
            id: 0,
            rule_id: 0,
            rule_name: String::new(),
            rule_type: RuleType::NullPercentage,
            severity: RuleSeverity::Warning,
            status: if passed {
                RuleStatus::Passed
            } else {
                RuleStatus::Failed
            },
            executed_at: chrono::Utc::now(),
            duration_ms: 0,
            total_rows,
            passed_rows,
            failed_rows,
            pass_percentage: if passed { 100.0 } else { 0.0 },
            failed_samples: vec![],
            error_message: None,
        })
    }

    /// 基数比例检查
    async fn check_cardinality_ratio(
        &self,
        collection_id: Option<i64>,
        field_id: Option<i64>,
        parameters: &serde_json::Value,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let (schema, table, field) = self.resolve_field_info(collection_id, field_id).await?;

        let min_ratio = parameters.get("min_ratio").and_then(|v| v.as_f64());
        let max_ratio = parameters.get("max_ratio").and_then(|v| v.as_f64());

        let sql = format!(
            "SELECT COUNT(*) as total, COUNT(DISTINCT \"{}\") as unique_count \
             FROM \"{}\".\"{}\" WHERE \"{}\" IS NOT NULL",
            field, schema, table, field
        );

        let row: (i64, i64) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        let non_null_count = row.0;
        let unique_count = row.1;
        let ratio = if non_null_count > 0 {
            unique_count as f64 / non_null_count as f64
        } else {
            0.0
        };

        let passed = (min_ratio.map(|m| ratio >= m).unwrap_or(true))
            && (max_ratio.map(|m| ratio <= m).unwrap_or(true));

        let total_rows = row.0;
        let passed_rows = if passed { total_rows } else { 0 };
        let failed_rows = if passed { 0 } else { total_rows };

        Ok(RuleExecutionResult {
            id: 0,
            rule_id: 0,
            rule_name: String::new(),
            rule_type: RuleType::CardinalityRatio,
            severity: RuleSeverity::Warning,
            status: if passed {
                RuleStatus::Passed
            } else {
                RuleStatus::Failed
            },
            executed_at: chrono::Utc::now(),
            duration_ms: 0,
            total_rows,
            passed_rows,
            failed_rows,
            pass_percentage: if passed { 100.0 } else { 0.0 },
            failed_samples: vec![],
            error_message: None,
        })
    }

    /// 自定义SQL检查
    async fn check_custom_sql(
        &self,
        _collection_id: Option<i64>,
        _field_id: Option<i64>,
        parameters: &serde_json::Value,
        _sample_limit: i64,
    ) -> Result<RuleExecutionResult, crate::models::QualityError> {
        let sql = parameters
            .get("sql")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::models::QualityError::InvalidParameters("sql required".into()))?;

        // 注意：这里需要处理SQL安全性，简化实现
        let row: (i64,) = sqlx::query_as(AssertSqlSafe(sql))
            .fetch_one(&self.db_pool)
            .await
            .map_err(|e| crate::models::QualityError::Execution(e.to_string()))?;

        let failed_count = row.0;

        Ok(RuleExecutionResult {
            id: 0,
            rule_id: 0,
            rule_name: String::new(),
            rule_type: RuleType::CustomSql,
            severity: RuleSeverity::Error,
            status: if failed_count == 0 {
                RuleStatus::Passed
            } else {
                RuleStatus::Failed
            },
            executed_at: chrono::Utc::now(),
            duration_ms: 0,
            total_rows: 0,
            passed_rows: 0,
            failed_rows: failed_count,
            pass_percentage: if failed_count == 0 { 100.0 } else { 0.0 },
            failed_samples: vec![],
            error_message: None,
        })
    }

    /// 辅助方法：获取失败样本
    async fn get_failed_samples(
        &self,
        schema: &str,
        table: &str,
        field: &str,
        where_clause: String,
        reason: &str,
        limit: i64,
    ) -> Result<Vec<FailedRowSample>, crate::models::QualityError> {
        let sql = format!(
            "SELECT ctid::text as row_id, \"{}\"::text as value \
             FROM \"{}\".\"{}\" \
             WHERE {} \
             LIMIT {}",
            field, schema, table, where_clause, limit
        );

        let rows: Vec<(String, Option<String>)> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_all(&self.db_pool)
            .await
            .map_err(crate::models::QualityError::Database)?;

        Ok(rows
            .into_iter()
            .map(|(row_id, value)| FailedRowSample {
                row_id: Some(row_id),
                field_value: value.unwrap_or_default(),
                reason: reason.to_string(),
            })
            .collect())
    }

    /// 辅助方法：构建执行结果
    fn build_result(
        &self,
        total_rows: i64,
        passed_rows: i64,
        failed_rows: i64,
        rule_type: RuleType,
    ) -> RuleExecutionResult {
        let pass_percentage = if total_rows > 0 {
            (passed_rows as f64 / total_rows as f64) * 100.0
        } else {
            100.0
        };

        RuleExecutionResult {
            id: 0,
            rule_id: 0,
            rule_name: String::new(),
            rule_type,
            severity: RuleSeverity::Error, // 默认为 Error 级别
            status: if failed_rows == 0 {
                RuleStatus::Passed
            } else {
                RuleStatus::Failed
            },
            executed_at: chrono::Utc::now(),
            duration_ms: 0,
            total_rows,
            passed_rows,
            failed_rows,
            pass_percentage,
            failed_samples: vec![],
            error_message: None,
        }
    }
}
