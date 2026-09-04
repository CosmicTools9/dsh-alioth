//! Audit event handlers
//!
//! Handles the persistence of audit events to the database.

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};

use crate::auth::jwt::{validate_access_token, Claims};
use crate::auth::AuthState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditEventType {
    AccessAllowed,
    AccessDenied,
    PolicyChanged,
    UserCreated,
    UserModified,
    UserDeleted,
    ObjectCreated,
    ObjectModified,
    ObjectDeleted,
}

impl AuditEventType {
    pub fn from_ngac_event(operation: &str, decision: &str, success: bool) -> Self {
        match (
            operation.to_lowercase().as_str(),
            decision.to_lowercase().as_str(),
            success,
        ) {
            ("create", _, true) => AuditEventType::ObjectCreated,
            ("update", _, true) => AuditEventType::ObjectModified,
            ("delete", _, true) => AuditEventType::ObjectDeleted,
            ("read", "allow", _) | ("read", "permit", _) => AuditEventType::AccessAllowed,
            ("write", "allow", _) | ("write", "permit", _) => AuditEventType::AccessAllowed,
            ("delete", "allow", _) | ("delete", "permit", _) => AuditEventType::AccessAllowed,
            (_, "deny", _) | (_, "forbid", _) | (_, "reject", _) => AuditEventType::AccessDenied,
            _ => AuditEventType::AccessDenied,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AuditEventType::AccessAllowed => "ACCESS_ALLOWED",
            AuditEventType::AccessDenied => "ACCESS_DENIED",
            AuditEventType::PolicyChanged => "POLICY_CHANGED",
            AuditEventType::UserCreated => "USER_CREATED",
            AuditEventType::UserModified => "USER_MODIFIED",
            AuditEventType::UserDeleted => "USER_DELETED",
            AuditEventType::ObjectCreated => "OBJECT_CREATED",
            AuditEventType::ObjectModified => "OBJECT_MODIFIED",
            AuditEventType::ObjectDeleted => "OBJECT_DELETED",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEventRecord {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub event_type: AuditEventType,
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

impl AuditEventRecord {
    pub fn from_ngac_event(event: crate::epp::NgacEvent) -> Self {
        let event_type =
            AuditEventType::from_ngac_event(&event.operation, &event.decision, event.success);

        Self {
            id: event.event_id,
            event_type,
            timestamp: event.timestamp,
            subject_id: event.subject_id,
            object_id: event.object_id,
            object_type: event.object_type,
            operation: event.operation,
            success: event.success,
            metadata: event.metadata,
        }
    }
}

pub async fn handle_audit_event(
    record: AuditEventRecord,
    pool: &PgPool,
) -> Result<i64, AuditHandlerError> {
    // 将 object_id/object_type 合并到 metadata
    let mut metadata = record.metadata.unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert("object_id".to_string(), serde_json::json!(record.object_id));
        obj.insert(
            "object_type".to_string(),
            serde_json::json!(&record.object_type),
        );
        obj.insert("success".to_string(), serde_json::json!(record.success));
        obj.insert(
            "event_type".to_string(),
            serde_json::json!(record.event_type.as_str()),
        );
    }

    // 通过 EPP EventHandler 记录访问事件（audit_events.id 由序列默认分配）。
    // user_email 在 AuditEventRecord 中不可用，使用占位值满足 NOT NULL 约束。
    let epp_decision = match record.event_type {
        AuditEventType::AccessAllowed => crate::ngac::pdp::Decision::Permit,
        AuditEventType::AccessDenied => crate::ngac::pdp::Decision::Deny,
        _ => crate::ngac::pdp::Decision::NotApplicable,
    };
    let access_event = crate::epp::AccessEvent::new(
        record.subject_id,
        Some("audit@local".to_string()),
        record.object_type.clone(),
        record.operation.clone(),
        epp_decision,
    );

    let event_id = crate::epp::EventHandler::new(pool.clone())
        .record_access_event(access_event)
        .await
        .map_err(|e| AuditHandlerError::DatabaseError(e.to_string()))?;

    // 补充 metadata（EPP 记录路径不携带 metadata 列）
    sqlx::query("UPDATE isahl_audit.audit_events SET metadata = $1 WHERE id = $2")
        .bind(metadata)
        .bind(event_id)
        .execute(pool)
        .await
        .map_err(|e| AuditHandlerError::DatabaseError(e.to_string()))?;

    Ok(event_id)
}

/// POST /api/ngac/audit — 审计事件摄入端点（NGAC 管控）
///
/// 作为 EPP 管道的兜底写入入口：已认证的调用方可主动提交一条访问事件，
/// 由 `handle_audit_event` 经 EPP `EventHandler` 持久化到 `audit_events`。
#[derive(Debug, Deserialize)]
pub struct IngestAuditRequest {
    #[serde(with = "common::serde_zuid")]
    pub subject_id: i64,
    pub object_type: String,
    pub operation: String,
    pub success: bool,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub object_id: Option<i64>,
    pub metadata: Option<serde_json::Value>,
}

pub async fn ingest_event(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    body: web::Json<IngestAuditRequest>,
) -> HttpResponse {
    if let Err(resp) = extract_claims(&req, &state).await {
        return resp;
    }
    let b = body.into_inner();
    let event_type = AuditEventType::from_ngac_event(
        &b.operation,
        if b.success { "permit" } else { "deny" },
        b.success,
    );
    let record = AuditEventRecord {
        id: 0,
        event_type,
        timestamp: Utc::now(),
        subject_id: b.subject_id,
        object_id: b.object_id.unwrap_or(0),
        object_type: b.object_type,
        operation: b.operation,
        success: b.success,
        metadata: b.metadata,
    };
    match handle_audit_event(record, pool.get_ref()).await {
        Ok(event_id) => HttpResponse::Created().json(serde_json::json!({ "id": event_id })),
        Err(e) => {
            HttpResponse::InternalServerError().json(serde_json::json!({ "error": e.to_string() }))
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuditHandlerError {
    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Invalid event data: {0}")]
    InvalidEvent(String),
}

/// Query parameters for listing audit events
#[derive(Debug, Deserialize)]
pub struct EventFilters {
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

/// Statistics response
#[derive(Debug, Serialize)]
pub struct AuditStats {
    #[serde(with = "common::serde_zuid")]
    pub total_events: i64,
    pub by_decision: serde_json::Value,
    pub by_operation: serde_json::Value,
    #[serde(with = "common::serde_zuid")]
    pub recent_failures: i64,
}

/// Cleanup query parameters
#[derive(Debug, Deserialize)]
pub struct CleanupFilters {
    #[serde(with = "common::serde_zuid::opt", default)]
    pub retention_days: Option<i64>,
}

/// Cleanup response
#[derive(Debug, Serialize)]
pub struct CleanupResult {
    #[serde(with = "common::serde_zuid")]
    pub deleted_count: i64,
}

/// Single event response
#[derive(Debug, Serialize)]
pub struct EventDetail {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub user_id: i64,
    pub user_email: Option<String>,
    pub object_path: String,
    pub operation: String,
    pub decision: String,
    pub subject_attributes: Vec<String>,
    pub object_attributes: Vec<String>,
    pub obligations_triggered: Vec<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub metadata: Option<serde_json::Value>,
}

/// Extract claims from request using the SSO ES256 public key.
async fn extract_claims(req: &HttpRequest, state: &AuthState) -> Result<Claims, HttpResponse> {
    validate_access_token(req, &state.verification_keys())
        .await
        .map_err(|e| {
            log::error!("Token validation error: {}", e);
            HttpResponse::Unauthorized().json(serde_json::json!({
                "error": "Invalid or missing authentication token"
            }))
        })
}

/// GET /admin/audit/events - List events with filtering
pub async fn list_events(
    req: HttpRequest,
    query: web::Query<EventFilters>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _claims = match extract_claims(&req, &state).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let page = query.page.unwrap_or(1).max(1);
    let offset = (page - 1) * limit;

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

    match db_query.fetch_all(pool.get_ref()).await {
        Ok(rows) => {
            let events: Vec<EventDetail> = rows.into_iter().map(|r| r.into_detail()).collect();
            HttpResponse::Ok().json(serde_json::json!({
                "events": events,
                "page": page,
                "limit": limit,
                "total": events.len()
            }))
        }
        Err(e) => {
            log::error!("Failed to list events: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to retrieve events"
            }))
        }
    }
}

/// GET /admin/audit/events/{id} - Get single event details
pub async fn get_event(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _claims = match extract_claims(&req, &state).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let event_id = path.into_inner();

    let sql = r#"
        SELECT id, user_id, user_email, object_path, operation, decision,
               subject_attributes::text[], object_attributes::text[], obligations_triggered,
               ip_address, user_agent, metadata, created_at
        FROM isahl_audit.audit_events
        WHERE id = $1
        "#;

    match sqlx::query_as::<_, DbAuditEvent>(sql)
        .bind(event_id)
        .fetch_optional(pool.get_ref())
        .await
    {
        Ok(Some(row)) => HttpResponse::Ok().json(row.into_detail()),
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Event not found"
        })),
        Err(e) => {
            log::error!("Failed to get event: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to retrieve event"
            }))
        }
    }
}

/// GET /admin/audit/stats - Get audit statistics
pub async fn get_stats(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _claims = match extract_claims(&req, &state).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let total_events =
        match sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM isahl_audit.audit_events")
            .fetch_one(pool.get_ref())
            .await
        {
            Ok(count) => count,
            Err(e) => {
                log::error!("Failed to get total events count: {}", e);
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "error": "Failed to retrieve statistics"
                }));
            }
        };

    let by_decision: Vec<(String, i64)> = match sqlx::query_as(
        "SELECT decision, COUNT(*) as count FROM isahl_audit.audit_events GROUP BY decision ORDER BY count DESC"
    )
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(results) => results,
        Err(e) => {
            log::error!("Failed to get decision stats: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to retrieve statistics"
            }));
        }
    };

    let by_operation: Vec<(String, i64)> = match sqlx::query_as(
        "SELECT operation, COUNT(*) as count FROM isahl_audit.audit_events GROUP BY operation ORDER BY count DESC"
    )
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(results) => results,
        Err(e) => {
            log::error!("Failed to get operation stats: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to retrieve statistics"
            }));
        }
    };

    let recent_failures = match sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM isahl_audit.audit_events WHERE decision = 'deny' AND created_at >= NOW() - INTERVAL '24 hours'"
    )
    .fetch_one(pool.get_ref())
    .await
    {
        Ok(count) => count,
        Err(e) => {
            log::error!("Failed to get recent failures count: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to retrieve statistics"
            }));
        }
    };

    let by_decision_map: serde_json::Value = serde_json::Map::from_iter(
        by_decision
            .into_iter()
            .map(|(k, v)| (k, serde_json::json!(v))),
    )
    .into();

    let by_operation_map: serde_json::Value = serde_json::Map::from_iter(
        by_operation
            .into_iter()
            .map(|(k, v)| (k, serde_json::json!(v))),
    )
    .into();

    HttpResponse::Ok().json(AuditStats {
        total_events,
        by_decision: by_decision_map,
        by_operation: by_operation_map,
        recent_failures,
    })
}

/// DELETE /admin/audit/events/cleanup - Cleanup old events
pub async fn cleanup_events(
    req: HttpRequest,
    query: web::Query<CleanupFilters>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _claims = match extract_claims(&req, &state).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let retention_days = query.retention_days.unwrap_or(90);

    let sql = r#"
        DELETE FROM isahl_audit.audit_events
        WHERE created_at < NOW() - INTERVAL '1 day' * $1
        "#;

    match sqlx::query(sql)
        .bind(retention_days)
        .execute(pool.get_ref())
        .await
    {
        Ok(result) => HttpResponse::Ok().json(CleanupResult {
            deleted_count: result.rows_affected() as i64,
        }),
        Err(e) => {
            log::error!("Failed to cleanup events: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to cleanup events"
            }))
        }
    }
}

/// Database row struct for audit events
#[derive(Debug, sqlx::FromRow)]
pub struct DbAuditEvent {
    pub id: i64,
    pub user_id: i64,
    pub user_email: String,
    pub object_path: String,
    pub operation: String,
    pub decision: String,
    pub subject_attributes: Vec<String>,
    pub object_attributes: Vec<String>,
    pub obligations_triggered: Vec<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

impl DbAuditEvent {
    pub fn into_detail(self) -> EventDetail {
        EventDetail {
            id: self.id,
            user_id: self.user_id,
            user_email: Some(self.user_email),
            object_path: self.object_path,
            operation: self.operation,
            decision: self.decision,
            subject_attributes: self.subject_attributes,
            object_attributes: self.object_attributes,
            obligations_triggered: self.obligations_triggered,
            ip_address: self.ip_address,
            user_agent: self.user_agent,
            timestamp: self.created_at,
            metadata: self.metadata,
        }
    }

    pub fn into_access_event(self) -> crate::epp::AccessEvent {
        use crate::ngac::pdp::Decision;

        let decision = match self.decision.to_lowercase().as_str() {
            "permit" => Decision::Permit,
            "deny" => Decision::Deny,
            _ => Decision::NotApplicable,
        };

        crate::epp::AccessEvent {
            id: self.id,
            user_id: self.user_id,
            user_email: Some(self.user_email),
            object_path: self.object_path,
            operation: self.operation,
            decision,
            subject_attributes: self.subject_attributes,
            object_attributes: self.object_attributes,
            obligations_triggered: self.obligations_triggered,
            ip_address: self.ip_address,
            user_agent: self.user_agent,
            timestamp: self.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_filters_deserialize() {
        let json = r#"{
            "user_id": 42,
            "object_path": "/projects/test",
            "operation": "read",
            "decision": "permit",
            "page": 2,
            "limit": 50
        }"#;

        let filters: EventFilters = serde_json::from_str(json).unwrap();
        assert_eq!(filters.user_id.unwrap(), 42);
        assert_eq!(filters.object_path, Some("/projects/test".to_string()));
        assert_eq!(filters.operation, Some("read".to_string()));
        assert_eq!(filters.decision, Some("permit".to_string()));
        assert_eq!(filters.page, Some(2));
        assert_eq!(filters.limit, Some(50));
    }

    #[test]
    fn test_event_filters_empty() {
        let json = r#"{}"#;
        let filters: EventFilters = serde_json::from_str(json).unwrap();
        assert_eq!(filters.user_id, None);
        assert_eq!(filters.object_path, None);
        assert_eq!(filters.operation, None);
        assert_eq!(filters.decision, None);
        assert_eq!(filters.page, None);
        assert_eq!(filters.limit, None);
    }

    #[test]
    fn test_cleanup_filters_deserialize() {
        let json = r#"{"retention_days": 30}"#;
        let filters: CleanupFilters = serde_json::from_str(json).unwrap();
        assert_eq!(filters.retention_days, Some(30));

        let json = r#"{}"#;
        let filters: CleanupFilters = serde_json::from_str(json).unwrap();
        assert_eq!(filters.retention_days, None);
    }

    #[test]
    fn test_audit_stats_serialization() {
        let stats = AuditStats {
            total_events: 100,
            by_decision: serde_json::json!({"permit": 80, "deny": 20}),
            by_operation: serde_json::json!({"read": 50, "write": 30, "delete": 20}),
            recent_failures: 5,
        };

        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("total_events"));
        assert!(json.contains("by_decision"));
        assert!(json.contains("by_operation"));
        assert!(json.contains("recent_failures"));
    }

    #[test]
    fn test_cleanup_result_serialization() {
        let result = CleanupResult { deleted_count: 50 };

        let json = serde_json::to_string(&result).unwrap();
        assert_eq!(json, r#"{"deleted_count":"50"}"#);
    }

    #[test]
    fn test_event_detail_serialization() {
        let detail = EventDetail {
            id: 1,
            user_id: 1,
            user_email: Some("test@example.com".to_string()),
            object_path: "/projects/test".to_string(),
            operation: "read".to_string(),
            decision: "permit".to_string(),
            subject_attributes: vec!["org.admin".to_string()],
            object_attributes: vec!["project.confidential".to_string()],
            obligations_triggered: vec!["audit:log".to_string()],
            ip_address: Some("192.168.1.1".to_string()),
            user_agent: Some("Mozilla/5.0".to_string()),
            timestamp: Utc::now(),
            metadata: Some(serde_json::json!({"key": "value"})),
        };

        let json = serde_json::to_string(&detail).unwrap();
        assert!(json.contains("user_email"));
        assert!(json.contains("object_path"));
        assert!(json.contains("subject_attributes"));
    }
}
