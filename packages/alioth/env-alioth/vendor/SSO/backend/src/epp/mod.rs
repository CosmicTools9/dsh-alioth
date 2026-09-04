//! EPP (Event Processing Point) module
//!
//! Receives events from the PEP and processes them for audit logging.
//! Events are validated, enriched, and forwarded to the audit system.

pub mod event_handler;

pub use event_handler::{AccessEvent, EventHandler, EventHandlerError, ObligationResult};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Event type enum for categorizing events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum EventType {
    Access,
    PolicyChange,
    UserCreated,
    UserModified,
    UserDeleted,
    ObjectCreated,
    ObjectModified,
    ObjectDeleted,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventType::Access => "ACCESS",
            EventType::PolicyChange => "POLICY_CHANGE",
            EventType::UserCreated => "USER_CREATED",
            EventType::UserModified => "USER_MODIFIED",
            EventType::UserDeleted => "USER_DELETED",
            EventType::ObjectCreated => "OBJECT_CREATED",
            EventType::ObjectModified => "OBJECT_MODIFIED",
            EventType::ObjectDeleted => "OBJECT_DELETED",
        }
    }
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NgacEvent {
    #[serde(with = "common::serde_zuid")]
    pub event_id: i64,
    pub timestamp: DateTime<Utc>,
    pub operation: String,
    #[serde(with = "common::serde_zuid")]
    pub subject_id: i64,
    #[serde(with = "common::serde_zuid")]
    pub object_id: i64,
    pub object_type: String,
    pub decision: String,
    pub success: bool,
    pub metadata: Option<serde_json::Value>,
}

impl NgacEvent {
    pub fn new(
        operation: String,
        subject_id: i64,
        object_id: i64,
        object_type: String,
        decision: String,
        success: bool,
    ) -> Result<Self, NgacEventError> {
        if operation.is_empty() {
            return Err(NgacEventError::InvalidOperation);
        }

        let decision_lower = decision.to_lowercase();
        if !["permit", "deny", "not_applicable"].contains(&decision_lower.as_str()) {
            return Err(NgacEventError::InvalidDecision(decision));
        }

        Ok(Self {
            event_id: 0,
            timestamp: Utc::now(),
            operation,
            subject_id,
            object_id,
            object_type,
            decision,
            success,
            metadata: None,
        })
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum NgacEventError {
    #[error("operation cannot be empty")]
    InvalidOperation,

    #[error("invalid decision: {0}")]
    InvalidDecision(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_type_as_str() {
        assert_eq!(EventType::Access.as_str(), "ACCESS");
        assert_eq!(EventType::PolicyChange.as_str(), "POLICY_CHANGE");
        assert_eq!(EventType::UserCreated.as_str(), "USER_CREATED");
        assert_eq!(EventType::ObjectCreated.as_str(), "OBJECT_CREATED");
    }

    #[test]
    fn test_event_type_display() {
        assert_eq!(format!("{}", EventType::Access), "ACCESS");
        assert_eq!(format!("{}", EventType::PolicyChange), "POLICY_CHANGE");
        assert_eq!(format!("{}", EventType::UserCreated), "USER_CREATED");
        assert_eq!(format!("{}", EventType::ObjectCreated), "OBJECT_CREATED");
    }

    #[test]
    fn test_ngac_event_new() {
        let event = NgacEvent::new(
            "read".to_string(),
            1,
            2,
            "projects/test".to_string(),
            "permit".to_string(),
            true,
        )
        .unwrap();

        assert_eq!(event.operation, "read");
        assert_eq!(event.decision, "permit");
        assert!(event.success);
        assert!(event.metadata.is_none());
    }

    #[test]
    fn test_ngac_event_with_metadata() {
        let event = NgacEvent::new(
            "read".to_string(),
            1,
            2,
            "projects/test".to_string(),
            "permit".to_string(),
            true,
        )
        .unwrap()
        .with_metadata(serde_json::json!({"key": "value"}));

        assert!(event.metadata.is_some());
        assert_eq!(event.metadata.unwrap()["key"], "value");
    }

    #[test]
    fn test_ngac_event_invalid_operation() {
        let result = NgacEvent::new(
            "".to_string(),
            1,
            2,
            "projects/test".to_string(),
            "permit".to_string(),
            true,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_ngac_event_invalid_decision() {
        let result = NgacEvent::new(
            "read".to_string(),
            1,
            2,
            "projects/test".to_string(),
            "invalid".to_string(),
            true,
        );

        assert!(result.is_err());
    }
}
