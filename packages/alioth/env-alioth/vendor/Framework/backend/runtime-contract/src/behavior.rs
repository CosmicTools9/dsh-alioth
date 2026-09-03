//! Runtime Behavior Types
//!
//! Provides semantic types for entity lifecycle management:
//! - State machines and transitions
//! - Lifecycle hooks (onCreate, onUpdate, onDelete, onTransition)
//! - Business rules with conditions and triggers

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────
// State Machine
// ─────────────────────────────────────────────────────────────

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

// ─────────────────────────────────────────────────────────────
// Transitions
// ─────────────────────────────────────────────────────────────

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
    let from_str = params.get("from")?;
    let from = if from_str.starts_with('[') && from_str.ends_with(']') {
        // Parse array format
        let content = &from_str[1..from_str.len() - 1];
        content
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        vec![from_str.clone()]
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

// ─────────────────────────────────────────────────────────────
// Lifecycle Hooks
// ─────────────────────────────────────────────────────────────

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

// ─────────────────────────────────────────────────────────────
// Business Rules
// ─────────────────────────────────────────────────────────────

/// When a rule should be evaluated
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "camelCase")]
pub enum RuleTrigger {
    /// Evaluate on entity creation
    OnCreate,
    /// Evaluate on entity update
    OnUpdate,
    /// Evaluate on entity deletion
    OnDelete,
    /// Evaluate during state transition
    OnTransition,
    /// Evaluate on any operation (default)
    #[default]
    Always,
    /// Evaluate on specific state(s)
    OnState,
    /// Custom trigger
    Custom(String),
}

impl std::fmt::Display for RuleTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuleTrigger::OnCreate => write!(f, "onCreate"),
            RuleTrigger::OnUpdate => write!(f, "onUpdate"),
            RuleTrigger::OnDelete => write!(f, "onDelete"),
            RuleTrigger::OnTransition => write!(f, "onTransition"),
            RuleTrigger::Always => write!(f, "always"),
            RuleTrigger::OnState => write!(f, "onState"),
            RuleTrigger::Custom(s) => write!(f, "{}", s),
        }
    }
}

impl std::str::FromStr for RuleTrigger {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "onCreate" | "on_create" => Ok(RuleTrigger::OnCreate),
            "onUpdate" | "on_update" => Ok(RuleTrigger::OnUpdate),
            "onDelete" | "on_delete" => Ok(RuleTrigger::OnDelete),
            "onTransition" | "on_transition" => Ok(RuleTrigger::OnTransition),
            "always" => Ok(RuleTrigger::Always),
            "onState" | "on_state" => Ok(RuleTrigger::OnState),
            _ => Ok(RuleTrigger::Custom(s.to_string())),
        }
    }
}

/// A business rule definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusinessRule {
    /// Rule name (unique identifier)
    pub name: String,
    /// Human-readable description
    #[serde(default)]
    pub description: Option<String>,
    /// Condition expression or function name
    pub condition: String,
    /// Action to execute when condition is met
    #[serde(default)]
    pub action: Option<String>,
    /// Error message when condition is not met
    #[serde(default)]
    pub error_message: Option<String>,
    /// Error code for programmatic handling
    #[serde(default)]
    pub error_code: Option<String>,
    /// Rule priority (higher = evaluated first)
    #[serde(default)]
    pub priority: i32,
    /// Whether this rule is active
    #[serde(default = "default_true")]
    pub active: bool,
    /// Whether this rule blocks the operation on failure
    #[serde(default = "default_true")]
    pub blocking: bool,
    /// When the rule should be evaluated
    #[serde(default)]
    pub trigger: RuleTrigger,
    /// Additional metadata
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

fn default_true() -> bool {
    true
}

impl BusinessRule {
    /// Create a new business rule
    pub fn new(name: impl Into<String>, condition: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            condition: condition.into(),
            action: None,
            error_message: None,
            error_code: None,
            priority: 0,
            active: true,
            blocking: true,
            trigger: RuleTrigger::Always,
            metadata: HashMap::new(),
        }
    }

    /// Set description
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set action
    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// Set error message
    pub fn with_error(mut self, message: impl Into<String>) -> Self {
        self.error_message = Some(message.into());
        self
    }

    /// Set error code
    pub fn with_error_code(mut self, code: impl Into<String>) -> Self {
        self.error_code = Some(code.into());
        self
    }

    /// Set priority
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Set trigger
    pub fn with_trigger(mut self, trigger: RuleTrigger) -> Self {
        self.trigger = trigger;
        self
    }

    /// Set active status
    pub fn set_active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Set blocking status
    pub fn set_blocking(mut self, blocking: bool) -> Self {
        self.blocking = blocking;
        self
    }
}

/// Collection of business rules
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BusinessRules {
    /// Rules indexed by name
    #[serde(default)]
    pub rules: HashMap<String, BusinessRule>,
    /// Rules indexed by trigger
    #[serde(skip)]
    pub by_trigger: HashMap<RuleTrigger, Vec<String>>,
}

impl BusinessRules {
    /// Create a new empty rules collection
    pub fn new() -> Self {
        Self {
            rules: HashMap::new(),
            by_trigger: HashMap::new(),
        }
    }

    /// Add a rule
    pub fn add(&mut self, rule: BusinessRule) {
        let name = rule.name.clone();
        let trigger = rule.trigger.clone();

        self.rules.insert(name.clone(), rule);
        self.by_trigger.entry(trigger).or_default().push(name);
    }

    /// Get a rule by name
    pub fn get(&self, name: &str) -> Option<&BusinessRule> {
        self.rules.get(name)
    }

    /// Remove a rule
    pub fn remove(&mut self, name: &str) -> Option<BusinessRule> {
        self.rules.remove(name).inspect(|rule| {
            if let Some(rules) = self.by_trigger.get_mut(&rule.trigger.clone()) {
                rules.retain(|r: &String| r != name);
            }
        })
    }

    /// Get rules for a specific trigger
    pub fn get_for_trigger(&self, trigger: RuleTrigger) -> Vec<&BusinessRule> {
        self.by_trigger
            .get(&trigger)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| self.rules.get(name))
                    .filter(|r| r.active)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all active rules sorted by priority (highest first)
    pub fn get_active_rules(&self) -> Vec<&BusinessRule> {
        let mut rules: Vec<_> = self.rules.values().filter(|r| r.active).collect();
        rules.sort_by_key(|r| -r.priority); // Higher priority first
        rules
    }

    /// Get rules sorted by priority for a specific trigger
    pub fn get_rules_for_trigger(&self, trigger: RuleTrigger) -> Vec<&BusinessRule> {
        let mut rules = self.get_for_trigger(trigger);
        rules.sort_by_key(|r| -r.priority);
        rules
    }

    /// Check if a rule exists
    pub fn has_rule(&self, name: &str) -> bool {
        self.rules.contains_key(name)
    }

    /// Get all rule names
    pub fn rule_names(&self) -> Vec<&String> {
        self.rules.keys().collect()
    }

    /// Rebuild the trigger index (call after deserialization)
    pub fn rebuild_index(&mut self) {
        self.by_trigger.clear();
        for (name, rule) in &self.rules {
            self.by_trigger
                .entry(rule.trigger.clone())
                .or_default()
                .push(name.clone());
        }
    }
}

/// Parse a business rule from annotation parameters
pub fn parse_rule(params: &HashMap<String, String>) -> Option<BusinessRule> {
    let name = params.get("name")?;
    let condition = params.get("condition")?;

    let mut rule = BusinessRule::new(name.clone(), condition.clone());

    if let Some(desc) = params.get("description") {
        rule.description = Some(desc.clone());
    }

    if let Some(action) = params.get("action") {
        rule.action = Some(action.clone());
    }

    if let Some(error) = params.get("error") {
        rule.error_message = Some(error.clone());
    }

    if let Some(error_code) = params.get("errorCode") {
        rule.error_code = Some(error_code.clone());
    }

    if let Some(priority_str) = params.get("priority") {
        if let Ok(priority) = priority_str.parse() {
            rule.priority = priority;
        }
    }

    if let Some(trigger_str) = params.get("trigger") {
        if let Ok(trigger) = trigger_str.parse() {
            rule.trigger = trigger;
        }
    }

    rule.active = !params.contains_key("inactive");
    rule.blocking = !params.contains_key("nonBlocking");

    Some(rule)
}

/// Rule evaluation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleEvaluation {
    /// Rule name
    pub rule_name: String,
    /// Whether the condition was satisfied
    pub passed: bool,
    /// Error message if failed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Error code if failed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Whether this was a blocking failure
    #[serde(default)]
    pub blocking: bool,
    /// Evaluation timestamp
    pub evaluated_at: String,
}

/// Rule evaluation summary
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuleEvaluationSummary {
    /// All evaluation results
    pub results: Vec<RuleEvaluation>,
    /// Whether all blocking rules passed
    #[serde(default)]
    pub all_passed: bool,
    /// Failed evaluations
    #[serde(default)]
    pub failures: Vec<RuleEvaluation>,
}

impl RuleEvaluationSummary {
    /// Create a new evaluation summary
    pub fn new() -> Self {
        Self {
            results: Vec::new(),
            all_passed: true,
            failures: Vec::new(),
        }
    }

    /// Add an evaluation result
    pub fn add_result(&mut self, result: RuleEvaluation) {
        if !result.passed {
            self.all_passed = false;
            if result.blocking {
                self.failures.push(result.clone());
            }
        }
        self.results.push(result);
    }

    /// Check if there are any blocking failures
    pub fn has_blocking_failures(&self) -> bool {
        self.failures.iter().any(|f| f.blocking)
    }

    /// Get all error messages
    pub fn error_messages(&self) -> Vec<String> {
        self.failures
            .iter()
            .filter_map(|f| f.error.clone())
            .collect()
    }
}

// ─────────────────────────────────────────────────────────────
// Entity Behavior
// ─────────────────────────────────────────────────────────────

/// Complete behavior definition for an entity
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EntityBehavior {
    /// State machine configuration
    #[serde(default)]
    pub state_machine: StateMachine,
    /// State transitions
    #[serde(default)]
    pub transitions: TransitionTable,
    /// Lifecycle hooks
    #[serde(default)]
    pub lifecycle_hooks: LifecycleHooks,
    /// Business rules
    #[serde(default)]
    pub business_rules: BusinessRules,
}

impl EntityBehavior {
    /// Create a new empty behavior definition
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if this entity has any behavior defined
    pub fn has_behavior(&self) -> bool {
        self.state_machine.enabled
            || !self.transitions.transitions.is_empty()
            || !self.lifecycle_hooks.hooks.is_empty()
            || !self.business_rules.rules.is_empty()
    }

    /// Validate the complete behavior definition
    pub fn validate(&self) -> Result<(), BehaviorError> {
        if let Err(e) = validate_state_machine(&self.state_machine) {
            return Err(BehaviorError::StateMachineError(e));
        }

        if self.state_machine.enabled {
            let state_names: Vec<String> = self
                .state_machine
                .states
                .iter()
                .map(|s| s.name.clone())
                .collect();

            let all_transitions: Vec<Transition> = self
                .transitions
                .transitions
                .values()
                .flatten()
                .cloned()
                .collect();

            if let Err(e) = validate_transitions(&all_transitions, &state_names) {
                return Err(BehaviorError::TransitionError(e));
            }
        }

        Ok(())
    }
}

/// Behavior validation error
#[derive(Debug, Clone)]
pub enum BehaviorError {
    StateMachineError(StateMachineError),
    TransitionError(TransitionError),
}

impl std::fmt::Display for BehaviorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BehaviorError::StateMachineError(e) => {
                write!(f, "State machine error: {}", e)
            }
            BehaviorError::TransitionError(e) => {
                write!(f, "Transition error: {}", e)
            }
        }
    }
}

impl std::error::Error for BehaviorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BehaviorError::StateMachineError(e) => Some(e),
            BehaviorError::TransitionError(e) => Some(e),
        }
    }
}

/// Behavior metadata for code generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorMetadata {
    /// Whether state machine is enabled
    pub has_state_machine: bool,
    /// Number of states
    pub state_count: usize,
    /// Number of transitions
    pub transition_count: usize,
    /// Number of lifecycle hooks
    pub lifecycle_hook_count: usize,
    /// Number of business rules
    pub business_rule_count: usize,
}

impl From<&EntityBehavior> for BehaviorMetadata {
    fn from(behavior: &EntityBehavior) -> Self {
        Self {
            has_state_machine: behavior.state_machine.enabled,
            state_count: behavior.state_machine.states.len(),
            transition_count: behavior.transitions.transitions.len(),
            lifecycle_hook_count: behavior.lifecycle_hooks.all_hooks().len(),
            business_rule_count: behavior.business_rules.rules.len(),
        }
    }
}
