//! Audit query functions
//!
//! Provides query capabilities for audit logs including filtering by
//! subject, object, time range, and event type.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};

use super::handlers::{AuditEventRecord, DbAuditEvent};
use crate::epp::AccessEvent;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuditLogEntry {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    #[serde(with = "common::serde_zuid")]
    pub subject_id: i64,
    #[serde(with = "common::serde_zuid")]
    pub object_id: i64,
    pub object_type: String,
    pub operation: String,
    pub success: bool,
    pub metadata: Option<serde_json::Value>,
}

impl From<AuditEventRecord> for AuditLogEntry {
    fn from(record: AuditEventRecord) -> Self {
        Self {
            id: record.id,
            event_type: record.event_type.as_str().to_string(),
            timestamp: record.timestamp,
            subject_id: record.subject_id,
            object_id: record.object_id,
            object_type: record.object_type,
            operation: record.operation,
            success: record.success,
            metadata: record.metadata,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AuditQueryParams {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub subject_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub object_id: Option<i64>,
    pub object_type: Option<String>,
    pub event_type: Option<String>,
    pub operation: Option<String>,
    pub success: Option<bool>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub limit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub offset: Option<i64>,
}

impl Default for AuditQueryParams {
    fn default() -> Self {
        Self {
            subject_id: None,
            object_id: None,
            object_type: None,
            event_type: None,
            operation: None,
            success: None,
            start_time: None,
            end_time: None,
            limit: Some(100),
            offset: Some(0),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AuditQueryResult {
    pub entries: Vec<AuditLogEntry>,
    #[serde(with = "common::serde_zuid")]
    pub total_count: i64,
    #[serde(with = "common::serde_zuid")]
    pub limit: i64,
    #[serde(with = "common::serde_zuid")]
    pub offset: i64,
}

pub async fn query_audit_logs(
    pool: &PgPool,
    params: AuditQueryParams,
) -> Result<AuditQueryResult, AuditQueryError> {
    let limit = params.limit.unwrap_or(100).min(1000);
    let offset = params.offset.unwrap_or(0);

    let mut query = String::from(
        r#"
        SELECT
            id,
            decision AS event_type,
            created_at AS timestamp,
            user_id AS subject_id,
            COALESCE((metadata->>'object_id')::BIGINT, 0) AS object_id,
            COALESCE(metadata->>'object_type', '') AS object_type,
            operation,
            COALESCE((metadata->>'success')::BOOLEAN, false) AS success,
            metadata
        FROM isahl_audit.audit_events
        WHERE 1=1
        "#,
    );

    let mut count_query = String::from("SELECT COUNT(*) FROM isahl_audit.audit_events WHERE 1=1");

    let mut conditions = Vec::new();

    if params.subject_id.is_some() {
        let idx = conditions.len() + 1;
        conditions.push(format!(" AND user_id = ${}", idx));
        query.push_str(&format!(" AND user_id = ${}", idx));
        count_query.push_str(&format!(" AND user_id = ${}", idx));
    }

    if params.object_id.is_some() {
        let idx = conditions.len() + 1;
        conditions.push(format!(" AND (metadata->>'object_id')::BIGINT = ${}", idx));
        query.push_str(&format!(" AND (metadata->>'object_id')::BIGINT = ${}", idx));
        count_query.push_str(&format!(" AND (metadata->>'object_id')::BIGINT = ${}", idx));
    }

    if params.object_type.is_some() {
        let idx = conditions.len() + 1;
        conditions.push(format!(" AND metadata->>'object_type' = ${}", idx));
        query.push_str(&format!(" AND metadata->>'object_type' = ${}", idx));
        count_query.push_str(&format!(" AND metadata->>'object_type' = ${}", idx));
    }

    if params.event_type.is_some() {
        let idx = conditions.len() + 1;
        conditions.push(format!(" AND decision = ${}", idx));
        query.push_str(&format!(" AND decision = ${}", idx));
        count_query.push_str(&format!(" AND decision = ${}", idx));
    }

    if params.operation.is_some() {
        let idx = conditions.len() + 1;
        conditions.push(format!(" AND operation = ${}", idx));
        query.push_str(&format!(" AND operation = ${}", idx));
        count_query.push_str(&format!(" AND operation = ${}", idx));
    }

    if params.success.is_some() {
        let idx = conditions.len() + 1;
        conditions.push(format!(" AND metadata->>'success' = ${}::TEXT", idx));
        query.push_str(&format!(" AND metadata->>'success' = ${}::TEXT", idx));
        count_query.push_str(&format!(" AND metadata->>'success' = ${}::TEXT", idx));
    }

    if params.start_time.is_some() {
        let idx = conditions.len() + 1;
        conditions.push(format!(" AND created_at >= ${}", idx));
        query.push_str(&format!(" AND created_at >= ${}", idx));
        count_query.push_str(&format!(" AND created_at >= ${}", idx));
    }

    if params.end_time.is_some() {
        let idx = conditions.len() + 1;
        conditions.push(format!(" AND created_at <= ${}", idx));
        query.push_str(&format!(" AND created_at <= ${}", idx));
        count_query.push_str(&format!(" AND created_at <= ${}", idx));
    }

    query.push_str(&format!(
        " ORDER BY created_at DESC LIMIT ${}",
        conditions.len() + 1
    ));
    query.push_str(&format!(" OFFSET ${}", conditions.len() + 2));

    let entries = sqlx::query_as::<_, AuditLogEntry>(AssertSqlSafe(query.as_str()))
        .bind(params.subject_id)
        .bind(params.object_id)
        .bind(&params.object_type)
        .bind(&params.event_type)
        .bind(&params.operation)
        .bind(params.success)
        .bind(params.start_time)
        .bind(params.end_time)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(|e| AuditQueryError::DatabaseError(e.to_string()))?;

    let total_count = sqlx::query_scalar::<_, i64>(AssertSqlSafe(count_query.as_str()))
        .bind(params.subject_id)
        .bind(params.object_id)
        .bind(&params.object_type)
        .bind(&params.event_type)
        .bind(&params.operation)
        .bind(params.success)
        .bind(params.start_time)
        .bind(params.end_time)
        .fetch_one(pool)
        .await
        .map_err(|e| AuditQueryError::DatabaseError(e.to_string()))?;

    Ok(AuditQueryResult {
        entries,
        total_count,
        limit,
        offset,
    })
}

pub async fn query_by_subject(
    pool: &PgPool,
    subject_id: i64,
    limit: Option<i64>,
) -> Result<Vec<AuditLogEntry>, AuditQueryError> {
    let params = AuditQueryParams {
        subject_id: Some(subject_id),
        limit,
        ..Default::default()
    };

    let result = query_audit_logs(pool, params).await?;
    Ok(result.entries)
}

pub async fn query_by_object(
    pool: &PgPool,
    object_id: i64,
    limit: Option<i64>,
) -> Result<Vec<AuditLogEntry>, AuditQueryError> {
    let params = AuditQueryParams {
        object_id: Some(object_id),
        limit,
        ..Default::default()
    };

    let result = query_audit_logs(pool, params).await?;
    Ok(result.entries)
}

pub async fn query_by_timerange(
    pool: &PgPool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    limit: Option<i64>,
) -> Result<Vec<AuditLogEntry>, AuditQueryError> {
    let params = AuditQueryParams {
        start_time: Some(start),
        end_time: Some(end),
        limit,
        ..Default::default()
    };

    let result = query_audit_logs(pool, params).await?;
    Ok(result.entries)
}

pub async fn query_failed_access(
    pool: &PgPool,
    limit: Option<i64>,
) -> Result<Vec<AuditLogEntry>, AuditQueryError> {
    let params = AuditQueryParams {
        event_type: Some("ACCESS_DENIED".to_string()),
        success: Some(false),
        limit,
        ..Default::default()
    };

    let result = query_audit_logs(pool, params).await?;
    Ok(result.entries)
}

/// Event query parameters for searching audit events
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EventQuery {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub user_id: Option<i64>,
    pub object_path: Option<String>,
    pub operation: Option<String>,
    pub decision: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub page: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub limit: Option<i64>,
}

impl EventQuery {
    pub fn offset(&self) -> i64 {
        let page = self.page.unwrap_or(1).max(1);
        let limit = self.limit.unwrap_or(50).clamp(1, 1000);
        (page - 1) * limit
    }

    pub fn effective_limit(&self) -> i64 {
        self.limit.unwrap_or(50).clamp(1, 1000)
    }
}

/// Get audit events by user ID
pub async fn get_events_by_user(
    pool: &PgPool,
    user_id: i64,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    limit: Option<i64>,
) -> Result<Vec<AccessEvent>, AuditQueryError> {
    let limit = limit.unwrap_or(100).min(1000);

    let mut query = String::from(
        r#"
        SELECT id, user_id, user_email, object_path, operation, decision,
               subject_attributes::text[], object_attributes::text[], obligations_triggered,
               ip_address, user_agent, metadata, created_at
        FROM isahl_audit.audit_events
        WHERE user_id = $1
        "#,
    );

    let mut param_count = 1;

    if from.is_some() {
        param_count += 1;
        query.push_str(&format!(" AND created_at >= ${}", param_count));
    }

    if to.is_some() {
        param_count += 1;
        query.push_str(&format!(" AND created_at <= ${}", param_count));
    }

    query.push_str(&format!(
        " ORDER BY created_at DESC LIMIT ${}",
        param_count + 1
    ));

    let mut db_query =
        sqlx::query_as::<_, DbAuditEvent>(AssertSqlSafe(query.as_str())).bind(user_id);

    if let Some(from_time) = from {
        db_query = db_query.bind(from_time);
    }

    if let Some(to_time) = to {
        db_query = db_query.bind(to_time);
    }

    db_query = db_query.bind(limit);

    let entries = db_query
        .fetch_all(pool)
        .await
        .map_err(|e| AuditQueryError::DatabaseError(e.to_string()))?;

    Ok(entries.into_iter().map(|e| e.into_access_event()).collect())
}

/// Get audit events by object path
pub async fn get_events_by_object(
    pool: &PgPool,
    object_path: String,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    limit: Option<i64>,
) -> Result<Vec<AccessEvent>, AuditQueryError> {
    let limit = limit.unwrap_or(100).min(1000);

    let mut query = String::from(
        r#"
        SELECT id, user_id, user_email, object_path, operation, decision,
               subject_attributes::text[], object_attributes::text[], obligations_triggered,
               ip_address, user_agent, metadata, created_at
        FROM isahl_audit.audit_events
        WHERE object_path = $1
        "#,
    );

    let mut param_count = 1;

    if from.is_some() {
        param_count += 1;
        query.push_str(&format!(" AND created_at >= ${}", param_count));
    }

    if to.is_some() {
        param_count += 1;
        query.push_str(&format!(" AND created_at <= ${}", param_count));
    }

    query.push_str(&format!(
        " ORDER BY created_at DESC LIMIT ${}",
        param_count + 1
    ));

    let mut db_query =
        sqlx::query_as::<_, DbAuditEvent>(AssertSqlSafe(query.as_str())).bind(object_path);

    if let Some(from_time) = from {
        db_query = db_query.bind(from_time);
    }

    if let Some(to_time) = to {
        db_query = db_query.bind(to_time);
    }

    db_query = db_query.bind(limit);

    let entries = db_query
        .fetch_all(pool)
        .await
        .map_err(|e| AuditQueryError::DatabaseError(e.to_string()))?;

    Ok(entries.into_iter().map(|e| e.into_access_event()).collect())
}

/// Get audit events by decision
pub async fn get_events_by_decision(
    pool: &PgPool,
    decision: String,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    limit: Option<i64>,
) -> Result<Vec<AccessEvent>, AuditQueryError> {
    let limit = limit.unwrap_or(100).min(1000);

    let mut query = String::from(
        r#"
        SELECT id, user_id, user_email, object_path, operation, decision,
               subject_attributes::text[], object_attributes::text[], obligations_triggered,
               ip_address, user_agent, metadata, created_at
        FROM isahl_audit.audit_events
        WHERE decision = $1
        "#,
    );

    let mut param_count = 1;

    if from.is_some() {
        param_count += 1;
        query.push_str(&format!(" AND created_at >= ${}", param_count));
    }

    if to.is_some() {
        param_count += 1;
        query.push_str(&format!(" AND created_at <= ${}", param_count));
    }

    query.push_str(&format!(
        " ORDER BY created_at DESC LIMIT ${}",
        param_count + 1
    ));

    let mut db_query =
        sqlx::query_as::<_, DbAuditEvent>(AssertSqlSafe(query.as_str())).bind(decision);

    if let Some(from_time) = from {
        db_query = db_query.bind(from_time);
    }

    if let Some(to_time) = to {
        db_query = db_query.bind(to_time);
    }

    db_query = db_query.bind(limit);

    let entries = db_query
        .fetch_all(pool)
        .await
        .map_err(|e| AuditQueryError::DatabaseError(e.to_string()))?;

    Ok(entries.into_iter().map(|e| e.into_access_event()).collect())
}

/// Get failed access events (denied access)
pub async fn get_failed_access_events(
    pool: &PgPool,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    limit: Option<i64>,
) -> Result<Vec<AccessEvent>, AuditQueryError> {
    let limit = limit.unwrap_or(100).min(1000);

    let mut query = String::from(
        r#"
        SELECT id, user_id, user_email, object_path, operation, decision,
               subject_attributes::text[], object_attributes::text[], obligations_triggered,
               ip_address, user_agent, metadata, created_at
        FROM isahl_audit.audit_events
        WHERE decision = 'deny'
        "#,
    );

    let mut param_count = 1;

    if from.is_some() {
        param_count += 1;
        query.push_str(&format!(" AND created_at >= ${}", param_count));
    }

    if to.is_some() {
        param_count += 1;
        query.push_str(&format!(" AND created_at <= ${}", param_count));
    }

    query.push_str(&format!(
        " ORDER BY created_at DESC LIMIT ${}",
        param_count + 1
    ));

    let mut db_query = sqlx::query_as::<_, DbAuditEvent>(AssertSqlSafe(query.as_str()));

    if let Some(from_time) = from {
        db_query = db_query.bind(from_time);
    }

    if let Some(to_time) = to {
        db_query = db_query.bind(to_time);
    }

    db_query = db_query.bind(limit);

    let entries = db_query
        .fetch_all(pool)
        .await
        .map_err(|e| AuditQueryError::DatabaseError(e.to_string()))?;

    Ok(entries.into_iter().map(|e| e.into_access_event()).collect())
}

/// Search events with complex query parameters
pub async fn search_events(
    pool: &PgPool,
    query: EventQuery,
) -> Result<Vec<AccessEvent>, AuditQueryError> {
    let limit = query.effective_limit();
    let offset = query.offset();

    let mut sql = String::from(
        r#"
        SELECT id, user_id, user_email, object_path, operation, decision,
               subject_attributes::text[], object_attributes::text[], obligations_triggered,
               ip_address, user_agent, metadata, created_at
        FROM isahl_audit.audit_events
        WHERE 1=1
        "#,
    );

    let mut param_count = 0;

    if query.user_id.is_some() {
        param_count += 1;
        sql.push_str(&format!(" AND user_id = ${}", param_count));
    }

    if query.object_path.is_some() {
        param_count += 1;
        sql.push_str(&format!(" AND object_path = ${}", param_count));
    }

    if query.operation.is_some() {
        param_count += 1;
        sql.push_str(&format!(" AND operation = ${}", param_count));
    }

    if query.decision.is_some() {
        param_count += 1;
        sql.push_str(&format!(" AND decision = ${}", param_count));
    }

    if query.from.is_some() {
        param_count += 1;
        sql.push_str(&format!(" AND created_at >= ${}", param_count));
    }

    if query.to.is_some() {
        param_count += 1;
        sql.push_str(&format!(" AND created_at <= ${}", param_count));
    }

    sql.push_str(&format!(
        " ORDER BY created_at DESC LIMIT ${}",
        param_count + 1
    ));
    sql.push_str(&format!(" OFFSET ${}", param_count + 2));

    let mut db_query = sqlx::query_as::<_, DbAuditEvent>(AssertSqlSafe(sql.as_str()));

    if let Some(user_id) = query.user_id {
        db_query = db_query.bind(user_id);
    }

    if let Some(ref object_path) = query.object_path {
        db_query = db_query.bind(object_path);
    }

    if let Some(ref operation) = query.operation {
        db_query = db_query.bind(operation);
    }

    if let Some(ref decision) = query.decision {
        db_query = db_query.bind(decision);
    }

    if let Some(from) = query.from {
        db_query = db_query.bind(from);
    }

    if let Some(to) = query.to {
        db_query = db_query.bind(to);
    }

    db_query = db_query.bind(limit).bind(offset);

    let entries = db_query
        .fetch_all(pool)
        .await
        .map_err(|e| AuditQueryError::DatabaseError(e.to_string()))?;

    Ok(entries.into_iter().map(|e| e.into_access_event()).collect())
}

#[derive(Debug, thiserror::Error)]
pub enum AuditQueryError {
    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Invalid query parameters: {0}")]
    InvalidParams(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_query_default() {
        let query = EventQuery::default();
        assert_eq!(query.user_id, None);
        assert_eq!(query.object_path, None);
        assert_eq!(query.operation, None);
        assert_eq!(query.decision, None);
        assert_eq!(query.from, None);
        assert_eq!(query.to, None);
        assert_eq!(query.page, None);
        assert_eq!(query.limit, None);
    }

    #[test]
    fn test_event_query_offset() {
        let query = EventQuery {
            page: Some(1),
            limit: Some(50),
            ..Default::default()
        };
        assert_eq!(query.offset(), 0);

        let query = EventQuery {
            page: Some(2),
            limit: Some(50),
            ..Default::default()
        };
        assert_eq!(query.offset(), 50);

        let query = EventQuery {
            page: Some(3),
            limit: Some(100),
            ..Default::default()
        };
        assert_eq!(query.offset(), 200);
    }

    #[test]
    fn test_event_query_effective_limit() {
        let query = EventQuery {
            limit: Some(50),
            ..Default::default()
        };
        assert_eq!(query.effective_limit(), 50);

        let query = EventQuery {
            limit: Some(2000),
            ..Default::default()
        };
        assert_eq!(query.effective_limit(), 1000);

        let query = EventQuery {
            limit: Some(0),
            ..Default::default()
        };
        assert_eq!(query.effective_limit(), 1);

        let query = EventQuery::default();
        assert_eq!(query.effective_limit(), 50);
    }

    #[test]
    fn test_event_query_page_bounds() {
        let query = EventQuery {
            page: Some(0),
            ..Default::default()
        };
        assert_eq!(query.offset(), 0);

        let query = EventQuery {
            page: Some(-1),
            ..Default::default()
        };
        assert_eq!(query.offset(), 0);
    }
}
