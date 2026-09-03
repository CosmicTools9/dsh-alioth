//! Smart Trigger Registry with Inheritance Support
//!
//! This module provides a trigger registry that understands PostgreSQL inheritance.
//! Triggers defined on parent tables are automatically applied to all leaf tables
//! (complete child tables) in the inheritance hierarchy.

use crate::{
    Trigger, TriggerContext, TriggerError, TriggerOperation, TriggerResult, TriggerTiming,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Inheritance graph for PostgreSQL tables
#[derive(Debug, Clone, Default)]
pub struct InheritanceGraph {
    /// parent -> [children]
    children: HashMap<String, Vec<String>>,
    /// child -> [parents]
    parents: HashMap<String, Vec<String>>,
    /// Set of all leaf tables (complete child tables)
    leaf_tables: HashSet<String>,
}

impl InheritanceGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an inheritance relationship
    pub fn add_inheritance(&mut self, child: impl Into<String>, parents: Vec<impl Into<String>>) {
        let child = child.into();
        let parents: Vec<String> = parents.into_iter().map(|p| p.into()).collect();

        // Update parents map
        self.parents.insert(child.clone(), parents.clone());

        // Update children map
        for parent in parents {
            self.children.entry(parent).or_default().push(child.clone());
        }

        // Recalculate leaf tables
        self.recalculate_leaf_tables();
    }

    /// Recalculate leaf tables after inheritance changes
    fn recalculate_leaf_tables(&mut self) {
        self.leaf_tables.clear();

        // A table is a leaf if it has no children
        for table in self.parents.keys() {
            if !self.children.contains_key(table) || self.children[table].is_empty() {
                self.leaf_tables.insert(table.clone());
            }
        }
    }

    /// Check if a table is a leaf table (complete child table)
    pub fn is_leaf_table(&self, table: &str) -> bool {
        self.leaf_tables.contains(table)
    }

    /// Get all leaf tables
    pub fn get_leaf_tables(&self) -> &HashSet<String> {
        &self.leaf_tables
    }

    /// Get all ancestors of a table (including the table itself)
    pub fn get_ancestors(&self, table: &str) -> Vec<String> {
        let mut ancestors = vec![table.to_string()];
        let mut to_process = vec![table.to_string()];
        let mut visited = HashSet::new();
        visited.insert(table.to_string());

        while let Some(current) = to_process.pop() {
            if let Some(parents) = self.parents.get(&current) {
                for parent in parents {
                    if !visited.contains(parent) {
                        visited.insert(parent.clone());
                        ancestors.push(parent.clone());
                        to_process.push(parent.clone());
                    }
                }
            }
        }

        ancestors
    }

    /// Get all descendants of a table (children, grandchildren, etc.)
    pub fn get_descendants(&self, table: &str) -> Vec<String> {
        let mut descendants = Vec::new();
        let mut to_process = vec![table.to_string()];
        let mut visited = HashSet::new();
        visited.insert(table.to_string());

        while let Some(current) = to_process.pop() {
            if let Some(children) = self.children.get(&current) {
                for child in children {
                    if !visited.contains(child) {
                        visited.insert(child.clone());
                        descendants.push(child.clone());
                        to_process.push(child.clone());
                    }
                }
            }
        }

        descendants
    }

    /// Get all leaf tables under a parent table
    pub fn get_leaf_descendants(&self, table: &str) -> Vec<String> {
        self.get_descendants(table)
            .into_iter()
            .filter(|t| self.is_leaf_table(t))
            .collect()
    }

    /// Load inheritance graph from DDL files
    pub fn from_ddl_files(_ddl_dir: &str) -> Result<Self, String> {
        let mut graph = Self::new();

        // Parse DDL files and populate graph
        // This is a simplified version - in production, parse actual DDL
        graph.load_default_alioth_hierarchy();

        Ok(graph)
    }

    /// Load default Alioth hierarchy
    pub fn load_default_alioth_hierarchy(&mut self) {
        // Root table
        // zc_ad_object is the ultimate root

        // Level 2: zc_ad_variable
        self.add_inheritance("zc_ad_variable", vec!["zc_ad_object"]);
        self.add_inheritance("zc_ad_scalar", vec!["zc_ad_variable"]);
        self.add_inheritance("zc_ad_vector", vec!["zc_ad_variable"]);
        self.add_inheritance("zc_ad_tensor", vec!["zc_ad_variable"]);

        // Level 3-4
        self.add_inheritance("zc_ad_dimension", vec!["zc_ad_vector"]);
        self.add_inheritance("zc_ad_relation", vec!["zc_ad_vector"]);

        // Level 5
        self.add_inheritance("zc_ad_relation_r_scalar", vec!["zc_ad_relation"]);
        self.add_inheritance("zc_ad_tensor_r_dimension", vec!["zc_ad_relation"]);
        self.add_inheritance("zc_ad_tensor_r_scalar", vec!["zc_ad_relation"]);
        self.add_inheritance("zc_ad_tensor_rr_non_self-ref", vec!["zc_ad_relation"]);

        // Level 6: zc_id_object
        self.add_inheritance("zc_id_object", vec!["zc_ad_variable", "zc_ad_tensor"]);

        // Level 7: zc_id_object subclasses
        self.add_inheritance("zc_id_evaluation", vec!["zc_id_object", "zc_ad_dimension"]);
        self.add_inheritance("zc_id_factor", vec!["zc_id_object", "zc_ad_dimension"]);
        self.add_inheritance("zc_id_function", vec!["zc_id_object", "zc_ad_dimension"]);
        self.add_inheritance("zc_id_scene", vec!["zc_id_object", "zc_ad_dimension"]);
        self.add_inheritance("zc_id_category", vec!["zc_id_object", "zc_ad_scalar"]);
        self.add_inheritance("zc_id_status", vec!["zc_id_object", "zc_ad_scalar"]);
        self.add_inheritance("zc_id_tags", vec!["zc_id_object", "zc_ad_scalar"]);
        self.add_inheritance("zc_id_unit", vec!["zc_id_object", "zc_ad_scalar"]);
        self.add_inheritance("zc_id_consensus", vec!["zc_id_object"]);

        // Level 8: zc_id_lifecycle
        self.add_inheritance("zc_id_lifecycle", vec!["zc_ad_tensor", "zc_id_object"]);

        // Level 9: zc_id_lifecycle subclasses
        self.add_inheritance("zc_id_bill", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_event", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_entity", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_agreement", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_statement", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_version", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_contract", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_contacts", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_detail", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_identity", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_invoice", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_prod-license", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_message", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_place", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_plan", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_protocol", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_storage", vec!["zc_id_lifecycle"]);
        self.add_inheritance("zc_id_threads", vec!["zc_id_lifecycle"]);

        // Level 10: zc_id_version subclasses
        self.add_inheritance("zc_id_bom", vec!["zc_id_version"]);
        self.add_inheritance("zc_id_document", vec!["zc_id_version"]);
        self.add_inheritance("zc_id_operation", vec!["zc_id_version"]);
        self.add_inheritance("zc_id_project", vec!["zc_id_version"]);
        self.add_inheritance("zc_id_process", vec!["zc_id_version"]);
        self.add_inheritance("zc_id_production", vec!["zc_id_version"]);
        self.add_inheritance("zc_id_standard", vec!["zc_id_version"]);
        self.add_inheritance("zc_id_task", vec!["zc_id_version"]);

        // Level 10a: zc_id_entity subclasses (missing from default)
        self.add_inheritance("zc_id_subjects", vec!["zc_id_entity"]);
        self.add_inheritance("zc_id_device", vec!["zc_id_entity"]);

        // Level 10b: (库存=统计关系，无 zc_id_inventory 继承链——用户 2026-08-07 定稿)

        // Level 10c: zc_id_place / zc_id_storage subclasses
        self.add_inheritance("zc_id_stor-place", vec!["zc_id_place", "zc_id_storage"]);
        self.add_inheritance("zc_id_stor-container", vec!["zc_id_storage"]);
        self.add_inheritance("zc_id_stor-plc-bin", vec!["zc_id_stor-place"]);
        self.add_inheritance("zc_id_stor-plc-warehouse", vec!["zc_id_stor-place"]);

        // Level 10d: zc_id_evaluation → zc_id_eval-calculable → zc_id_rate
        self.add_inheritance("zc_id_eval-calculable", vec!["zc_id_evaluation"]);
        self.add_inheritance("zc_id_rate", vec!["zc_id_eval-calculable"]);
        self.add_inheritance("zc_id_rate-exchange", vec!["zc_id_rate"]);

        // Level 10e: zc_id_operation subclasses
        self.add_inheritance("zc_id_oper-transport_tracking", vec!["zc_id_operation"]);

        // Level 10f: zc_id_process subclasses
        self.add_inheritance("zc_id_proc-approve", vec!["zc_id_process"]);
        self.add_inheritance("zc_id_proc-loading", vec!["zc_id_process"]);
        self.add_inheritance("zc_id_proc-make", vec!["zc_id_process"]);
        self.add_inheritance("zc_id_proc-project", vec!["zc_id_process"]);
        self.add_inheritance("zc_id_proc-purchase", vec!["zc_id_process"]);
        self.add_inheritance("zc_id_proc-service", vec!["zc_id_process"]);

        // Level 10g: zc_id_protocol subclasses
        self.add_inheritance("zc_id_prot-llm_config", vec!["zc_id_protocol"]);

        // Level 10h: zc_id_bill subclasses
        self.add_inheritance("zc_id_bill-check", vec!["zc_id_bill"]);
        self.add_inheritance("zc_id_bill-pricing", vec!["zc_id_bill"]);

        // Level 10i: zc_id_plan subclasses
        self.add_inheritance("zc_id_plan-delivery", vec!["zc_id_plan"]);
        self.add_inheritance("zc_id_plan-inbound", vec!["zc_id_plan"]);
        self.add_inheritance("zc_id_plan-making", vec!["zc_id_plan"]);
        self.add_inheritance("zc_id_plan-material", vec!["zc_id_plan"]);
        self.add_inheritance("zc_id_plan-outbound", vec!["zc_id_plan"]);
        self.add_inheritance("zc_id_plan-perform", vec!["zc_id_plan"]);
        self.add_inheritance("zc_id_plan-project", vec!["zc_id_plan"]);
        self.add_inheritance("zc_id_plan-promotion", vec!["zc_id_plan"]);
        self.add_inheritance("zc_id_plan-purchase", vec!["zc_id_plan"]);
        self.add_inheritance("zc_id_plan-recruitment", vec!["zc_id_plan"]);

        // Level 10j: zc_id_subjects subclasses
        self.add_inheritance("zc_id_subj-employee", vec!["zc_id_subjects"]);
        self.add_inheritance("zc_id_subj-group", vec!["zc_id_subjects"]);
        self.add_inheritance("zc_id_subj-position", vec!["zc_id_subjects"]);

        // Level 10k: zc_id_subj-employee subclasses
        self.add_inheritance("zc_id_empl-agent", vec!["zc_id_subj-employee"]);
        self.add_inheritance("zc_id_empl-natural", vec!["zc_id_subj-employee"]);

        // Level 10l: zc_id_stat-trade_order subclasses
        self.add_inheritance("zc_id_orde-ahbl", vec!["zc_id_stat-trade_order"]);
        self.add_inheritance("zc_id_orde-airlift", vec!["zc_id_stat-trade_order"]);
        self.add_inheritance("zc_id_orde-consult", vec!["zc_id_stat-trade_order"]);
        self.add_inheritance("zc_id_orde-hbl", vec!["zc_id_stat-trade_order"]);
        self.add_inheritance("zc_id_orde-land", vec!["zc_id_stat-trade_order"]);
        self.add_inheritance("zc_id_orde-railway", vec!["zc_id_stat-trade_order"]);
        self.add_inheritance("zc_id_orde-retail", vec!["zc_id_stat-trade_order"]);
        self.add_inheritance("zc_id_orde-shipping", vec!["zc_id_stat-trade_order"]);

        // Note: In a real implementation, load all 570+ leaf tables from DDL
    }
}

/// Smart trigger registry that understands PostgreSQL inheritance.
///
/// # Config-Driven Mode (Strategy A)
///
/// Instead of relying on the hard-coded `load_default_alioth_hierarchy()`,
/// the registry can load the inheritance graph dynamically from
/// `meta_collections.config.inherits` via [`config_driven::init_smart_registry_from_db`](crate::config_driven::init_smart_registry_from_db).
///
/// Trigger implementations are still registered statically via
/// `register_all_triggers()`, but the *scope* of each trigger (which leaf
/// tables it applies to) is determined at runtime by walking the
/// `InheritanceGraph` built from the database config.
pub struct SmartTriggerRegistry {
    /// Base trigger registry
    triggers: HashMap<String, Vec<Arc<dyn Trigger>>>,
    /// Inheritance graph
    pub inheritance: InheritanceGraph,
    /// Cache: leaf table -> applicable triggers
    leaf_trigger_cache: HashMap<String, Vec<Arc<dyn Trigger>>>,
}

impl Default for SmartTriggerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SmartTriggerRegistry {
    pub fn new() -> Self {
        Self {
            triggers: HashMap::new(),
            inheritance: InheritanceGraph::new(),
            leaf_trigger_cache: HashMap::new(),
        }
    }

    /// Replace the inheritance graph and invalidate all caches.
    /// Used when loading the graph dynamically from `meta_collections.config`.
    pub fn with_inheritance(mut self, graph: InheritanceGraph) -> Self {
        self.inheritance = graph;
        self.leaf_trigger_cache.clear();
        self
    }

    /// Replace the inheritance graph in-place.
    pub fn set_inheritance(&mut self, graph: InheritanceGraph) {
        self.inheritance = graph;
        self.leaf_trigger_cache.clear();
    }

    /// Register a trigger on a parent table
    pub fn register_on_parent(
        &mut self,
        parent_table: impl Into<String>,
        trigger: Arc<dyn Trigger>,
    ) {
        let parent = parent_table.into();
        self.triggers.entry(parent).or_default().push(trigger);

        // Invalidate cache since triggers changed
        self.leaf_trigger_cache.clear();
    }

    /// 判断注册在 `ancestor` 上的触发器是否应作用于叶子表 `table_name`。
    ///
    /// `applies_to` 语义（B1 治本 + 继承还原）：
    /// - `applies_to` 为空 => 通配，覆盖注册父表及其全部后代（保留 vector/variable 历史行为）；
    /// - `table_name`（或其任一祖先）命中 `applies_to` => 允许触发。
    ///
    /// 注册在父表上的触发器必须作用于全部后代表：若只做精确表名匹配，
    /// 白名单父表（如 `zc_id_scale`、`zc_id_evaluation`）的**子表**
    /// （`zc_id_scal-duration`、`zc_id_oper-approve` 等）将全部丢失 o_number
    /// 生成。祖先匹配修复此缺口。
    ///
    /// 仍排除无关树：如 `applies_to=["zc_id_lifecycle"]` 的 Lifecycle 模板，
    /// 对 `zc_id_lifecycle` 树之外的表（geometry/scal 等）不触发——这些表的
    /// ancestors 不含 `zc_id_lifecycle`。
    fn applies_to_allows(applies_to: &[&str], ancestors: &[String]) -> bool {
        applies_to.is_empty()
            || applies_to
                .iter()
                .any(|t| ancestors.iter().any(|a| a.as_str() == *t))
    }

    /// Get all triggers applicable to a leaf table (including inherited ones)
    pub fn get_triggers_for_leaf(
        &mut self,
        table_name: &str,
        timing: TriggerTiming,
        operation: TriggerOperation,
    ) -> Vec<Arc<dyn Trigger>> {
        // Check cache first
        let cache_key = format!("{}_{:?}_{:?}", table_name, timing, operation);
        if let Some(cached) = self.leaf_trigger_cache.get(&cache_key) {
            return cached.clone();
        }

        // Get all ancestors of this table
        let ancestors = self.inheritance.get_ancestors(table_name);

        // Collect triggers from all ancestors
        let mut result = Vec::new();
        for ancestor in &ancestors {
            if let Some(triggers) = self.triggers.get(ancestor) {
                for trigger in triggers {
                    if trigger.timing() == timing
                        && trigger.operations().contains(&operation)
                        && Self::applies_to_allows(trigger.applies_to(), &ancestors)
                    {
                        result.push(trigger.clone());
                    }
                }
            }
        }

        // Cache result
        self.leaf_trigger_cache.insert(cache_key, result.clone());

        result
    }

    /// Check if a table is a leaf table
    pub fn is_leaf_table(&self, table: &str) -> bool {
        self.inheritance.is_leaf_table(table)
    }

    /// Get all leaf tables
    pub fn get_all_leaf_tables(&self) -> Vec<String> {
        self.inheritance.get_leaf_tables().iter().cloned().collect()
    }

    /// Load inheritance from DDL
    pub fn load_inheritance_from_ddl(&mut self, ddl_dir: &str) -> Result<(), String> {
        self.inheritance = InheritanceGraph::from_ddl_files(ddl_dir)?;
        self.leaf_trigger_cache.clear();
        Ok(())
    }

    /// Execute all BEFORE triggers for a table
    pub async fn execute_before_triggers(
        &mut self,
        table_name: &str,
        operation: TriggerOperation,
        old_record: Option<&HashMap<String, serde_json::Value>>,
        new_record: Option<&HashMap<String, serde_json::Value>>,
        ctx: &TriggerContext,
    ) -> Result<TriggerResult, TriggerError> {
        let triggers = self.get_triggers_for_leaf(table_name, TriggerTiming::Before, operation);
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

    /// Execute all AFTER triggers for a table
    pub async fn execute_after_triggers(
        &mut self,
        table_name: &str,
        operation: TriggerOperation,
        old_record: Option<&HashMap<String, serde_json::Value>>,
        new_record: Option<&HashMap<String, serde_json::Value>>,
        ctx: &TriggerContext,
    ) -> Result<TriggerResult, TriggerError> {
        let triggers = self.get_triggers_for_leaf(table_name, TriggerTiming::After, operation);
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
mod tests {
    use super::*;
    use crate::{
        init::register_all_triggers, AppContainer, TriggerError, TriggerOperation, TriggerTiming,
    };
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashMap;

    struct TestTrigger {
        name: String,
    }

    #[async_trait]
    impl Trigger for TestTrigger {
        fn name(&self) -> &str {
            &self.name
        }

        fn applies_to(&self) -> &[&str] {
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

    #[test]
    fn test_inheritance_graph() {
        let mut graph = InheritanceGraph::new();
        graph.add_inheritance("child", vec!["parent"]);
        graph.add_inheritance("grandchild", vec!["child"]);

        assert!(graph.is_leaf_table("grandchild"));
        assert!(!graph.is_leaf_table("child"));
        assert!(!graph.is_leaf_table("parent"));

        let ancestors = graph.get_ancestors("grandchild");
        assert!(ancestors.contains(&"grandchild".to_string()));
        assert!(ancestors.contains(&"child".to_string()));
        assert!(ancestors.contains(&"parent".to_string()));
    }

    #[test]
    fn test_smart_registry() {
        let mut registry = SmartTriggerRegistry::new();
        registry.inheritance.load_default_alioth_hierarchy();

        // Register trigger on zc_id_object
        registry.register_on_parent(
            "zc_id_object",
            Arc::new(TestTrigger {
                name: "test_trigger".to_string(),
            }),
        );

        // Should be inherited by zc_id_scene
        let triggers = registry.get_triggers_for_leaf(
            "zc_id_scene",
            TriggerTiming::Before,
            TriggerOperation::Insert,
        );

        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].name(), "test_trigger");
    }

    /// 构建默认层级 + 全量注册的 registry，供 B1 行为测试复用。
    fn build_default_registry() -> SmartTriggerRegistry {
        let mut registry = SmartTriggerRegistry::new();
        registry.inheritance.load_default_alioth_hierarchy();
        register_all_triggers(&mut registry, AppContainer::Meta);
        registry
    }

    fn has_trigger(triggers: &[Arc<dyn Trigger>], name: &str) -> bool {
        triggers.iter().any(|t| t.name() == name)
    }

    #[test]
    fn test_b1_lifecycle_injective_hits_lifecycle_leaf() {
        let mut registry = build_default_registry();
        let triggers = registry.get_triggers_for_leaf(
            "zc_id_entity",
            TriggerTiming::Before,
            TriggerOperation::Insert,
        );
        // B1 修复后：LifecycleInjectiveTemplate 注册在 zc_id_lifecycle，应命中 lifecycle 叶子
        assert!(
            has_trigger(&triggers, "gf_gen_tf_bf_ups_on_var_injective"),
            "lifecycle 叶子 zc_id_entity 应触发 LifecycleInjectiveTemplate"
        );
    }

    #[test]
    fn test_b1_lifecycle_injective_skips_non_lifecycle_leaf() {
        let mut registry = build_default_registry();
        let triggers = registry.get_triggers_for_leaf(
            "zc_id_evaluation",
            TriggerTiming::Before,
            TriggerOperation::Insert,
        );
        // zc_id_evaluation 是 object+dimension（非 lifecycle），不应被 lifecycle 化
        assert!(
            !has_trigger(&triggers, "gf_gen_tf_bf_ups_on_var_injective"),
            "非 lifecycle 叶子 zc_id_evaluation 不应触发 LifecycleInjectiveTemplate"
        );
    }

    #[test]
    fn test_b1_vector_variable_wildcard_regression() {
        let mut registry = build_default_registry();
        // c_count/ak_components 列已从 DDL 移除（规约 DTO_DESIGN_SPEC §6.1 同步），
        // Vector/Variable Count & Components 模板随之移除——对应表不再触发。
        let var_triggers = registry.get_triggers_for_leaf(
            "zc_id_category",
            TriggerTiming::Before,
            TriggerOperation::Insert,
        );
        assert!(
            !has_trigger(&var_triggers, "tf_bf_ins_on_zc_ad_variable_count"),
            "VariableCountTemplate 已移除（c_count 列不存在）"
        );
        let vec_triggers = registry.get_triggers_for_leaf(
            "zc_id_evaluation",
            TriggerTiming::Before,
            TriggerOperation::Insert,
        );
        assert!(
            !has_trigger(&vec_triggers, "gf_gen_tf_bf_ups_on_vec_components"),
            "VectorComponentsTemplate 已移除（ak_components 列不存在）"
        );
    }

    #[test]
    fn test_b1_object_onumber_inherits_to_descendants() {
        let mut registry = build_default_registry();
        // 模拟业务叶子表：挂白名单业务根 zc_id_evaluation（如 zc_id_scale → zc_id_scal-duration）
        registry
            .inheritance
            .add_inheritance("zc_id_virtual_x", vec!["zc_id_evaluation"]);

        let triggers = registry.get_triggers_for_leaf(
            "zc_id_virtual_x",
            TriggerTiming::Before,
            TriggerOperation::Insert,
        );
        assert!(
            has_trigger(&triggers, "tf_bf_ins_93_on_zc_id_object"),
            "白名单业务根 zc_id_evaluation 的后代应触发 ObjectONumberTemplate（v9 SQL 触发器继承语义）"
        );
        // 但 object 树表不应被 lifecycle 模板误触发（applies_to 不含 object 树祖先）
        assert!(
            !has_trigger(&triggers, "tf_bf_lifecycle__f__type"),
            "非 lifecycle 树表不应触发 LifecycleBizTemplate"
        );
        // 无关树：仅挂抽象根 zc_ad_object（不在任何白名单）→ 两个模板都不触发
        registry
            .inheritance
            .add_inheritance("zc_id_virtual_other", vec!["zc_ad_object"]);
        let other_triggers = registry.get_triggers_for_leaf(
            "zc_id_virtual_other",
            TriggerTiming::Before,
            TriggerOperation::Insert,
        );
        assert!(
            !has_trigger(&other_triggers, "tf_bf_ins_93_on_zc_id_object"),
            "白名单外抽象根后代不应触发 ObjectONumberTemplate"
        );
        assert!(
            !has_trigger(&other_triggers, "tf_bf_lifecycle__f__type"),
            "无关树不应触发 LifecycleBizTemplate"
        );
        // 白名单内叶子自身仍应命中
        let entity_triggers = registry.get_triggers_for_leaf(
            "zc_id_entity",
            TriggerTiming::Before,
            TriggerOperation::Insert,
        );
        assert!(
            has_trigger(&entity_triggers, "tf_bf_ins_93_on_zc_id_object"),
            "白名单内叶子 zc_id_entity 仍应触发 ObjectONumberTemplate"
        );
        // 抽象根 zc_id_object 的直接后代（白名单含 zc_id_object）也应命中
        registry
            .inheritance
            .add_inheritance("zc_id_virtual_obj", vec!["zc_id_object"]);
        let obj_triggers = registry.get_triggers_for_leaf(
            "zc_id_virtual_obj",
            TriggerTiming::Before,
            TriggerOperation::Insert,
        );
        assert!(
            has_trigger(&obj_triggers, "tf_bf_ins_93_on_zc_id_object"),
            "zc_id_object 直接后代应触发 ObjectONumberTemplate（含 zc_id_object 白名单）"
        );
    }

    #[test]
    fn test_b1_applies_to_allows_unit() {
        // 通配
        assert!(SmartTriggerRegistry::applies_to_allows(
            &[],
            &["zc_ad_vector".to_string(), "zc_id_evaluation".to_string()]
        ));
        // 显式命中（自身在祖先链）
        assert!(SmartTriggerRegistry::applies_to_allows(
            &["zc_id_entity"],
            &["zc_id_entity".to_string()]
        ));
        // 祖先命中（白名单父表在祖先链——v9 继承语义）
        assert!(SmartTriggerRegistry::applies_to_allows(
            &["zc_id_object"],
            &["zc_id_virtual_x".to_string(), "zc_id_object".to_string()]
        ));
        // 未命中：白名单表不在目标祖先链（无关树，如 geometry 不含 zc_id_lifecycle）
        assert!(!SmartTriggerRegistry::applies_to_allows(
            &["zc_id_lifecycle"],
            &["zc_id_geometry".to_string(), "zc_id_evaluation".to_string()]
        ));
    }
}
