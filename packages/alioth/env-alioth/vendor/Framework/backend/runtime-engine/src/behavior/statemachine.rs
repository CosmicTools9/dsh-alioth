//! State Machine Module
//!
//! Provides state machine definitions for entity lifecycle management.
//!
//! ## Example DSL
//!
//! ```dsl
//! @statemachine
//! @states([Pending, Confirmed, Shipped, Delivered, Cancelled])
//! entity Order {
//!     status: OrderStatus
//! }
//! ```

use serde::{Deserialize, Serialize};

/// State machine definition for an entity
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateMachine {
    /// Whether this entity has a state machine
    #[serde(default)]
    pub enabled: bool,
    /// List of all possible states
    #[serde(default)]
    pub states: Vec<State>,
    /// Initial state when entity is created
    #[serde(default)]
    pub initial_state: Option<String>,
    /// State field name (e.g., "status")
    #[serde(default)]
    pub state_field: Option<String>,
}

/// A single state in the state machine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    /// State name (e.g., "Pending", "Confirmed")
    pub name: String,
    /// Optional state description
    #[serde(default)]
    pub description: Option<String>,
    /// Whether this is a terminal/final state
    #[serde(default)]
    pub is_final: bool,
    /// Entry action name (lifecycle hook)
    #[serde(default)]
    pub on_entry: Option<String>,
    /// Exit action name (lifecycle hook)
    #[serde(default)]
    pub on_exit: Option<String>,
}

impl State {
    /// Create a new state with the given name
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            is_final: false,
            on_entry: None,
            on_exit: None,
        }
    }

    /// Set the state description
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Mark this state as final
    pub fn as_final(mut self) -> Self {
        self.is_final = true;
        self
    }

    /// Set entry action
    pub fn with_on_entry(mut self, action: impl Into<String>) -> Self {
        self.on_entry = Some(action.into());
        self
    }

    /// Set exit action
    pub fn with_on_exit(mut self, action: impl Into<String>) -> Self {
        self.on_exit = Some(action.into());
        self
    }
}

/// Parse states from @states annotation parameters
pub fn parse_states(params: &[String]) -> Vec<State> {
    params.iter().map(|name| State::new(name.clone())).collect()
}

/// Validate state machine definition
pub fn validate_state_machine(sm: &StateMachine) -> Result<(), StateMachineError> {
    if !sm.enabled {
        return Ok(());
    }

    if sm.states.is_empty() {
        return Err(StateMachineError::NoStatesDefined);
    }

    // Check for duplicate state names
    let mut seen = std::collections::HashSet::new();
    for state in &sm.states {
        if !seen.insert(&state.name) {
            return Err(StateMachineError::DuplicateState(state.name.clone()));
        }
    }

    // Validate initial state exists
    if let Some(ref initial) = sm.initial_state {
        if !sm.states.iter().any(|s| &s.name == initial) {
            return Err(StateMachineError::InvalidInitialState(initial.clone()));
        }
    }

    Ok(())
}

/// State machine validation errors
#[derive(Debug, Clone, PartialEq)]
pub enum StateMachineError {
    NoStatesDefined,
    DuplicateState(String),
    InvalidInitialState(String),
    StateNotFound(String),
}

impl std::fmt::Display for StateMachineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateMachineError::NoStatesDefined => {
                write!(f, "State machine enabled but no states defined")
            }
            StateMachineError::DuplicateState(name) => {
                write!(f, "Duplicate state name: {}", name)
            }
            StateMachineError::InvalidInitialState(name) => {
                write!(f, "Initial state '{}' not found in states list", name)
            }
            StateMachineError::StateNotFound(name) => {
                write!(f, "State '{}' not found", name)
            }
        }
    }
}

impl std::error::Error for StateMachineError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_new() {
        let state = State::new("Pending");
        assert_eq!(state.name, "Pending");
        assert!(!state.is_final);
    }

    #[test]
    fn test_state_builder() {
        let state = State::new("Completed")
            .with_description("Order is completed")
            .as_final()
            .with_on_entry("notifyCompletion");

        assert_eq!(state.name, "Completed");
        assert_eq!(state.description, Some("Order is completed".to_string()));
        assert!(state.is_final);
        assert_eq!(state.on_entry, Some("notifyCompletion".to_string()));
    }

    #[test]
    fn test_parse_states() {
        let params = vec![
            "Pending".to_string(),
            "Confirmed".to_string(),
            "Shipped".to_string(),
        ];
        let states = parse_states(&params);

        assert_eq!(states.len(), 3);
        assert_eq!(states[0].name, "Pending");
        assert_eq!(states[1].name, "Confirmed");
        assert_eq!(states[2].name, "Shipped");
    }

    #[test]
    fn test_validate_empty_states() {
        let sm = StateMachine {
            enabled: true,
            states: vec![],
            initial_state: None,
            state_field: None,
        };

        assert!(matches!(
            validate_state_machine(&sm),
            Err(StateMachineError::NoStatesDefined)
        ));
    }

    #[test]
    fn test_validate_duplicate_states() {
        let sm = StateMachine {
            enabled: true,
            states: vec![State::new("Pending"), State::new("Pending")],
            initial_state: None,
            state_field: None,
        };

        assert!(matches!(
            validate_state_machine(&sm),
            Err(StateMachineError::DuplicateState(_))
        ));
    }

    #[test]
    fn test_validate_invalid_initial_state() {
        let sm = StateMachine {
            enabled: true,
            states: vec![State::new("Pending")],
            initial_state: Some("NonExistent".to_string()),
            state_field: None,
        };

        assert!(matches!(
            validate_state_machine(&sm),
            Err(StateMachineError::InvalidInitialState(_))
        ));
    }

    #[test]
    fn test_validate_valid_state_machine() {
        let sm = StateMachine {
            enabled: true,
            states: vec![State::new("Pending"), State::new("Confirmed")],
            initial_state: Some("Pending".to_string()),
            state_field: Some("status".to_string()),
        };

        assert!(validate_state_machine(&sm).is_ok());
    }

    #[test]
    fn test_disabled_state_machine_skips_validation() {
        let sm = StateMachine {
            enabled: false,
            states: vec![],
            initial_state: None,
            state_field: None,
        };

        assert!(validate_state_machine(&sm).is_ok());
    }
}
