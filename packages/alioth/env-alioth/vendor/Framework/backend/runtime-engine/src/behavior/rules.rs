//! Business Rules Module
//!
//! Defines business rules with conditions and actions.
//!
//! ## Example DSL
//!
//! ```dsl
//! @rule(name: "inventoryCheck", condition: "quantity > 0", action: "reserveStock")
//! @rule(name: "priceValidation", condition: "price >= 0", error: "Price cannot be negative")
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_business_rule_new() {
        let rule = BusinessRule::new("checkStock", "quantity > 0");
        assert_eq!(rule.name, "checkStock");
        assert_eq!(rule.condition, "quantity > 0");
        assert!(rule.active);
        assert!(rule.blocking);
    }

    #[test]
    fn test_business_rule_builder() {
        let rule = BusinessRule::new("priceCheck", "price >= 0")
            .with_description("Ensure price is non-negative")
            .with_error("Price cannot be negative")
            .with_error_code("INVALID_PRICE")
            .with_priority(10)
            .with_trigger(RuleTrigger::OnCreate)
            .set_blocking(true);

        assert_eq!(
            rule.description,
            Some("Ensure price is non-negative".to_string())
        );
        assert_eq!(
            rule.error_message,
            Some("Price cannot be negative".to_string())
        );
        assert_eq!(rule.error_code, Some("INVALID_PRICE".to_string()));
        assert_eq!(rule.priority, 10);
        assert_eq!(rule.trigger, RuleTrigger::OnCreate);
    }

    #[test]
    fn test_rule_trigger_from_str() {
        assert_eq!(
            "onCreate".parse::<RuleTrigger>().unwrap(),
            RuleTrigger::OnCreate
        );
        assert_eq!(
            "on_update".parse::<RuleTrigger>().unwrap(),
            RuleTrigger::OnUpdate
        );
        assert_eq!(
            "always".parse::<RuleTrigger>().unwrap(),
            RuleTrigger::Always
        );
    }

    #[test]
    fn test_business_rules_collection() {
        let mut rules = BusinessRules::new();

        rules.add(BusinessRule::new("rule1", "cond1").with_priority(1));
        rules.add(BusinessRule::new("rule2", "cond2").with_priority(2));
        rules.add(
            BusinessRule::new("rule3", "cond3")
                .with_trigger(RuleTrigger::OnCreate)
                .with_priority(3),
        );

        // Test get
        assert!(rules.get("rule1").is_some());
        assert!(rules.get("nonexistent").is_none());

        // Test get_for_trigger
        let create_rules = rules.get_for_trigger(RuleTrigger::OnCreate);
        assert_eq!(create_rules.len(), 1);
        assert_eq!(create_rules[0].name, "rule3");

        // Test get_active_rules
        let active = rules.get_active_rules();
        assert_eq!(active.len(), 3);

        // Verify priority order (highest first)
        let sorted = rules.get_rules_for_trigger(RuleTrigger::Always);
        assert_eq!(sorted[0].priority, 2);
        assert_eq!(sorted[1].priority, 1);
    }

    #[test]
    fn test_parse_rule() {
        let mut params = HashMap::new();
        params.insert("name".to_string(), "inventoryCheck".to_string());
        params.insert("condition".to_string(), "quantity > 0".to_string());
        params.insert("action".to_string(), "reserveStock".to_string());
        params.insert("error".to_string(), "Out of stock".to_string());
        params.insert("priority".to_string(), "5".to_string());
        params.insert("trigger".to_string(), "onCreate".to_string());

        let rule = parse_rule(&params).unwrap();
        assert_eq!(rule.name, "inventoryCheck");
        assert_eq!(rule.condition, "quantity > 0");
        assert_eq!(rule.action, Some("reserveStock".to_string()));
        assert_eq!(rule.error_message, Some("Out of stock".to_string()));
        assert_eq!(rule.priority, 5);
        assert_eq!(rule.trigger, RuleTrigger::OnCreate);
    }

    #[test]
    fn test_parse_rule_missing_required() {
        let params = HashMap::new();
        assert!(parse_rule(&params).is_none());

        let mut params = HashMap::new();
        params.insert("name".to_string(), "test".to_string());
        assert!(parse_rule(&params).is_none()); // Missing condition
    }

    #[test]
    fn test_rule_evaluation_summary() {
        let mut summary = RuleEvaluationSummary::new();

        summary.add_result(RuleEvaluation {
            rule_name: "rule1".to_string(),
            passed: true,
            error: None,
            error_code: None,
            blocking: true,
            evaluated_at: chrono::Utc::now().to_rfc3339(),
        });

        summary.add_result(RuleEvaluation {
            rule_name: "rule2".to_string(),
            passed: false,
            error: Some("Validation failed".to_string()),
            error_code: Some("E001".to_string()),
            blocking: true,
            evaluated_at: chrono::Utc::now().to_rfc3339(),
        });

        assert!(!summary.all_passed);
        assert!(summary.has_blocking_failures());
        assert_eq!(summary.error_messages(), vec!["Validation failed"]);
    }
}
