use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Access Right association following NGAC POLICY-03
/// Links subject attributes to object attributes with permitted operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessRight {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub user_attribute_id: i64,
    #[serde(with = "common::serde_zuid")]
    pub object_attribute_id: i64,
    pub action: String,
    pub conditions: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

impl AccessRight {
    pub fn new(
        user_attribute_id: i64,
        object_attribute_id: i64,
        action: impl Into<String>,
    ) -> Self {
        Self {
            id: 0,
            user_attribute_id,
            object_attribute_id,
            action: action.into(),
            conditions: None,
            created_at: Utc::now(),
        }
    }

    pub fn with_conditions(mut self, conditions: serde_json::Value) -> Self {
        self.conditions = Some(conditions);
        self
    }

    /// Check if this access right matches the given subject, object, and operation
    pub fn matches(
        &self,
        user_attribute_id: i64,
        object_attribute_id: i64,
        operation: &str,
    ) -> bool {
        self.user_attribute_id == user_attribute_id
            && self.object_attribute_id == object_attribute_id
            && self.action == operation
    }

    /// Check if conditions are satisfied (if any)
    pub fn conditions_satisfied(&self, _context: &serde_json::Value) -> bool {
        // If no conditions, access is granted
        // If conditions exist, they should be evaluated against context
        // This is a simplified implementation - full condition evaluation
        // would check time-based, environment-based conditions from JSONB
        self.conditions.is_none()
    }
}

/// CRUD operations supported by access rights
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Operation {
    Create,
    Read,
    Update,
    Delete,
}

impl Operation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Operation::Create => "create",
            Operation::Read => "read",
            Operation::Update => "update",
            Operation::Delete => "delete",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "create" => Some(Operation::Create),
            "read" => Some(Operation::Read),
            "update" => Some(Operation::Update),
            "delete" => Some(Operation::Delete),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_access_right_creation() {
        let user_attr_id = 100i64;
        let obj_attr_id = 200i64;

        let ar = AccessRight::new(user_attr_id, obj_attr_id, "read");

        assert_eq!(ar.user_attribute_id, user_attr_id);
        assert_eq!(ar.object_attribute_id, obj_attr_id);
        assert_eq!(ar.action, "read");
        assert!(ar.conditions.is_none());
    }

    #[test]
    fn test_access_right_matches() {
        let user_attr_id = 100i64;
        let obj_attr_id = 200i64;

        let ar = AccessRight::new(user_attr_id, obj_attr_id, "read");

        assert!(ar.matches(user_attr_id, obj_attr_id, "read"));
        assert!(!ar.matches(user_attr_id, obj_attr_id, "write"));
        assert!(!ar.matches(300i64, obj_attr_id, "read"));
    }

    #[test]
    fn test_operation_from_str() {
        assert_eq!(Operation::parse("create"), Some(Operation::Create));
        assert_eq!(Operation::parse("READ"), Some(Operation::Read));
        assert_eq!(Operation::parse("Update"), Some(Operation::Update));
        assert_eq!(Operation::parse("delete"), Some(Operation::Delete));
        assert_eq!(Operation::parse("unknown"), None);
    }
}
