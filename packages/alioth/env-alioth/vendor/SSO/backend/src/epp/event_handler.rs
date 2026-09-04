//! Event handling logic for EPP
//!
//! Processes access events, records them to the database,
//! and triggers obligations for permitted access.

use super::NgacEvent;
use crate::ngac::pdp::Decision;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// Access event representing a single access request and decision
#[derive(Debug, Clone)]
pub struct AccessEvent {
    pub id: i64,
    pub user_id: i64,
    pub user_email: Option<String>,
    pub object_path: String,
    pub operation: String,
    pub decision: Decision,
    pub subject_attributes: Vec<String>,
    pub object_attributes: Vec<String>,
    pub obligations_triggered: Vec<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub timestamp: DateTime<Utc>,
}

impl AccessEvent {
    pub fn new(
        user_id: i64,
        user_email: Option<String>,
        object_path: String,
        operation: String,
        decision: Decision,
    ) -> Self {
        Self {
            id: 0,
            user_id,
            user_email,
            object_path,
            operation,
            decision,
            subject_attributes: Vec::new(),
            object_attributes: Vec::new(),
            obligations_triggered: Vec::new(),
            ip_address: None,
            user_agent: None,
            timestamp: Utc::now(),
        }
    }

    pub fn with_subject_attributes(mut self, attrs: Vec<String>) -> Self {
        self.subject_attributes = attrs;
        self
    }

    pub fn with_object_attributes(mut self, attrs: Vec<String>) -> Self {
        self.object_attributes = attrs;
        self
    }

    pub fn with_ip_address(mut self, ip: String) -> Self {
        self.ip_address = Some(ip);
        self
    }

    pub fn with_user_agent(mut self, ua: String) -> Self {
        self.user_agent = Some(ua);
        self
    }

    pub fn with_obligations(mut self, obligations: Vec<String>) -> Self {
        self.obligations_triggered = obligations;
        self
    }
}

impl From<NgacEvent> for AccessEvent {
    fn from(event: NgacEvent) -> Self {
        let decision = match event.decision.to_lowercase().as_str() {
            "permit" => Decision::Permit,
            "deny" => Decision::Deny,
            _ => Decision::NotApplicable,
        };

        Self {
            id: event.event_id,
            user_id: event.subject_id,
            // NgacEvent 不携带 email；audit_events.user_email 为 NOT NULL，
            // 沿用 handlers.rs 的占位值惯例（满足约束，避免 null-value 违规）。
            user_email: Some("audit@local".to_string()),
            object_path: event.object_type,
            operation: event.operation,
            decision,
            subject_attributes: Vec::new(),
            object_attributes: Vec::new(),
            obligations_triggered: Vec::new(),
            ip_address: None,
            user_agent: None,
            timestamp: event.timestamp,
        }
    }
}

/// Result of obligation processing
#[derive(Debug, Clone)]
pub struct ObligationResult {
    pub obligation_id: String,
    pub action_type: String,
    pub success: bool,
    pub error_message: Option<String>,
}

/// Event handler for processing access events
#[derive(Clone)]
pub struct EventHandler {
    pool: PgPool,
}

impl EventHandler {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Record an access event to the database
    pub async fn record_access_event(&self, event: AccessEvent) -> Result<i64, EventHandlerError> {
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO isahl_audit.audit_events (user_id, user_email, object_path, operation, decision, subject_attributes, object_attributes, obligations_triggered, ip_address, user_agent, created_at)
             VALUES ($1, $2, $3, $4, $5, $6::ag_catalog.ltree[], $7::ag_catalog.ltree[], $8, $9, $10, $11) RETURNING id"
        )
        .bind(event.user_id)
        .bind(&event.user_email)
        .bind(&event.object_path)
        .bind(&event.operation)
        .bind(event.decision.to_string())
        .bind(&event.subject_attributes)
        .bind(&event.object_attributes)
        .bind(&event.obligations_triggered)
        .bind(&event.ip_address)
        .bind(&event.user_agent)
        .bind(event.timestamp)
        .fetch_one(&self.pool)
        .await
        .map_err(EventHandlerError::DatabaseError)
    }

    pub async fn handle_access_event(&self, event: AccessEvent) -> Result<i64, EventHandlerError> {
        let event_id = event.id;

        let pool = self.pool.clone();
        tokio::spawn(async move {
            let handler = EventHandler { pool };
            if let Err(e) = handler.record_access_event(event).await {
                log::error!("Failed to record access event: {}", e);
            }
        });

        Ok(event_id)
    }

    /// Record event synchronously (for testing)
    pub async fn handle_access_event_sync(
        &self,
        event: AccessEvent,
    ) -> Result<i64, EventHandlerError> {
        let event_id = event.id;
        self.record_access_event(event).await?;
        Ok(event_id)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EventHandlerError {
    #[error("Database error: {0}")]
    DatabaseError(sqlx::Error),

    #[error("Invalid event data: {0}")]
    InvalidEvent(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_access_event_new() {
        let user_id = 1;
        let event = AccessEvent::new(
            user_id,
            Some("user@example.com".to_string()),
            "projects/test".to_string(),
            "read".to_string(),
            Decision::Permit,
        );

        assert_eq!(event.user_id, user_id);
        assert_eq!(event.user_email, Some("user@example.com".to_string()));
        assert_eq!(event.object_path, "projects/test");
        assert_eq!(event.operation, "read");
        assert_eq!(event.decision, Decision::Permit);
        assert!(event.subject_attributes.is_empty());
        assert!(event.object_attributes.is_empty());
        assert!(event.obligations_triggered.is_empty());
        assert!(event.ip_address.is_none());
        assert!(event.user_agent.is_none());
    }

    #[test]
    fn test_access_event_with_subject_attributes() {
        let user_id = 1;
        let event = AccessEvent::new(
            user_id,
            Some("user@example.com".to_string()),
            "projects/test".to_string(),
            "read".to_string(),
            Decision::Permit,
        )
        .with_subject_attributes(vec!["org.admin".to_string(), "team.lead".to_string()]);

        assert_eq!(event.subject_attributes.len(), 2);
        assert_eq!(event.subject_attributes[0], "org.admin");
        assert_eq!(event.subject_attributes[1], "team.lead");
    }

    #[test]
    fn test_access_event_with_object_attributes() {
        let user_id = 1;
        let event = AccessEvent::new(
            user_id,
            Some("user@example.com".to_string()),
            "projects/test".to_string(),
            "read".to_string(),
            Decision::Permit,
        )
        .with_object_attributes(vec!["project.confidential".to_string()]);

        assert_eq!(event.object_attributes.len(), 1);
        assert_eq!(event.object_attributes[0], "project.confidential");
    }

    #[test]
    fn test_access_event_with_ip_address() {
        let user_id = 1;
        let event = AccessEvent::new(
            user_id,
            Some("user@example.com".to_string()),
            "projects/test".to_string(),
            "read".to_string(),
            Decision::Permit,
        )
        .with_ip_address("192.168.1.1".to_string());

        assert_eq!(event.ip_address, Some("192.168.1.1".to_string()));
    }

    #[test]
    fn test_access_event_with_user_agent() {
        let user_id = 1;
        let event = AccessEvent::new(
            user_id,
            Some("user@example.com".to_string()),
            "projects/test".to_string(),
            "read".to_string(),
            Decision::Permit,
        )
        .with_user_agent("Mozilla/5.0".to_string());

        assert_eq!(event.user_agent, Some("Mozilla/5.0".to_string()));
    }

    #[test]
    fn test_access_event_with_obligations() {
        let user_id = 1;
        let event = AccessEvent::new(
            user_id,
            Some("user@example.com".to_string()),
            "projects/test".to_string(),
            "read".to_string(),
            Decision::Permit,
        )
        .with_obligations(vec!["audit:log".to_string(), "notify:admin".to_string()]);

        assert_eq!(event.obligations_triggered.len(), 2);
        assert_eq!(event.obligations_triggered[0], "audit:log");
        assert_eq!(event.obligations_triggered[1], "notify:admin");
    }

    #[test]
    fn test_access_event_from_ngac_event() {
        let ngac_event = NgacEvent::new(
            "read".to_string(),
            1,
            1,
            "projects/test".to_string(),
            "permit".to_string(),
            true,
        )
        .unwrap();

        let access_event = AccessEvent::from(ngac_event.clone());

        assert_eq!(access_event.id, ngac_event.event_id);
        assert_eq!(access_event.user_id, ngac_event.subject_id);
        assert_eq!(access_event.object_path, ngac_event.object_type);
        assert_eq!(access_event.operation, ngac_event.operation);
        assert_eq!(access_event.decision, Decision::Permit);
    }

    #[test]
    fn test_obligation_result() {
        let result = ObligationResult {
            obligation_id: "audit:log".to_string(),
            action_type: "audit".to_string(),
            success: true,
            error_message: None,
        };

        assert_eq!(result.obligation_id, "audit:log");
        assert_eq!(result.action_type, "audit");
        assert!(result.success);
        assert!(result.error_message.is_none());
    }
}

#[cfg(test)]
mod tests2 {
    use super::*;

    #[test]
    fn test_access_event_new() {
        let event = AccessEvent::new(
            1,
            Some("test@example.com".to_string()),
            "document:123".to_string(),
            "read".to_string(),
            Decision::Permit,
        );

        assert_eq!(event.operation, "read");
        assert_eq!(event.decision, Decision::Permit);
        assert!(event.subject_attributes.is_empty());
        assert!(event.obligations_triggered.is_empty());
    }

    #[test]
    fn test_access_event_with_attributes() {
        let event = AccessEvent::new(
            1,
            Some("test@example.com".to_string()),
            "document:123".to_string(),
            "read".to_string(),
            Decision::Permit,
        )
        .with_subject_attributes(vec!["role:admin".to_string()])
        .with_object_attributes(vec!["classification:public".to_string()])
        .with_obligations(vec!["audit:log".to_string()])
        .with_ip_address("192.168.1.1".to_string())
        .with_user_agent("Mozilla/5.0".to_string());

        assert_eq!(event.subject_attributes.len(), 1);
        assert_eq!(event.object_attributes.len(), 1);
        assert_eq!(event.obligations_triggered.len(), 1);
        assert_eq!(event.ip_address, Some("192.168.1.1".to_string()));
        assert_eq!(event.user_agent, Some("Mozilla/5.0".to_string()));
    }

    #[test]
    fn test_access_event_builder_pattern() {
        let event = AccessEvent::new(
            1,
            None,
            "resource:123".to_string(),
            "write".to_string(),
            Decision::Deny,
        );

        assert!(event.user_email.is_none());
        assert_eq!(event.decision, Decision::Deny);
    }

    #[test]
    fn test_event_handler_new() {
        // Note: This test would require a real database connection
        // For now, we just test that the constructor compiles
        // let handler = EventHandler::new(pool);
        // assert!(true);
    }

    #[test]
    fn test_obligation_result() {
        let result = ObligationResult {
            obligation_id: "test:123".to_string(),
            action_type: "audit".to_string(),
            success: true,
            error_message: None,
        };

        assert_eq!(result.obligation_id, "test:123");
        assert_eq!(result.action_type, "audit");
        assert!(result.success);
        assert!(result.error_message.is_none());
    }

    #[test]
    fn test_event_handler_error_display() {
        let error = EventHandlerError::InvalidEvent("test error".to_string());
        assert_eq!(format!("{}", error), "Invalid event data: test error");

        let db_error = sqlx::Error::RowNotFound;
        let error = EventHandlerError::DatabaseError(db_error);
        assert!(format!("{}", error).contains("Database error"));
    }
}
