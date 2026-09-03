use chrono::Utc;
use regex::Regex;
use sqlx::{AssertSqlSafe, PgPool};

use crate::models::{
    CheckStatus, ColumnProfile, FailedRecord, QualityCheck, QualityMetrics, QualityReport,
    QualityRule, RuleType, TableProfile,
};

pub struct QualityProfiler;

fn validate_pg_ident(ident: &str) -> Result<(), sqlx::Error> {
    if ident.is_empty() || ident.len() > 63 {
        return Err(sqlx::Error::Protocol(
            "Invalid identifier: exceeds length limits or empty".to_string(),
        ));
    }
    let mut chars = ident.chars();
    if let Some(first) = chars.next() {
        if first.is_ascii_digit() || !(first.is_ascii_alphabetic() || first == '_') {
            return Err(sqlx::Error::Protocol(
                "Invalid identifier: must start with letter or underscore".to_string(),
            ));
        }
    }
    if chars.any(|c| !(c.is_ascii_alphanumeric() || c == '_')) {
        return Err(sqlx::Error::Protocol(
            "Invalid identifier: contains illegal characters".to_string(),
        ));
    }
    Ok(())
}

impl QualityProfiler {
    pub async fn create_rule(
        pool: &PgPool,
        name: String,
        rule_type: RuleType,
        target_table: String,
        target_column: Option<String>,
        parameters: serde_json::Value,
        severity: crate::models::Severity,
    ) -> Result<QualityRule, sqlx::Error> {
        let now = Utc::now();

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS isahl_meta.quality_rules (
                id BIGSERIAL PRIMARY KEY,
                name VARCHAR(255) NOT NULL,
                rule_type VARCHAR(50) NOT NULL,
                target_table VARCHAR(255) NOT NULL,
                target_column VARCHAR(255),
                parameters JSONB NOT NULL,
                severity VARCHAR(20) NOT NULL,
                created_at TIMESTAMPTZ NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await?;

        let id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO isahl_meta.quality_rules (name, rule_type, target_table, target_column, parameters, severity, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id
            "#,
        )
        .bind(&name)
        .bind(rule_type.as_str())
        .bind(&target_table)
        .bind(&target_column)
        .bind(&parameters)
        .bind(severity.as_str())
        .bind(now)
        .fetch_one(pool)
        .await?;

        Ok(QualityRule {
            id,
            name,
            rule_type,
            target_table,
            target_column,
            parameters,
            severity,
            created_at: now,
        })
    }

    pub async fn list_rules(pool: &PgPool) -> Result<Vec<QualityRule>, sqlx::Error> {
        let rows = sqlx::query_as::<_, (i64, String, String, String, Option<String>, serde_json::Value, String, chrono::DateTime<Utc>)>(
            r#"
            SELECT id, name, rule_type, target_table, target_column, parameters, severity, created_at
            FROM isahl_meta.quality_rules
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    name,
                    rule_type,
                    target_table,
                    target_column,
                    parameters,
                    severity,
                    created_at,
                )| {
                    QualityRule {
                        id,
                        name,
                        rule_type: RuleType::from_string(&rule_type),
                        target_table,
                        target_column,
                        parameters,
                        severity: crate::models::Severity::from_string(&severity),
                        created_at,
                    }
                },
            )
            .collect())
    }

    pub async fn get_rule(pool: &PgPool, rule_id: i64) -> Result<Option<QualityRule>, sqlx::Error> {
        let row = sqlx::query_as::<_, (i64, String, String, String, Option<String>, serde_json::Value, String, chrono::DateTime<Utc>)>(
            r#"
            SELECT id, name, rule_type, target_table, target_column, parameters, severity, created_at
            FROM isahl_meta.quality_rules
            WHERE id = $1
            "#,
        )
        .bind(rule_id)
        .fetch_optional(pool)
        .await?;

        Ok(row.map(
            |(
                id,
                name,
                rule_type,
                target_table,
                target_column,
                parameters,
                severity,
                created_at,
            )| {
                QualityRule {
                    id,
                    name,
                    rule_type: RuleType::from_string(&rule_type),
                    target_table,
                    target_column,
                    parameters,
                    severity: crate::models::Severity::from_string(&severity),
                    created_at,
                }
            },
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_rule(
        pool: &PgPool,
        rule_id: i64,
        name: Option<String>,
        rule_type: Option<RuleType>,
        target_table: Option<String>,
        target_column: Option<Option<String>>,
        parameters: Option<serde_json::Value>,
        severity: Option<crate::models::Severity>,
    ) -> Result<QualityRule, sqlx::Error> {
        let existing = Self::get_rule(pool, rule_id).await?;
        let mut rule = match existing {
            Some(r) => r,
            None => return Err(sqlx::Error::RowNotFound),
        };

        if let Some(n) = name {
            rule.name = n;
        }
        if let Some(rt) = rule_type {
            rule.rule_type = rt;
        }
        if let Some(tt) = target_table {
            rule.target_table = tt;
        }
        if let Some(tc) = target_column {
            rule.target_column = tc;
        }
        if let Some(p) = parameters {
            rule.parameters = p;
        }
        if let Some(s) = severity {
            rule.severity = s;
        }

        sqlx::query(
            r#"
            UPDATE isahl_meta.quality_rules
            SET name = $1, rule_type = $2, target_table = $3, target_column = $4, parameters = $5, severity = $6
            WHERE id = $7
            "#,
        )
        .bind(&rule.name)
        .bind(rule.rule_type.as_str())
        .bind(&rule.target_table)
        .bind(&rule.target_column)
        .bind(&rule.parameters)
        .bind(rule.severity.as_str())
        .bind(rule_id)
        .execute(pool)
        .await?;

        Ok(rule)
    }

    pub async fn delete_rule(pool: &PgPool, rule_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            DELETE FROM isahl_meta.quality_rules
            WHERE id = $1
            "#,
        )
        .bind(rule_id)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn run_check(pool: &PgPool, rule_id: i64) -> Result<QualityCheck, sqlx::Error> {
        let now = Utc::now();

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS isahl_meta.quality_checks (
                id BIGSERIAL PRIMARY KEY,
                rule_id BIGINT NOT NULL,
                status VARCHAR(20) NOT NULL,
                records_checked BIGINT DEFAULT 0,
                records_failed BIGINT DEFAULT 0,
                started_at TIMESTAMPTZ NOT NULL,
                completed_at TIMESTAMPTZ
            )
            "#,
        )
        .execute(pool)
        .await?;

        let check_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO isahl_meta.quality_checks (rule_id, status, started_at)
            VALUES ($1, $2, $3)
            RETURNING id
            "#,
        )
        .bind(rule_id)
        .bind(CheckStatus::Running.as_str())
        .bind(now)
        .fetch_one(pool)
        .await?;

        let rule = match Self::get_rule(pool, rule_id).await? {
            Some(r) => r,
            None => {
                return Err(sqlx::Error::RowNotFound);
            }
        };

        let (records_checked, records_failed) = Self::execute_rule_check(pool, &rule).await?;

        let status = if records_failed == 0 {
            CheckStatus::Passed
        } else {
            CheckStatus::Failed
        };

        sqlx::query(
            r#"
            UPDATE isahl_meta.quality_checks
            SET status = $1, records_checked = $2, records_failed = $3, completed_at = $4
            WHERE id = $5
            "#,
        )
        .bind(status.as_str())
        .bind(records_checked)
        .bind(records_failed)
        .bind(Utc::now())
        .bind(check_id)
        .execute(pool)
        .await?;

        Ok(QualityCheck {
            id: check_id,
            rule_id,
            status,
            records_checked,
            records_failed,
            started_at: now,
            completed_at: Some(Utc::now()),
        })
    }

    async fn execute_rule_check(
        pool: &PgPool,
        rule: &QualityRule,
    ) -> Result<(i64, i64), sqlx::Error> {
        match rule.rule_type {
            RuleType::NotNull => Self::check_not_null(pool, rule).await,
            RuleType::Unique => Self::check_unique(pool, rule).await,
            RuleType::Pattern => Self::check_pattern(pool, rule).await,
            RuleType::Range => Self::check_range(pool, rule).await,
            RuleType::Length => Self::check_length(pool, rule).await,
            RuleType::DataType => Self::check_data_type(pool, rule).await,
        }
    }

    async fn check_not_null(pool: &PgPool, rule: &QualityRule) -> Result<(i64, i64), sqlx::Error> {
        let column = rule.target_column.as_ref().unwrap();
        let table = &rule.target_table;
        validate_pg_ident(table)?;
        validate_pg_ident(column)?;

        let total: (i64,) = sqlx::query_as(AssertSqlSafe(
            format!("SELECT COUNT(*) FROM \"{}\"", table).as_str(),
        ))
        .fetch_one(pool)
        .await?;

        let non_null: (i64,) = sqlx::query_as(AssertSqlSafe(format!(
            "SELECT COUNT(*) FROM \"{}\" WHERE \"{}\" IS NOT NULL",
            table, column
        )))
        .fetch_one(pool)
        .await?;

        let records_checked = total.0;
        let records_failed = total.0 - non_null.0;

        Ok((records_checked, records_failed))
    }

    async fn check_unique(pool: &PgPool, rule: &QualityRule) -> Result<(i64, i64), sqlx::Error> {
        let column = rule.target_column.as_ref().unwrap();
        let table = &rule.target_table;
        validate_pg_ident(table)?;
        validate_pg_ident(column)?;

        let total: (i64,) = sqlx::query_as(AssertSqlSafe(
            format!("SELECT COUNT(*) FROM \"{}\"", table).as_str(),
        ))
        .fetch_one(pool)
        .await?;

        let unique: (i64,) = sqlx::query_as(AssertSqlSafe(format!(
            "SELECT COUNT(DISTINCT \"{}\") FROM \"{}\"",
            column, table
        )))
        .fetch_one(pool)
        .await?;

        let records_checked = total.0;
        let records_failed = total.0 - unique.0;

        Ok((records_checked, records_failed))
    }

    async fn check_pattern(pool: &PgPool, rule: &QualityRule) -> Result<(i64, i64), sqlx::Error> {
        let column = rule.target_column.as_ref().unwrap();
        let table = &rule.target_table;
        validate_pg_ident(table)?;
        validate_pg_ident(column)?;
        let pattern = rule
            .parameters
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or(".*");

        let regex = Regex::new(pattern)
            .map_err(|_| sqlx::Error::Protocol("Invalid regex pattern".to_string()))?;

        let rows = sqlx::query_as::<_, (String,)>(AssertSqlSafe(format!(
            "SELECT \"{}\" FROM \"{}\" WHERE \"{}\" IS NOT NULL",
            column, table, column
        )))
        .fetch_all(pool)
        .await?;

        let records_checked = rows.len() as i64;
        let records_failed = rows.iter().filter(|r| !regex.is_match(&r.0)).count() as i64;

        Ok((records_checked, records_failed))
    }

    async fn check_range(pool: &PgPool, rule: &QualityRule) -> Result<(i64, i64), sqlx::Error> {
        let column = rule.target_column.as_ref().unwrap();
        let table = &rule.target_table;
        validate_pg_ident(table)?;
        validate_pg_ident(column)?;
        let min = rule.parameters.get("min").and_then(|v| v.as_f64());
        let max = rule.parameters.get("max").and_then(|v| v.as_f64());

        let rows = sqlx::query_as::<_, (Option<String>,)>(AssertSqlSafe(format!(
            "SELECT \"{}\"::TEXT FROM \"{}\" WHERE \"{}\" IS NOT NULL",
            column, table, column
        )))
        .fetch_all(pool)
        .await?;

        let records_checked = rows.len() as i64;
        let records_failed = rows
            .iter()
            .filter(|r| {
                if let Some(ref val) = r.0 {
                    if let Ok(num) = val.parse::<f64>() {
                        if let Some(m) = min {
                            if num < m {
                                return true;
                            }
                        }
                        if let Some(m) = max {
                            if num > m {
                                return true;
                            }
                        }
                    }
                }
                false
            })
            .count() as i64;

        Ok((records_checked, records_failed))
    }

    async fn check_length(pool: &PgPool, rule: &QualityRule) -> Result<(i64, i64), sqlx::Error> {
        let column = rule.target_column.as_ref().unwrap();
        let table = &rule.target_table;
        validate_pg_ident(table)?;
        validate_pg_ident(column)?;
        let min_len = rule
            .parameters
            .get("min_length")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as usize;
        let max_len = rule
            .parameters
            .get("max_length")
            .and_then(|v| v.as_i64())
            .unwrap_or(i64::MAX) as usize;

        let rows = sqlx::query_as::<_, (Option<String>,)>(AssertSqlSafe(format!(
            "SELECT \"{}\" FROM \"{}\" WHERE \"{}\" IS NOT NULL",
            column, table, column
        )))
        .fetch_all(pool)
        .await?;

        let records_checked = rows.len() as i64;
        let records_failed = rows
            .iter()
            .filter(|r| {
                if let Some(ref val) = r.0 {
                    let len = val.len();
                    if len < min_len || len > max_len {
                        return true;
                    }
                }
                false
            })
            .count() as i64;

        Ok((records_checked, records_failed))
    }

    async fn check_data_type(pool: &PgPool, rule: &QualityRule) -> Result<(i64, i64), sqlx::Error> {
        let column = rule.target_column.as_ref().unwrap();
        let table = &rule.target_table;
        validate_pg_ident(table)?;
        validate_pg_ident(column)?;

        let rows = sqlx::query_as::<_, (Option<String>,)>(AssertSqlSafe(format!(
            "SELECT \"{}\"::TEXT FROM \"{}\" WHERE \"{}\" IS NOT NULL",
            column, table, column
        )))
        .fetch_all(pool)
        .await?;

        let expected_type = rule
            .parameters
            .get("expected_type")
            .and_then(|v| v.as_str())
            .unwrap_or("string");

        let records_checked = rows.len() as i64;
        let records_failed = rows
            .iter()
            .filter(|r| {
                if let Some(ref val) = r.0 {
                    match expected_type {
                        "integer" => val.parse::<i64>().is_err(),
                        "float" | "decimal" => val.parse::<f64>().is_err(),
                        "boolean" => {
                            !["true", "false", "0", "1"].contains(&val.to_lowercase().as_str())
                        }
                        _ => false,
                    }
                } else {
                    false
                }
            })
            .count() as i64;

        Ok((records_checked, records_failed))
    }

    pub async fn get_profile(pool: &PgPool, table: &str) -> Result<TableProfile, sqlx::Error> {
        let row_count: (i64,) = sqlx::query_as(AssertSqlSafe(
            format!("SELECT COUNT(*) FROM \"{}\"", table).as_str(),
        ))
        .fetch_one(pool)
        .await?;

        let columns = sqlx::query_as::<_, (String, String)>(AssertSqlSafe(
            "SELECT column_name, data_type FROM information_schema.columns WHERE table_name = $1"
                .to_string(),
        ))
        .bind(table)
        .fetch_all(pool)
        .await?;

        let mut column_profiles = Vec::new();
        let mut total_completeness = 0.0;
        let mut total_uniqueness = 0.0;

        for (col_name, _data_type) in columns {
            let null_count: (i64,) = sqlx::query_as(AssertSqlSafe(format!(
                "SELECT COUNT(*) FROM \"{}\" WHERE \"{}\" IS NULL",
                table, col_name
            )))
            .fetch_one(pool)
            .await?;

            let unique_count: (i64,) = sqlx::query_as(AssertSqlSafe(format!(
                "SELECT COUNT(DISTINCT \"{}\") FROM \"{}\"",
                col_name, table
            )))
            .fetch_one(pool)
            .await?;

            let total_rows = row_count.0.max(1);
            let null_pct = (null_count.0 as f64 / total_rows as f64) * 100.0;
            let unique_pct = (unique_count.0 as f64 / total_rows as f64) * 100.0;

            total_completeness += 100.0 - null_pct;
            total_uniqueness += unique_pct;

            column_profiles.push(ColumnProfile {
                column_name: col_name,
                data_type: _data_type,
                null_count: null_count.0,
                null_percentage: null_pct,
                unique_count: unique_count.0,
                unique_percentage: unique_pct,
            });
        }

        let col_count = column_profiles.len().max(1) as f64;
        let completeness = total_completeness / col_count;
        let uniqueness = total_uniqueness / col_count;

        Ok(TableProfile {
            table_name: table.to_string(),
            row_count: row_count.0,
            metrics: QualityMetrics {
                completeness,
                uniqueness,
                validity: 100.0,
                consistency: 100.0,
            },
            column_profiles,
        })
    }

    pub async fn get_check(
        pool: &PgPool,
        check_id: i64,
    ) -> Result<Option<QualityCheck>, sqlx::Error> {
        let row = sqlx::query_as::<
            _,
            (
                i64,
                i64,
                String,
                i64,
                i64,
                chrono::DateTime<Utc>,
                Option<chrono::DateTime<Utc>>,
            ),
        >(
            r#"
            SELECT id, rule_id, status, records_checked, records_failed, started_at, completed_at
            FROM isahl_meta.quality_checks
            WHERE id = $1
            "#,
        )
        .bind(check_id)
        .fetch_optional(pool)
        .await?;

        Ok(row.map(
            |(id, rule_id, status, records_checked, records_failed, started_at, completed_at)| {
                QualityCheck {
                    id,
                    rule_id,
                    status: CheckStatus::from_string(&status),
                    records_checked,
                    records_failed,
                    started_at,
                    completed_at,
                }
            },
        ))
    }

    pub async fn generate_report(
        pool: &PgPool,
        check_id: i64,
    ) -> Result<Option<QualityReport>, sqlx::Error> {
        let check = match Self::get_check(pool, check_id).await? {
            Some(c) => c,
            None => return Ok(None),
        };

        let rule = match Self::get_rule(pool, check.rule_id).await? {
            Some(r) => r,
            None => return Ok(None),
        };

        let score = if check.records_checked > 0 {
            ((check.records_checked - check.records_failed) as f64 / check.records_checked as f64)
                * 100.0
        } else {
            100.0
        };

        let failed_records = Self::get_failed_records_sample(pool, &rule, 100).await?;

        Ok(Some(QualityReport {
            id: 0,
            check_id,
            overall_score: score,
            metrics: QualityMetrics {
                completeness: 100.0
                    - (check.records_failed as f64 / check.records_checked.max(1) as f64 * 100.0),
                uniqueness: 100.0,
                validity: score,
                consistency: 100.0,
            },
            failed_records,
        }))
    }

    async fn get_failed_records_sample(
        pool: &PgPool,
        rule: &QualityRule,
        limit: i64,
    ) -> Result<Vec<FailedRecord>, sqlx::Error> {
        let table = &rule.target_table;
        let column = rule.target_column.as_ref().unwrap();

        let query = match rule.rule_type {
            RuleType::NotNull => format!(
                "SELECT ctid::TEXT as record_id, '{}' as column, NULL::TEXT as value, 'Value is NULL' as reason FROM \"{}\" WHERE \"{}\" IS NULL LIMIT {}",
                column, table, column, limit
            ),
            RuleType::Unique => format!(
                "SELECT \"{}\"::TEXT as record_id, '{}' as column, \"{}\"::TEXT as value, 'Duplicate value' as reason FROM \"{}\" GROUP BY \"{}\" HAVING COUNT(*) > 1 LIMIT {}",
                column, column, column, table, column, limit
            ),
            _ => format!(
                "SELECT ctid::TEXT as record_id, '{}' as column, \"{}\"::TEXT as value, 'Quality check failed' as reason FROM \"{}\" LIMIT {}",
                column, column, table, limit
            ),
        };

        let rows = sqlx::query_as::<_, (String, String, Option<String>, String)>(AssertSqlSafe(
            query.as_str(),
        ))
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(record_id, column, value, reason)| FailedRecord {
                record_id,
                column,
                value,
                reason,
            })
            .collect())
    }
}
