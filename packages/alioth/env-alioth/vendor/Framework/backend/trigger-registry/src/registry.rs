use crate::{Trigger, TriggerContext, TriggerOperation, TriggerResult, TriggerTiming};
use std::collections::HashMap;
use std::sync::Arc;

/// Registry of all triggers indexed by table name and operation
#[derive(Default, Clone)]
pub struct TriggerRegistry {
    /// Triggers indexed by table name -> timing -> operation -> trigger names
    triggers: HashMap<String, Vec<Arc<dyn Trigger>>>,
}

impl TriggerRegistry {
    pub fn new() -> Self {
        Self {
            triggers: HashMap::new(),
        }
    }

    /// Register a trigger
    pub fn register(&mut self, trigger: Arc<dyn Trigger>) {
        for table in trigger.applies_to() {
            self.triggers
                .entry(table.to_string())
                .or_default()
                .push(trigger.clone());
        }
    }

    /// Get all triggers applicable to a table for a given timing and operation
    pub fn get_triggers(
        &self,
        table_name: &str,
        timing: TriggerTiming,
        operation: TriggerOperation,
    ) -> Vec<Arc<dyn Trigger>> {
        self.triggers
            .get(table_name)
            .map(|triggers| {
                triggers
                    .iter()
                    .filter(|t| t.timing() == timing && t.operations().contains(&operation))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Check if any triggers exist for a table/operation/timing combination
    pub fn has_triggers(
        &self,
        table_name: &str,
        timing: TriggerTiming,
        operation: TriggerOperation,
    ) -> bool {
        !self.get_triggers(table_name, timing, operation).is_empty()
    }

    /// Execute all matching before triggers and return merged results
    pub async fn execute_before_triggers(
        &self,
        table_name: &str,
        operation: TriggerOperation,
        old_record: Option<&HashMap<String, serde_json::Value>>,
        new_record: Option<&HashMap<String, serde_json::Value>>,
        ctx: &TriggerContext,
    ) -> Result<TriggerResult, crate::TriggerError> {
        let triggers = self.get_triggers(table_name, TriggerTiming::Before, operation);
        let mut merged = TriggerResult::new();

        for trigger in triggers {
            let result = trigger.execute(old_record, new_record, ctx).await?;

            if result.blocked {
                return Ok(result);
            }

            // Merge modified fields
            for (key, value) in result.modified_fields {
                merged.modified_fields.insert(key, value);
            }

            // Merge side effects
            for effect in result.side_effects {
                merged.side_effects.push(effect);
            }
        }

        Ok(merged)
    }

    /// Execute all matching after triggers and return merged results
    pub async fn execute_after_triggers(
        &self,
        table_name: &str,
        operation: TriggerOperation,
        old_record: Option<&HashMap<String, serde_json::Value>>,
        new_record: Option<&HashMap<String, serde_json::Value>>,
        ctx: &TriggerContext,
    ) -> Result<TriggerResult, crate::TriggerError> {
        let triggers = self.get_triggers(table_name, TriggerTiming::After, operation);
        let mut merged = TriggerResult::new();

        for trigger in triggers {
            let result = trigger.execute(old_record, new_record, ctx).await?;

            if result.blocked {
                return Ok(result);
            }

            for (key, value) in result.modified_fields {
                merged.modified_fields.insert(key, value);
            }

            for effect in result.side_effects {
                merged.side_effects.push(effect);
            }
        }

        Ok(merged)
    }
}

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;
    use crate::{TriggerError, TriggerTiming};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashMap;

    struct TestTrigger {
        name: String,
        tables: Vec<String>,
    }

    #[async_trait]
    impl Trigger for TestTrigger {
        fn name(&self) -> &str {
            &self.name
        }

        fn applies_to(&self) -> &[&str] {
            // This is a test limitation; real triggers use static slices
            &[]
        }

        fn operations(&self) -> &[TriggerOperation] {
            &[TriggerOperation::Insert]
        }

        fn timing(&self) -> TriggerTiming {
            TriggerTiming::Before
        }

        async fn execute(
            &self,
            _old: Option<&HashMap<String, Value>>,
            _new: Option<&HashMap<String, Value>>,
            _ctx: &TriggerContext,
        ) -> Result<TriggerResult, TriggerError> {
            Ok(TriggerResult::new())
        }
    }

    // Note: applies_to returns empty slice in test due to trait limitation
    // Real implementations use static string slices
}
