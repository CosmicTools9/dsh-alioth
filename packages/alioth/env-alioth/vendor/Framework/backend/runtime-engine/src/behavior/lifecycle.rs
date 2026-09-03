//! Lifecycle Hooks Module
//!
//! Defines lifecycle hooks for entity operations and state transitions.
//!
//! ## Example DSL
//!
//! ```dsl
//! @onCreate
//! fn initializeOrder() { }
//!
//! @onUpdate
//! fn validateOrder() { }
//!
//! @onDelete
//! fn cleanupOrder() { }
//!
//! @onTransition(from: Pending, to: Confirmed)
//! fn sendConfirmationEmail() { }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Lifecycle event types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum LifecycleEvent {
    /// Called when entity is created
    OnCreate,
    /// Called when entity is updated
    OnUpdate,
    /// Called when entity is deleted
    OnDelete,
    /// Called during state transition
    OnTransition,
}

impl std::fmt::Display for LifecycleEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LifecycleEvent::OnCreate => write!(f, "onCreate"),
            LifecycleEvent::OnUpdate => write!(f, "onUpdate"),
            LifecycleEvent::OnDelete => write!(f, "onDelete"),
            LifecycleEvent::OnTransition => write!(f, "onTransition"),
        }
    }
}

impl std::str::FromStr for LifecycleEvent {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "onCreate" | "on_create" => Ok(LifecycleEvent::OnCreate),
            "onUpdate" | "on_update" => Ok(LifecycleEvent::OnUpdate),
            "onDelete" | "on_delete" => Ok(LifecycleEvent::OnDelete),
            "onTransition" | "on_transition" => Ok(LifecycleEvent::OnTransition),
            _ => Err(format!("Unknown lifecycle event: {}", s)),
        }
    }
}

/// A lifecycle hook definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleHook {
    /// The lifecycle event type
    pub event: LifecycleEvent,
    /// Hook function name
    pub function_name: String,
    /// For onTransition: source state (optional)
    #[serde(default)]
    pub from_state: Option<String>,
    /// For onTransition: target state (optional)
    #[serde(default)]
    pub to_state: Option<String>,
    /// Execution order (lower = earlier)
    #[serde(default)]
    pub order: i32,
    /// Whether this hook is async
    #[serde(default)]
    pub is_async: bool,
    /// Additional parameters
    #[serde(default)]
    pub params: HashMap<String, String>,
}

impl LifecycleHook {
    /// Create a new lifecycle hook
    pub fn new(event: LifecycleEvent, function_name: impl Into<String>) -> Self {
        Self {
            event,
            function_name: function_name.into(),
            from_state: None,
            to_state: None,
            order: 0,
            is_async: false,
            params: HashMap::new(),
        }
    }

    /// Set the source state (for transition hooks)
    pub fn with_from_state(mut self, state: impl Into<String>) -> Self {
        self.from_state = Some(state.into());
        self
    }

    /// Set the target state (for transition hooks)
    pub fn with_to_state(mut self, state: impl Into<String>) -> Self {
        self.to_state = Some(state.into());
        self
    }

    /// Set execution order
    pub fn with_order(mut self, order: i32) -> Self {
        self.order = order;
        self
    }

    /// Mark as async
    pub fn as_async(mut self) -> Self {
        self.is_async = true;
        self
    }

    /// Check if this hook matches the given transition
    pub fn matches_transition(&self, from: &str, to: &str) -> bool {
        if self.event != LifecycleEvent::OnTransition {
            return false;
        }

        let from_matches = self.from_state.as_ref().map(|s| s == from).unwrap_or(true);
        let to_matches = self.to_state.as_ref().map(|s| s == to).unwrap_or(true);

        from_matches && to_matches
    }
}

/// Collection of lifecycle hooks for an entity
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LifecycleHooks {
    /// Hooks indexed by event type
    #[serde(default)]
    pub hooks: HashMap<LifecycleEvent, Vec<LifecycleHook>>,
}

impl LifecycleHooks {
    /// Create a new empty lifecycle hooks collection
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
        }
    }

    /// Add a hook
    pub fn add(&mut self, hook: LifecycleHook) {
        self.hooks.entry(hook.event).or_default().push(hook);
    }

    /// Get hooks for a specific event
    pub fn get_hooks(&self, event: LifecycleEvent) -> Vec<&LifecycleHook> {
        self.hooks
            .get(&event)
            .map(|hooks| hooks.iter().collect())
            .unwrap_or_default()
    }

    /// Get hooks for a specific transition
    pub fn get_transition_hooks(&self, from: &str, to: &str) -> Vec<&LifecycleHook> {
        self.hooks
            .get(&LifecycleEvent::OnTransition)
            .map(|hooks| {
                hooks
                    .iter()
                    .filter(|h| h.matches_transition(from, to))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all hooks sorted by execution order
    pub fn get_sorted_hooks(&self, event: LifecycleEvent) -> Vec<&LifecycleHook> {
        let mut hooks = self.get_hooks(event);
        hooks.sort_by_key(|h| h.order);
        hooks
    }

    /// Check if there are any hooks for an event
    pub fn has_hooks(&self, event: LifecycleEvent) -> bool {
        self.hooks
            .get(&event)
            .map(|h| !h.is_empty())
            .unwrap_or(false)
    }

    /// Get all hooks across all events
    pub fn all_hooks(&self) -> Vec<&LifecycleHook> {
        self.hooks.values().flatten().collect()
    }
}

/// Parse a lifecycle hook from annotation parameters
pub fn parse_lifecycle_hook(
    event: LifecycleEvent,
    function_name: impl Into<String>,
    params: &HashMap<String, String>,
) -> LifecycleHook {
    let mut hook = LifecycleHook::new(event, function_name);

    if let Some(from) = params.get("from") {
        hook.from_state = Some(from.clone());
    }

    if let Some(to) = params.get("to") {
        hook.to_state = Some(to.clone());
    }

    if let Some(order_str) = params.get("order") {
        if let Ok(order) = order_str.parse() {
            hook.order = order;
        }
    }

    hook.is_async = params.contains_key("async");
    hook.params = params.clone();

    hook
}

/// Hook invocation context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    /// Entity ID
    pub fk_entity: String,
    /// Entity type
    pub t_entity_type: String,
    /// Current state (if applicable)
    #[serde(default)]
    pub current_state: Option<String>,
    /// Previous state (if transition)
    #[serde(default)]
    pub previous_state: Option<String>,
    /// Operation timestamp
    pub timestamp: String,
    /// Additional context data
    #[serde(default)]
    pub data: HashMap<String, serde_json::Value>,
}

impl HookContext {
    /// Create a new hook context
    pub fn new(fk_entity: impl Into<String>, t_entity_type: impl Into<String>) -> Self {
        Self {
            fk_entity: fk_entity.into(),
            t_entity_type: t_entity_type.into(),
            current_state: None,
            previous_state: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            data: HashMap::new(),
        }
    }

    /// Set current state
    pub fn with_current_state(mut self, state: impl Into<String>) -> Self {
        self.current_state = Some(state.into());
        self
    }

    /// Set previous state
    pub fn with_previous_state(mut self, state: impl Into<String>) -> Self {
        self.previous_state = Some(state.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lifecycle_event_display() {
        assert_eq!(LifecycleEvent::OnCreate.to_string(), "onCreate");
        assert_eq!(LifecycleEvent::OnUpdate.to_string(), "onUpdate");
        assert_eq!(LifecycleEvent::OnDelete.to_string(), "onDelete");
        assert_eq!(LifecycleEvent::OnTransition.to_string(), "onTransition");
    }

    #[test]
    fn test_lifecycle_event_from_str() {
        assert_eq!(
            "onCreate".parse::<LifecycleEvent>().unwrap(),
            LifecycleEvent::OnCreate
        );
        assert_eq!(
            "on_create".parse::<LifecycleEvent>().unwrap(),
            LifecycleEvent::OnCreate
        );
        assert_eq!(
            "onTransition".parse::<LifecycleEvent>().unwrap(),
            LifecycleEvent::OnTransition
        );
        assert!("invalid".parse::<LifecycleEvent>().is_err());
    }

    #[test]
    fn test_lifecycle_hook_new() {
        let hook = LifecycleHook::new(LifecycleEvent::OnCreate, "initializeOrder");
        assert_eq!(hook.event, LifecycleEvent::OnCreate);
        assert_eq!(hook.function_name, "initializeOrder");
        assert!(hook.from_state.is_none());
        assert!(hook.to_state.is_none());
    }

    #[test]
    fn test_lifecycle_hook_transition() {
        let hook = LifecycleHook::new(LifecycleEvent::OnTransition, "sendEmail")
            .with_from_state("Pending")
            .with_to_state("Confirmed")
            .with_order(1)
            .as_async();

        assert_eq!(hook.from_state, Some("Pending".to_string()));
        assert_eq!(hook.to_state, Some("Confirmed".to_string()));
        assert_eq!(hook.order, 1);
        assert!(hook.is_async);
    }

    #[test]
    fn test_matches_transition() {
        let hook = LifecycleHook::new(LifecycleEvent::OnTransition, "sendEmail")
            .with_from_state("Pending")
            .with_to_state("Confirmed");

        assert!(hook.matches_transition("Pending", "Confirmed"));
        assert!(!hook.matches_transition("Confirmed", "Shipped"));
        assert!(!hook.matches_transition("Pending", "Cancelled"));

        // Wildcard hook (no from/to specified)
        let wildcard = LifecycleHook::new(LifecycleEvent::OnTransition, "logTransition");
        assert!(wildcard.matches_transition("Any", "State"));
    }

    #[test]
    fn test_lifecycle_hooks_collection() {
        let mut hooks = LifecycleHooks::new();

        hooks.add(LifecycleHook::new(LifecycleEvent::OnCreate, "init1"));
        hooks.add(LifecycleHook::new(LifecycleEvent::OnCreate, "init2"));
        hooks.add(
            LifecycleHook::new(LifecycleEvent::OnTransition, "onPendingToConfirmed")
                .with_from_state("Pending")
                .with_to_state("Confirmed"),
        );

        // Test get_hooks
        let create_hooks = hooks.get_hooks(LifecycleEvent::OnCreate);
        assert_eq!(create_hooks.len(), 2);

        // Test get_transition_hooks
        let transition_hooks = hooks.get_transition_hooks("Pending", "Confirmed");
        assert_eq!(transition_hooks.len(), 1);

        // Test has_hooks
        assert!(hooks.has_hooks(LifecycleEvent::OnCreate));
        assert!(!hooks.has_hooks(LifecycleEvent::OnDelete));
    }

    #[test]
    fn test_sorted_hooks() {
        let mut hooks = LifecycleHooks::new();

        hooks.add(LifecycleHook::new(LifecycleEvent::OnCreate, "third").with_order(3));
        hooks.add(LifecycleHook::new(LifecycleEvent::OnCreate, "first").with_order(1));
        hooks.add(LifecycleHook::new(LifecycleEvent::OnCreate, "second").with_order(2));

        let sorted = hooks.get_sorted_hooks(LifecycleEvent::OnCreate);
        assert_eq!(sorted[0].function_name, "first");
        assert_eq!(sorted[1].function_name, "second");
        assert_eq!(sorted[2].function_name, "third");
    }

    #[test]
    fn test_parse_lifecycle_hook() {
        let mut params = HashMap::new();
        params.insert("from".to_string(), "Pending".to_string());
        params.insert("to".to_string(), "Confirmed".to_string());
        params.insert("order".to_string(), "5".to_string());
        params.insert("async".to_string(), "true".to_string());

        let hook = parse_lifecycle_hook(LifecycleEvent::OnTransition, "sendEmail", &params);

        assert_eq!(hook.event, LifecycleEvent::OnTransition);
        assert_eq!(hook.function_name, "sendEmail");
        assert_eq!(hook.from_state, Some("Pending".to_string()));
        assert_eq!(hook.to_state, Some("Confirmed".to_string()));
        assert_eq!(hook.order, 5);
        assert!(hook.is_async);
    }

    #[test]
    fn test_hook_context() {
        let ctx = HookContext::new("123", "Order").with_current_state("Confirmed");

        assert_eq!(ctx.fk_entity, "123");
        assert_eq!(ctx.t_entity_type, "Order");
        assert_eq!(ctx.current_state, Some("Confirmed".to_string()));
    }
}
