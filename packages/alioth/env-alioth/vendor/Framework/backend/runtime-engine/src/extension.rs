//! 应用级逻辑扩展运行时注册表
//!
//! 提供 `AppExtensionRegistry`：按应用代码隔离的扩展配置注册表，
//! 支持在实体 CRUD 生命周期中注入约束验证、业务规则、工作流等逻辑。
//!
//! # 使用方式
//!
//! ```rust,ignore
//! use runtime_engine::AppExtensionRegistry;
//! use runtime_contract::AppLogicExtension;
//!
//! let registry = AppExtensionRegistry::new();
//! registry.register(app_logic_extension);
//!
//! // 在 CRUD handler 中调用
//! let result = registry.before_create("oms", "Order", &variables)?;
//! if !result.all_passed {
//!     return Err(ApiError::ValidationFailed(result.blocking_errors));
//! }
//! ```

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use runtime_contract::behavior::LifecycleEvent;
use runtime_contract::extension::*;

use crate::engine::constraint::{ConstraintEngine, ConstraintValidationResult};
use crate::engine::expression::ExpressionEngine;
use crate::engine::rule::{RuleEngine, RuleExecutionResult};

// ─────────────────────────────────────────────────────────────
// Errors
// ─────────────────────────────────────────────────────────────

/// 扩展运行时错误
#[derive(Debug, Clone)]
pub enum ExtensionRuntimeError {
    ProfileNotFound(String),
    EvaluationFailed(String),
    ConstraintViolated(Vec<String>),
    RuleExecutionFailed(Vec<String>),
}

impl std::fmt::Display for ExtensionRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtensionRuntimeError::ProfileNotFound(app) => {
                write!(f, "Extension profile not found for app: {}", app)
            }
            ExtensionRuntimeError::EvaluationFailed(msg) => {
                write!(f, "Extension evaluation failed: {}", msg)
            }
            ExtensionRuntimeError::ConstraintViolated(errors) => {
                write!(f, "Constraint violations: {:?}", errors)
            }
            ExtensionRuntimeError::RuleExecutionFailed(errors) => {
                write!(f, "Rule execution failed: {:?}", errors)
            }
        }
    }
}

impl std::error::Error for ExtensionRuntimeError {}

// ─────────────────────────────────────────────────────────────
// AppExtensionRegistry
// ─────────────────────────────────────────────────────────────

/// 应用级逻辑扩展注册表
///
/// 线程安全的单例注册表，按 `app_code` 隔离存储各应用的扩展配置。
/// 在 Gateway `register_apps()` 时初始化并注册所有应用的扩展。
#[derive(Debug, Clone)]
pub struct AppExtensionRegistry {
    profiles: Arc<RwLock<HashMap<String, AppLogicExtension>>>,
}

impl AppExtensionRegistry {
    /// 创建空的注册表
    pub fn new() -> Self {
        Self {
            profiles: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 注册应用的扩展配置
    pub fn register(&self, profile: AppLogicExtension) {
        let mut profiles = self.profiles.write().unwrap();
        profiles.insert(profile.app_code.clone(), profile);
    }

    /// 取消注册应用扩展
    pub fn unregister(&self, app_code: &str) {
        let mut profiles = self.profiles.write().unwrap();
        profiles.remove(app_code);
    }

    /// 获取应用的扩展配置
    pub fn get_profile(&self, app_code: &str) -> Option<AppLogicExtension> {
        self.profiles.read().unwrap().get(app_code).cloned()
    }

    /// 检查应用是否有扩展配置
    pub fn has_profile(&self, app_code: &str) -> bool {
        self.profiles.read().unwrap().contains_key(app_code)
    }

    /// 获取所有已注册的应用代码
    pub fn registered_apps(&self) -> Vec<String> {
        self.profiles.read().unwrap().keys().cloned().collect()
    }

    /// 清空所有注册表
    pub fn clear(&self) {
        self.profiles.write().unwrap().clear();
    }

    // =============================================================
    // 约束验证
    // =============================================================

    /// 获取指定应用、指定实体的所有约束
    pub fn get_constraints(&self, app_code: &str, entity: &str) -> Vec<ConstraintExtension> {
        self.profiles
            .read()
            .unwrap()
            .get(app_code)
            .map(|p| {
                p.constraints
                    .iter()
                    .filter(|c| c.entity == entity)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 验证指定应用、指定实体的所有约束
    pub fn validate_constraints(
        &self,
        app_code: &str,
        entity: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<ConstraintValidationResult, ExtensionRuntimeError> {
        let constraints = self.get_constraints(app_code, entity);
        if constraints.is_empty() {
            return Ok(ConstraintValidationResult::new());
        }

        let engine_constraints: Vec<crate::engine::constraint::ConstraintConfig> = constraints
            .into_iter()
            .map(|c| crate::engine::constraint::ConstraintConfig {
                entity: c.entity,
                field: c.field,
                expression: c.expression,
                level: match c.level {
                    ConstraintSeverity::Error => "error".to_string(),
                    ConstraintSeverity::Warning => "warning".to_string(),
                },
                message: c.message,
            })
            .collect();

        Ok(ConstraintEngine::validate(&engine_constraints, variables))
    }

    // =============================================================
    // 状态机
    // =============================================================

    /// 获取指定应用、指定实体的状态机定义（第一个匹配）
    pub fn get_state_machine(&self, app_code: &str, entity: &str) -> Option<StateMachineExtension> {
        self.profiles.read().unwrap().get(app_code).and_then(|p| {
            p.state_machines
                .iter()
                .find(|sm| sm.entity == entity)
                .cloned()
        })
    }

    // =============================================================
    // 业务规则
    // =============================================================

    /// 获取指定应用、指定实体、指定触发器的所有规则
    pub fn get_rules(&self, app_code: &str, entity: &str, trigger: &str) -> Vec<RuleExtension> {
        self.profiles
            .read()
            .unwrap()
            .get(app_code)
            .map(|p| {
                p.business_rules
                    .iter()
                    .filter(|r| r.entity == entity && r.trigger == trigger)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 执行指定应用、指定实体、指定触发器的所有规则
    pub fn execute_rules(
        &self,
        app_code: &str,
        entity: &str,
        trigger: &str,
        variables: &mut HashMap<String, Value>,
    ) -> Result<RuleExecutionResult, ExtensionRuntimeError> {
        let rules = self.get_rules(app_code, entity, trigger);
        if rules.is_empty() {
            return Ok(RuleExecutionResult::new());
        }

        let engine_rules: Vec<crate::engine::rule::BusinessRuleConfig> = rules
            .into_iter()
            .map(|r| crate::engine::rule::BusinessRuleConfig {
                entity: r.entity,
                rule_name: r.name,
                trigger: r.trigger,
                condition: r.condition,
                action: r.action,
                priority: r.priority,
                error_message: r.error_message,
                blocking: r.blocking,
            })
            .collect();

        Ok(RuleEngine::execute(&engine_rules, variables))
    }

    // =============================================================
    // 生命周期钩子 — 统一入口
    // =============================================================

    /// 实体创建前执行所有扩展逻辑
    ///
    /// 执行顺序：
    /// 1. 约束验证（失败则返回错误，阻止创建）
    /// 2. 业务规则（onCreate 触发，可能修改字段值）
    ///
    /// 返回的 `mutations` 应被应用到创建请求中。
    pub fn before_create(
        &self,
        app_code: &str,
        entity: &str,
        variables: &mut HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        let mut result = ExtensionResult::new();

        // 1. 约束验证
        let constraint_result = self.validate_constraints(app_code, entity, variables)?;
        for cr in &constraint_result.results {
            result.add(ExtensionEvaluation {
                extension_type: ExtensionType::Constraint,
                name: format!("{}.{}", cr.entity, cr.field.as_deref().unwrap_or("*")),
                passed: cr.passed,
                error: if cr.passed {
                    None
                } else {
                    Some(cr.message.clone())
                },
                mutations: HashMap::new(),
                evaluated_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        if !constraint_result.is_valid {
            // 如果有 error 级别的约束失败，直接返回
            let has_errors = constraint_result
                .results
                .iter()
                .any(|r| !r.passed && r.level == "error");
            if has_errors {
                return Ok(result);
            }
        }

        // 2. 状态机验证：若 state_field 在变量中且已定义状态机，验证初始状态
        if let Some(sm) = self.get_state_machine(app_code, entity) {
            if let Some(state_val) = variables.get(&sm.state_field) {
                if let Some(state_str) = state_val.as_str() {
                    if let crate::engine::state_machine::StateMachineResult::Failed(msg) =
                        crate::engine::state_machine::StateMachineEngine::validate_initial_state(
                            &sm.states, state_str,
                        )
                    {
                        result.add(ExtensionEvaluation {
                            extension_type: ExtensionType::StateMachine,
                            name: format!("{}.initial_state", entity),
                            passed: false,
                            error: Some(msg),
                            mutations: HashMap::new(),
                            evaluated_at: chrono::Utc::now().to_rfc3339(),
                        });
                        return Ok(result);
                    }
                }
            }
        }

        // 3. 业务规则（onCreate）
        let rule_result = self.execute_rules(app_code, entity, "onCreate", variables)?;
        for exec in &rule_result.executions {
            let mut mutations = HashMap::new();
            for (field, value) in &exec.mutations {
                mutations.insert(field.clone(), value.clone());
            }
            result.add(ExtensionEvaluation {
                extension_type: ExtensionType::BusinessRule,
                name: exec.rule_name.clone(),
                passed: exec.error.is_none(),
                error: exec.error.clone(),
                mutations,
                evaluated_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        Ok(result)
    }

    /// 实体创建后执行扩展逻辑
    ///
    /// 主要用于触发工作流、SWRL 推理等后置逻辑。
    pub fn after_create(
        &self,
        app_code: &str,
        entity: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        let mut result = ExtensionResult::new();

        // 执行业务规则（post-create，trigger = "afterCreate" 或 "always"）
        let mut vars = variables.clone();
        let rule_result = self.execute_rules(app_code, entity, "afterCreate", &mut vars)?;
        if rule_result.executions.is_empty() {
            let rule_result = self.execute_rules(app_code, entity, "always", &mut vars)?;
            for exec in &rule_result.executions {
                let mut mutations = HashMap::new();
                for (field, value) in &exec.mutations {
                    mutations.insert(field.clone(), value.clone());
                }
                result.add(ExtensionEvaluation {
                    extension_type: ExtensionType::BusinessRule,
                    name: exec.rule_name.clone(),
                    passed: exec.error.is_none(),
                    error: exec.error.clone(),
                    mutations,
                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        } else {
            for exec in &rule_result.executions {
                let mut mutations = HashMap::new();
                for (field, value) in &exec.mutations {
                    mutations.insert(field.clone(), value.clone());
                }
                result.add(ExtensionEvaluation {
                    extension_type: ExtensionType::BusinessRule,
                    name: exec.rule_name.clone(),
                    passed: exec.error.is_none(),
                    error: exec.error.clone(),
                    mutations,
                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }

        // 执行工作流
        if let Some(profile) = self.get_profile(app_code) {
            for workflow in &profile.workflows {
                if workflow.trigger.entity == entity
                    && workflow.trigger.event == LifecycleEvent::OnCreate
                {
                    // 检查触发条件
                    if let Some(ref condition) = workflow.trigger.condition {
                        match ExpressionEngine::evaluate(condition, variables) {
                            Ok(value) => {
                                if !value.as_bool().unwrap_or(false) {
                                    continue;
                                }
                            }
                            Err(e) => {
                                result.add(ExtensionEvaluation {
                                    extension_type: ExtensionType::Workflow,
                                    name: workflow.name.clone(),
                                    passed: false,
                                    error: Some(format!(
                                        "Workflow trigger evaluation failed: {}",
                                        e
                                    )),
                                    mutations: HashMap::new(),
                                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                                });
                                continue;
                            }
                        }
                    }

                    // 工作流执行成功（当前版本标记为通过，实际执行由外部调度）
                    result.add(ExtensionEvaluation {
                        extension_type: ExtensionType::Workflow,
                        name: workflow.name.clone(),
                        passed: true,
                        error: None,
                        mutations: HashMap::new(),
                        evaluated_at: chrono::Utc::now().to_rfc3339(),
                    });
                }
            }
        }

        Ok(result)
    }

    /// 实体更新前执行扩展逻辑
    ///
    /// `current_variables` 是当前数据库中实体的状态（用于状态机转换验证等）。
    pub fn before_update(
        &self,
        app_code: &str,
        entity: &str,
        variables: &mut HashMap<String, Value>,
        current_variables: &HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        let mut result = ExtensionResult::new();

        // 约束验证
        let constraint_result = self.validate_constraints(app_code, entity, variables)?;
        for cr in &constraint_result.results {
            result.add(ExtensionEvaluation {
                extension_type: ExtensionType::Constraint,
                name: format!("{}.{}", cr.entity, cr.field.as_deref().unwrap_or("*")),
                passed: cr.passed,
                error: if cr.passed {
                    None
                } else {
                    Some(cr.message.clone())
                },
                mutations: HashMap::new(),
                evaluated_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        if !constraint_result.is_valid {
            let has_errors = constraint_result
                .results
                .iter()
                .any(|r| !r.passed && r.level == "error");
            if has_errors {
                return Ok(result);
            }
        }

        // 2. 状态机验证：检查状态转换是否合法
        if let Some(sm) = self.get_state_machine(app_code, entity) {
            let old_state = current_variables
                .get(&sm.state_field)
                .and_then(|v| v.as_str());
            let new_state = variables.get(&sm.state_field).and_then(|v| v.as_str());
            if let (Some(from), Some(to)) = (old_state, new_state) {
                if from != to {
                    // 尝试用 from→to 匹配任意 event；若用户未指定 event 则自动推断
                    let event = variables
                        .get("event")
                        .and_then(|v| v.as_str())
                        .unwrap_or("update");
                    if let crate::engine::state_machine::StateMachineResult::Failed(msg) =
                        crate::engine::state_machine::StateMachineEngine::validate_transition(
                            &sm.transitions,
                            from,
                            to,
                            event,
                            variables,
                        )
                    {
                        result.add(ExtensionEvaluation {
                            extension_type: ExtensionType::StateMachine,
                            name: format!("{}.transition.{}_{}", entity, from, to),
                            passed: false,
                            error: Some(msg),
                            mutations: HashMap::new(),
                            evaluated_at: chrono::Utc::now().to_rfc3339(),
                        });
                        return Ok(result);
                    }
                }
            }
        }

        // 3. 业务规则（onUpdate）
        let rule_result = self.execute_rules(app_code, entity, "onUpdate", variables)?;
        for exec in &rule_result.executions {
            let mut mutations = HashMap::new();
            for (field, value) in &exec.mutations {
                mutations.insert(field.clone(), value.clone());
            }
            result.add(ExtensionEvaluation {
                extension_type: ExtensionType::BusinessRule,
                name: exec.rule_name.clone(),
                passed: exec.error.is_none(),
                error: exec.error.clone(),
                mutations,
                evaluated_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        Ok(result)
    }

    /// 实体更新后执行扩展逻辑
    pub fn after_update(
        &self,
        app_code: &str,
        entity: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        let mut result = ExtensionResult::new();

        let mut vars = variables.clone();
        let rule_result = self.execute_rules(app_code, entity, "afterUpdate", &mut vars)?;
        if rule_result.executions.is_empty() {
            let rule_result = self.execute_rules(app_code, entity, "always", &mut vars)?;
            for exec in &rule_result.executions {
                let mut mutations = HashMap::new();
                for (field, value) in &exec.mutations {
                    mutations.insert(field.clone(), value.clone());
                }
                result.add(ExtensionEvaluation {
                    extension_type: ExtensionType::BusinessRule,
                    name: exec.rule_name.clone(),
                    passed: exec.error.is_none(),
                    error: exec.error.clone(),
                    mutations,
                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        } else {
            for exec in &rule_result.executions {
                let mut mutations = HashMap::new();
                for (field, value) in &exec.mutations {
                    mutations.insert(field.clone(), value.clone());
                }
                result.add(ExtensionEvaluation {
                    extension_type: ExtensionType::BusinessRule,
                    name: exec.rule_name.clone(),
                    passed: exec.error.is_none(),
                    error: exec.error.clone(),
                    mutations,
                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }

        // 工作流
        if let Some(profile) = self.get_profile(app_code) {
            for workflow in &profile.workflows {
                if workflow.trigger.entity == entity
                    && workflow.trigger.event == LifecycleEvent::OnUpdate
                {
                    if let Some(ref condition) = workflow.trigger.condition {
                        match ExpressionEngine::evaluate(condition, variables) {
                            Ok(value) => {
                                if !value.as_bool().unwrap_or(false) {
                                    continue;
                                }
                            }
                            Err(e) => {
                                result.add(ExtensionEvaluation {
                                    extension_type: ExtensionType::Workflow,
                                    name: workflow.name.clone(),
                                    passed: false,
                                    error: Some(format!(
                                        "Workflow trigger evaluation failed: {}",
                                        e
                                    )),
                                    mutations: HashMap::new(),
                                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                                });
                                continue;
                            }
                        }
                    }
                    result.add(ExtensionEvaluation {
                        extension_type: ExtensionType::Workflow,
                        name: workflow.name.clone(),
                        passed: true,
                        error: None,
                        mutations: HashMap::new(),
                        evaluated_at: chrono::Utc::now().to_rfc3339(),
                    });
                }
            }
        }

        Ok(result)
    }

    /// 实体删除前执行扩展逻辑
    pub fn before_delete(
        &self,
        app_code: &str,
        entity: &str,
        variables: &mut HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        let mut result = ExtensionResult::new();

        // 约束验证
        let constraint_result = self.validate_constraints(app_code, entity, variables)?;
        for cr in &constraint_result.results {
            result.add(ExtensionEvaluation {
                extension_type: ExtensionType::Constraint,
                name: format!("{}.{}", cr.entity, cr.field.as_deref().unwrap_or("*")),
                passed: cr.passed,
                error: if cr.passed {
                    None
                } else {
                    Some(cr.message.clone())
                },
                mutations: HashMap::new(),
                evaluated_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        if !constraint_result.is_valid {
            let has_errors = constraint_result
                .results
                .iter()
                .any(|r| !r.passed && r.level == "error");
            if has_errors {
                return Ok(result);
            }
        }

        // 业务规则（onDelete）
        let rule_result = self.execute_rules(app_code, entity, "onDelete", variables)?;
        for exec in &rule_result.executions {
            let mut mutations = HashMap::new();
            for (field, value) in &exec.mutations {
                mutations.insert(field.clone(), value.clone());
            }
            result.add(ExtensionEvaluation {
                extension_type: ExtensionType::BusinessRule,
                name: exec.rule_name.clone(),
                passed: exec.error.is_none(),
                error: exec.error.clone(),
                mutations,
                evaluated_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        Ok(result)
    }

    /// 实体删除后执行扩展逻辑
    pub fn after_delete(
        &self,
        app_code: &str,
        entity: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        let mut result = ExtensionResult::new();

        let mut vars = variables.clone();
        let rule_result = self.execute_rules(app_code, entity, "afterDelete", &mut vars)?;
        if rule_result.executions.is_empty() {
            let rule_result = self.execute_rules(app_code, entity, "always", &mut vars)?;
            for exec in &rule_result.executions {
                let mut mutations = HashMap::new();
                for (field, value) in &exec.mutations {
                    mutations.insert(field.clone(), value.clone());
                }
                result.add(ExtensionEvaluation {
                    extension_type: ExtensionType::BusinessRule,
                    name: exec.rule_name.clone(),
                    passed: exec.error.is_none(),
                    error: exec.error.clone(),
                    mutations,
                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        } else {
            for exec in &rule_result.executions {
                let mut mutations = HashMap::new();
                for (field, value) in &exec.mutations {
                    mutations.insert(field.clone(), value.clone());
                }
                result.add(ExtensionEvaluation {
                    extension_type: ExtensionType::BusinessRule,
                    name: exec.rule_name.clone(),
                    passed: exec.error.is_none(),
                    error: exec.error.clone(),
                    mutations,
                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }

        Ok(result)
    }

    /// 状态转换时执行扩展逻辑
    ///
    /// 在实体状态变更前调用，验证转换是否允许，执行守卫条件和动作。
    pub fn on_transition(
        &self,
        app_code: &str,
        entity: &str,
        from_state: &str,
        to_state: &str,
        variables: &mut HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        let mut result = ExtensionResult::new();

        if let Some(profile) = self.get_profile(app_code) {
            for sm in &profile.state_machines {
                if sm.entity == entity {
                    // 查找匹配的 transition
                    for transition in &sm.transitions {
                        if transition.can_transition_from(from_state) && transition.to == to_state {
                            // 检查 guard 条件
                            if let Some(ref guard) = transition.guard {
                                match ExpressionEngine::evaluate(guard, variables) {
                                    Ok(value) => {
                                        if !value.as_bool().unwrap_or(false) {
                                            result.add(ExtensionEvaluation {
                                                extension_type: ExtensionType::StateMachine,
                                                name: format!(
                                                    "transition.{}.{}.{}.{}",
                                                    entity, from_state, to_state, transition.event
                                                ),
                                                passed: false,
                                                error: Some(format!(
                                                    "Transition guard failed: {}",
                                                    guard
                                                )),
                                                mutations: HashMap::new(),
                                                evaluated_at: chrono::Utc::now().to_rfc3339(),
                                            });
                                            return Ok(result);
                                        }
                                    }
                                    Err(e) => {
                                        result.add(ExtensionEvaluation {
                                            extension_type: ExtensionType::StateMachine,
                                            name: format!(
                                                "transition.{}.{}.{}.{}",
                                                entity, from_state, to_state, transition.event
                                            ),
                                            passed: false,
                                            error: Some(format!("Guard evaluation error: {}", e)),
                                            mutations: HashMap::new(),
                                            evaluated_at: chrono::Utc::now().to_rfc3339(),
                                        });
                                        return Ok(result);
                                    }
                                }
                            }

                            // 执行 transition action
                            if let Some(ref action) = transition.action {
                                match ExpressionEngine::evaluate(action, variables) {
                                    Ok(value) => {
                                        result =
                                            result.with_mutation("transition_action_result", value);
                                    }
                                    Err(e) => {
                                        result.add(ExtensionEvaluation {
                                            extension_type: ExtensionType::StateMachine,
                                            name: format!(
                                                "transition.{}.{}.{}.{}",
                                                entity, from_state, to_state, transition.event
                                            ),
                                            passed: false,
                                            error: Some(format!("Action evaluation error: {}", e)),
                                            mutations: HashMap::new(),
                                            evaluated_at: chrono::Utc::now().to_rfc3339(),
                                        });
                                        return Ok(result);
                                    }
                                }
                            }

                            result.add(ExtensionEvaluation {
                                extension_type: ExtensionType::StateMachine,
                                name: format!(
                                    "transition.{}.{}.{}.{}",
                                    entity, from_state, to_state, transition.event
                                ),
                                passed: true,
                                error: None,
                                mutations: HashMap::new(),
                                evaluated_at: chrono::Utc::now().to_rfc3339(),
                            });
                        }
                    }
                }
            }
        }

        // 执行业务规则（onTransition）
        let rule_result = self.execute_rules(app_code, entity, "onTransition", variables)?;
        for exec in &rule_result.executions {
            let mut mutations = HashMap::new();
            for (field, value) in &exec.mutations {
                mutations.insert(field.clone(), value.clone());
            }
            result.add(ExtensionEvaluation {
                extension_type: ExtensionType::BusinessRule,
                name: exec.rule_name.clone(),
                passed: exec.error.is_none(),
                error: exec.error.clone(),
                mutations,
                evaluated_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        Ok(result)
    }
}

impl Default for AppExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────
// RegistryLoader — 从文件系统加载扩展配置
// ─────────────────────────────────────────────────────────────

use std::path::Path;

/// 从文件系统加载应用扩展配置
pub struct ExtensionLoader;

impl ExtensionLoader {
    /// 加载单个应用的扩展配置目录
    ///
    /// 读取 `Pre-Proc/{namespace}/Apps/{app}/extensions/` 下的所有 YAML 文件，
    /// 合并为单个 `AppLogicExtension`。
    pub fn load_from_dir(
        app_code: &str,
        dir: impl AsRef<Path>,
    ) -> Result<AppLogicExtension, String> {
        let dir = dir.as_ref();
        if !dir.exists() {
            return Ok(AppLogicExtension::new(app_code));
        }

        let mut extension = AppLogicExtension::new(app_code);

        // constraints.yaml
        let constraints_path = dir.join("constraints.yaml");
        if constraints_path.exists() {
            let content = std::fs::read_to_string(&constraints_path)
                .map_err(|e| format!("Failed to read constraints.yaml: {}", e))?;
            let constraints: Vec<ConstraintExtension> = yaml_serde::from_str(&content)
                .map_err(|e| format!("Failed to parse constraints.yaml: {}", e))?;
            extension.constraints = constraints;
        }

        // rules.yaml
        let rules_path = dir.join("rules.yaml");
        if rules_path.exists() {
            let content = std::fs::read_to_string(&rules_path)
                .map_err(|e| format!("Failed to read rules.yaml: {}", e))?;
            let rules: Vec<RuleExtension> = yaml_serde::from_str(&content)
                .map_err(|e| format!("Failed to parse rules.yaml: {}", e))?;
            extension.business_rules = rules;
        }

        // statemachines.yaml
        let sm_path = dir.join("statemachines.yaml");
        if sm_path.exists() {
            let content = std::fs::read_to_string(&sm_path)
                .map_err(|e| format!("Failed to read statemachines.yaml: {}", e))?;
            let state_machines: Vec<StateMachineExtension> = yaml_serde::from_str(&content)
                .map_err(|e| format!("Failed to parse statemachines.yaml: {}", e))?;
            extension.state_machines = state_machines;
        }

        // workflows.yaml
        let wf_path = dir.join("workflows.yaml");
        if wf_path.exists() {
            let content = std::fs::read_to_string(&wf_path)
                .map_err(|e| format!("Failed to read workflows.yaml: {}", e))?;
            let workflows: Vec<WorkflowDefinition> = yaml_serde::from_str(&content)
                .map_err(|e| format!("Failed to parse workflows.yaml: {}", e))?;
            extension.workflows = workflows;
        }

        // profiles.yaml — 领域模型配置单（叶表激活决策）
        let profiles_path = dir.join("profiles.yaml");
        if profiles_path.exists() {
            let content = std::fs::read_to_string(&profiles_path)
                .map_err(|e| format!("Failed to read profiles.yaml: {}", e))?;
            #[derive(serde::Deserialize)]
            struct ProfilesWrapper {
                #[serde(default)]
                profiles: std::collections::HashMap<String, runtime_contract::AppModelConfig>,
            }
            let wrapper: ProfilesWrapper = yaml_serde::from_str(&content)
                .map_err(|e| format!("Failed to parse profiles.yaml: {}", e))?;
            extension.model_profiles = wrapper.profiles;
        }

        Ok(extension)
    }

    /// 校验扩展配置的实体名和字段名是否合法。
    ///
    /// `known_entities`: entity_name → set of known field names
    ///
    /// 返回校验错误列表。若列表为空则全部通过。
    /// 校验失败不阻止扩展加载，但应记录 ERROR 级别日志。
    pub fn validate_entities(
        extension: &AppLogicExtension,
        known_entities: &HashMap<String, HashSet<String>>,
    ) -> Vec<String> {
        let mut errors = Vec::new();
        let app_code = &extension.app_code;

        for c in &extension.constraints {
            let ent = &c.entity;
            if !known_entities.contains_key(ent) {
                errors.push(format!(
                    "[{}] constraint entity '{}' not found in known entities: {:?}",
                    app_code,
                    ent,
                    known_entities.keys().collect::<Vec<_>>()
                ));
                continue;
            }
            if let Some(field) = &c.field {
                let known_fields = known_entities.get(ent).unwrap();
                if !known_fields.contains(field) {
                    errors.push(format!(
                        "[{}] constraint on '{}.{}': field '{}' not found in entity fields: {:?}",
                        app_code, ent, field, field, known_fields
                    ));
                }
            }
        }

        for r in &extension.business_rules {
            let ent = &r.entity;
            if !known_entities.contains_key(ent) {
                errors.push(format!(
                    "[{}] rule '{}' entity '{}' not found in known entities",
                    app_code, r.name, ent
                ));
            }
        }

        for sm in &extension.state_machines {
            let ent = &sm.entity;
            if !known_entities.contains_key(ent) {
                errors.push(format!(
                    "[{}] state machine entity '{}' not found in known entities",
                    app_code, ent
                ));
            }
        }

        errors
    }
    /// 加载所有应用的扩展配置
    ///
    /// 扫描 `Pre-Proc/{namespace}/Apps/` 和 `Samples/` 目录下的所有应用。
    pub fn load_all(apps_dir: impl AsRef<Path>) -> Result<Vec<AppLogicExtension>, String> {
        let mut results = Vec::new();
        let apps_dir = apps_dir.as_ref();

        if let Ok(entries) = std::fs::read_dir(apps_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_dir() {
                    let app_code = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if app_code.is_empty() {
                        continue;
                    }
                    let ext_dir = path.join("extensions");
                    let extension = Self::load_from_dir(app_code, ext_dir)?;
                    if !extension.is_empty() {
                        results.push(extension);
                    }
                }
            }
        }

        Ok(results)
    }
}
