//! State Transitions Module
//!
//! Defines state transitions and their validation.
//!
//! ## Example DSL
//!
//! ```dsl
//! @transition(event: "confirm", from: Pending, to: Confirmed, guard: "paymentReceived")
//! @transition(event: "ship", from: Confirmed, to: Shipped)
//! @transition(event: "cancel", from: [Pending, Confirmed], to: Cancelled)
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A transition between states
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    /// Event name that triggers this transition (e.g., "confirm", "ship")
    pub event: String,
    /// Source state(s) - can be single state or multiple
    #[serde(default)]
    pub from: Vec<String>,
    /// Target state
    pub to: String,
    /// Guard condition (optional function name or expression)
    #[serde(default)]
    pub guard: Option<String>,
    /// Action to execute during transition (optional function name)
    #[serde(default)]
    pub action: Option<String>,
    /// Whether this transition is the default for the source state(s)
    #[serde(default)]
    pub is_default: bool,
}

impl Transition {
    /// Create a new transition
    pub fn new(event: impl Into<String>, from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            event: event.into(),
            from: vec![from.into()],
            to: to.into(),
            guard: None,
            action: None,
            is_default: false,
        }
    }

    /// Create a transition from multiple source states
    pub fn new_multi_from(
        event: impl Into<String>,
        from: Vec<String>,
        to: impl Into<String>,
    ) -> Self {
        Self {
            event: event.into(),
            from,
            to: to.into(),
            guard: None,
            action: None,
            is_default: false,
        }
    }

    /// Set guard condition
    pub fn with_guard(mut self, guard: impl Into<String>) -> Self {
        self.guard = Some(guard.into());
        self
    }

    /// Set action to execute
    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// Mark as default transition
    pub fn as_default(mut self) -> Self {
        self.is_default = true;
        self
    }

    /// Check if this transition can be triggered from the given state
    pub fn can_transition_from(&self, state: &str) -> bool {
        self.from.iter().any(|s| s == state)
    }
}

/// Collection of transitions for a state machine
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TransitionTable {
    /// All transitions indexed by event name
    #[serde(default)]
    pub transitions: HashMap<String, Vec<Transition>>,
    /// Transitions indexed by source state for quick lookup
    #[serde(default, skip_serializing)]
    pub by_source: HashMap<String, Vec<Transition>>,
}

impl TransitionTable {
    /// Create a new empty transition table
    pub fn new() -> Self {
        Self {
            transitions: HashMap::new(),
            by_source: HashMap::new(),
        }
    }

    /// Add a transition to the table
    pub fn add(&mut self, transition: Transition) {
        // Index by event
        self.transitions
            .entry(transition.event.clone())
            .or_default()
            .push(transition.clone());

        // Index by source state
        for from_state in &transition.from {
            self.by_source
                .entry(from_state.clone())
                .or_default()
                .push(transition.clone());
        }
    }

    /// Get all transitions for an event
    pub fn get_by_event(&self, event: &str) -> Option<&Vec<Transition>> {
        self.transitions.get(event)
    }

    /// Get all transitions from a specific state
    pub fn get_from_state(&self, state: &str) -> Option<&Vec<Transition>> {
        self.by_source.get(state)
    }

    /// Check if a transition is possible from current state via event
    pub fn can_transition(&self, current_state: &str, event: &str) -> bool {
        self.transitions
            .get(event)
            .map(|transitions| {
                transitions
                    .iter()
                    .any(|t| t.can_transition_from(current_state))
            })
            .unwrap_or(false)
    }

    /// Get the target state for a transition from current state via event
    pub fn get_target_state(&self, current_state: &str, event: &str) -> Option<String> {
        self.transitions.get(event).and_then(|transitions| {
            transitions
                .iter()
                .find(|t| t.can_transition_from(current_state))
                .map(|t| t.to.clone())
        })
    }

    /// Get all available events from current state
    pub fn available_events(&self, current_state: &str) -> Vec<String> {
        self.by_source
            .get(current_state)
            .map(|transitions| {
                transitions
                    .iter()
                    .map(|t| t.event.clone())
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Rebuild the by_source index (call after deserialization)
    pub fn rebuild_index(&mut self) {
        self.by_source.clear();
        for transitions in self.transitions.values() {
            for transition in transitions {
                for from_state in &transition.from {
                    self.by_source
                        .entry(from_state.clone())
                        .or_default()
                        .push(transition.clone());
                }
            }
        }
    }
}

/// Parse transition parameters from annotation
pub fn parse_transition_params(params: &HashMap<String, String>) -> Option<Transition> {
    let event = params.get("event")?;
    let to = params.get("to")?;

    // Parse "from" - can be single value or array
    let from = if let Some(from_str) = params.get("from") {
        // Check if it's an array format: [State1, State2]
        if from_str.starts_with('[') && from_str.ends_with(']') {
            // Parse array format
            let content = &from_str[1..from_str.len() - 1];
            content
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            vec![from_str.clone()]
        }
    } else {
        return None;
    };

    let guard = params.get("guard").cloned();
    let action = params.get("action").cloned();

    Some(Transition {
        event: event.clone(),
        from,
        to: to.clone(),
        guard,
        action,
        is_default: params.contains_key("default"),
    })
}

/// Validate transitions against defined states
pub fn validate_transitions(
    transitions: &[Transition],
    states: &[String],
) -> Result<(), TransitionError> {
    let state_set: std::collections::HashSet<_> = states.iter().collect();

    for transition in transitions {
        // Validate source states exist
        for from_state in &transition.from {
            if !state_set.contains(from_state) {
                return Err(TransitionError::InvalidSourceState {
                    event: transition.event.clone(),
                    state: from_state.clone(),
                });
            }
        }

        // Validate target state exists
        if !state_set.contains(&transition.to) {
            return Err(TransitionError::InvalidTargetState {
                event: transition.event.clone(),
                state: transition.to.clone(),
            });
        }

        // Check for self-loop warning (not an error)
        if transition.from.len() == 1 && transition.from[0] == transition.to {
            // Self-loop detected - this is valid but may be worth noting
        }
    }

    // Check for unreachable states (states with no incoming transitions)
    let mut has_incoming = std::collections::HashSet::new();
    has_incoming.insert(states[0].clone()); // Initial state always reachable

    for transition in transitions {
        has_incoming.insert(transition.to.clone());
    }

    let unreachable: Vec<_> = states
        .iter()
        .filter(|s| !has_incoming.contains(*s))
        .cloned()
        .collect();

    if !unreachable.is_empty() {
        return Err(TransitionError::UnreachableStates(unreachable));
    }

    Ok(())
}

/// Transition validation errors
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionError {
    InvalidSourceState { event: String, state: String },
    InvalidTargetState { event: String, state: String },
    UnreachableStates(Vec<String>),
    DuplicateTransition { event: String, from: String },
    ConflictingDefaultTransitions { state: String },
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransitionError::InvalidSourceState { event, state } => {
                write!(
                    f,
                    "Transition '{}' has invalid source state '{}'",
                    event, state
                )
            }
            TransitionError::InvalidTargetState { event, state } => {
                write!(
                    f,
                    "Transition '{}' has invalid target state '{}'",
                    event, state
                )
            }
            TransitionError::UnreachableStates(states) => {
                write!(f, "Unreachable states: {:?}", states)
            }
            TransitionError::DuplicateTransition { event, from } => {
                write!(f, "Duplicate transition '{}' from state '{}'", event, from)
            }
            TransitionError::ConflictingDefaultTransitions { state } => {
                write!(f, "Multiple default transitions for state '{}'", state)
            }
        }
    }
}

impl std::error::Error for TransitionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transition_new() {
        let t = Transition::new("confirm", "Pending", "Confirmed");
        assert_eq!(t.event, "confirm");
        assert_eq!(t.from, vec!["Pending"]);
        assert_eq!(t.to, "Confirmed");
    }

    #[test]
    fn test_transition_multi_from() {
        let t = Transition::new_multi_from(
            "cancel",
            vec!["Pending".to_string(), "Confirmed".to_string()],
            "Cancelled",
        );
        assert_eq!(t.event, "cancel");
        assert_eq!(t.from, vec!["Pending", "Confirmed"]);
        assert_eq!(t.to, "Cancelled");
    }

    #[test]
    fn test_transition_builder() {
        let t = Transition::new("confirm", "Pending", "Confirmed")
            .with_guard("paymentReceived")
            .with_action("sendConfirmation");

        assert_eq!(t.guard, Some("paymentReceived".to_string()));
        assert_eq!(t.action, Some("sendConfirmation".to_string()));
    }

    #[test]
    fn test_can_transition_from() {
        let t = Transition::new_multi_from(
            "cancel",
            vec!["Pending".to_string(), "Confirmed".to_string()],
            "Cancelled",
        );

        assert!(t.can_transition_from("Pending"));
        assert!(t.can_transition_from("Confirmed"));
        assert!(!t.can_transition_from("Shipped"));
    }

    #[test]
    fn test_transition_table() {
        let mut table = TransitionTable::new();

        table.add(Transition::new("confirm", "Pending", "Confirmed"));
        table.add(Transition::new("cancel", "Pending", "Cancelled"));
        table.add(Transition::new("cancel", "Confirmed", "Cancelled"));

        // Test get_by_event
        let confirm = table.get_by_event("confirm").unwrap();
        assert_eq!(confirm.len(), 1);

        // Test get_from_state
        let from_pending = table.get_from_state("Pending").unwrap();
        assert_eq!(from_pending.len(), 2);

        // Test can_transition
        assert!(table.can_transition("Pending", "confirm"));
        assert!(!table.can_transition("Confirmed", "confirm"));

        // Test get_target_state
        assert_eq!(
            table.get_target_state("Pending", "confirm"),
            Some("Confirmed".to_string())
        );

        // Test available_events
        let events = table.available_events("Pending");
        assert!(events.contains(&"confirm".to_string()));
        assert!(events.contains(&"cancel".to_string()));
    }

    #[test]
    fn test_parse_transition_params() {
        let mut params = HashMap::new();
        params.insert("event".to_string(), "confirm".to_string());
        params.insert("from".to_string(), "Pending".to_string());
        params.insert("to".to_string(), "Confirmed".to_string());
        params.insert("guard".to_string(), "paymentReceived".to_string());

        let t = parse_transition_params(&params).unwrap();
        assert_eq!(t.event, "confirm");
        assert_eq!(t.from, vec!["Pending"]);
        assert_eq!(t.to, "Confirmed");
        assert_eq!(t.guard, Some("paymentReceived".to_string()));
    }

    #[test]
    fn test_parse_transition_params_array_from() {
        let mut params = HashMap::new();
        params.insert("event".to_string(), "cancel".to_string());
        params.insert("from".to_string(), "[Pending, Confirmed]".to_string());
        params.insert("to".to_string(), "Cancelled".to_string());

        let t = parse_transition_params(&params).unwrap();
        assert_eq!(t.event, "cancel");
        assert_eq!(t.from, vec!["Pending", "Confirmed"]);
        assert_eq!(t.to, "Cancelled");
    }

    #[test]
    fn test_validate_valid_transitions() {
        let states = vec!["Pending".to_string(), "Confirmed".to_string()];
        let transitions = vec![Transition::new("confirm", "Pending", "Confirmed")];

        assert!(validate_transitions(&transitions, &states).is_ok());
    }

    #[test]
    fn test_validate_invalid_source() {
        let states = vec!["Pending".to_string(), "Confirmed".to_string()];
        let transitions = vec![Transition::new("confirm", "NonExistent", "Confirmed")];

        assert!(matches!(
            validate_transitions(&transitions, &states),
            Err(TransitionError::InvalidSourceState { .. })
        ));
    }

    #[test]
    fn test_validate_invalid_target() {
        let states = vec!["Pending".to_string(), "Confirmed".to_string()];
        let transitions = vec![Transition::new("confirm", "Pending", "NonExistent")];

        assert!(matches!(
            validate_transitions(&transitions, &states),
            Err(TransitionError::InvalidTargetState { .. })
        ));
    }

    #[test]
    fn test_validate_unreachable_states() {
        let states = vec![
            "Pending".to_string(),
            "Confirmed".to_string(),
            "Unreachable".to_string(),
        ];
        let transitions = vec![Transition::new("confirm", "Pending", "Confirmed")];

        assert!(matches!(
            validate_transitions(&transitions, &states),
            Err(TransitionError::UnreachableStates(_))
        ));
    }
}
