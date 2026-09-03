//! 业务规则引擎
//!
//! 执行业务规则：条件-动作模式。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// 业务规则配置（由 LLM-Agent 生成）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusinessRuleConfig {
    pub entity: String,
    pub rule_name: String,
    pub trigger: String,
    pub condition: String,
    pub action: String,
    pub priority: i32,
    pub error_message: String,
    /// 是否为阻塞规则（条件成立时阻止操作，返回 error_message）
    #[serde(default = "default_true")]
    pub blocking: bool,
}

fn default_true() -> bool {
    true
}

/// 单条规则执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleExecution {
    pub rule_name: String,
    pub entity: String,
    pub triggered: bool,
    pub executed: bool,
    pub mutations: Vec<(String, Value)>,
    pub error: Option<String>,
}

/// 批量规则执行结果
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuleExecutionResult {
    pub executions: Vec<RuleExecution>,
    pub all_passed: bool,
    pub mutations: HashMap<String, Value>,
    pub errors: Vec<String>,
}

impl RuleExecutionResult {
    pub fn new() -> Self {
        Self {
            executions: Vec::new(),
            all_passed: true,
            mutations: HashMap::new(),
            errors: Vec::new(),
        }
    }

    pub fn add(&mut self, exec: RuleExecution) {
        if exec.error.is_some() {
            self.all_passed = false;
            self.errors.push(exec.error.clone().unwrap());
        }
        for (field, value) in &exec.mutations {
            self.mutations.insert(field.clone(), value.clone());
        }
        self.executions.push(exec);
    }
}

/// 业务规则引擎
pub struct RuleEngine;

impl RuleEngine {
    /// 执行一组业务规则
    pub fn execute(
        rules: &[BusinessRuleConfig],
        variables: &mut HashMap<String, Value>,
    ) -> RuleExecutionResult {
        let mut result = RuleExecutionResult::new();

        let mut sorted_rules: Vec<_> = rules.iter().collect();
        sorted_rules.sort_by_key(|r| r.priority);

        for config in sorted_rules {
            let exec = Self::execute_single(config, variables);
            result.add(exec);
        }

        result
    }

    fn execute_single(
        config: &BusinessRuleConfig,
        variables: &mut HashMap<String, Value>,
    ) -> RuleExecution {
        let mut exec = RuleExecution {
            rule_name: config.rule_name.clone(),
            entity: config.entity.clone(),
            triggered: false,
            executed: false,
            mutations: Vec::new(),
            error: None,
        };

        // 1. 评估条件
        let condition_met = match Self::evaluate_condition(&config.condition, variables) {
            Ok(met) => met,
            Err(e) => {
                exec.error = Some(format!("Condition evaluation failed: {}", e));
                return exec;
            }
        };

        exec.triggered = condition_met;
        if !condition_met {
            return exec;
        }

        // 2. 阻塞规则：条件成立即阻止操作，不执行动作
        if config.blocking {
            exec.error = Some(config.error_message.clone());
            return exec;
        }

        // 3. 非阻塞规则：执行动作（字段赋值/副作用）
        match Self::execute_action(&config.action, variables) {
            Ok(mutations) => {
                exec.executed = true;
                exec.mutations = mutations;
            }
            Err(e) => {
                exec.error = Some(format!("Action execution failed: {}", e));
            }
        }

        exec
    }

    fn evaluate_condition(
        condition: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<bool, String> {
        let value =
            crate::expression::ExpressionEvaluator::evaluate_expression(condition, variables)
                .map_err(|e| format!("Evaluation error: {}", e))?;
        Ok(crate::expression::evaluator::ExpressionEvaluator::is_truthy(&value))
    }

    fn execute_action(
        action: &str,
        variables: &mut HashMap<String, Value>,
    ) -> Result<Vec<(String, Value)>, String> {
        let mut mutations = Vec::new();

        if let Some(pos) = action.find('=') {
            let field = action[..pos].trim();
            let expr = action[pos + 1..].trim();

            let new_value =
                crate::expression::ExpressionEvaluator::evaluate_expression(expr, variables)
                    .map_err(|e| format!("Action expression error: {}", e))?;

            variables.insert(field.to_string(), new_value.clone());
            mutations.push((field.to_string(), new_value));
        } else {
            return Err(format!("Unsupported action format: {}", action));
        }

        Ok(mutations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx(vars: &[(&str, Value)]) -> HashMap<String, Value> {
        vars.iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_vip_discount_rule() {
        let rules = vec![BusinessRuleConfig {
            entity: "Order".to_string(),
            rule_name: "vip_discount".to_string(),
            trigger: "onCreate".to_string(),
            condition: "customer_level == 'VIP'".to_string(),
            action: "discount_rate = 0.15".to_string(),
            priority: 100,
            error_message: "VIP客户自动享受15%折扣".to_string(),
            blocking: false,
        }];

        let mut vars = ctx(&[
            ("customer_level", json!("VIP")),
            ("discount_rate", json!(0.0)),
        ]);

        let result = RuleEngine::execute(&rules, &mut vars);
        assert!(result.all_passed);
        assert_eq!(vars.get("discount_rate").unwrap(), &json!(0.15));
    }

    #[test]
    fn test_condition_not_met() {
        let rules = vec![BusinessRuleConfig {
            entity: "Order".to_string(),
            rule_name: "vip_discount".to_string(),
            trigger: "onCreate".to_string(),
            condition: "customer_level == 'VIP'".to_string(),
            action: "discount_rate = 0.15".to_string(),
            priority: 100,
            error_message: "VIP客户自动享受15%折扣".to_string(),
            blocking: false,
        }];

        let mut vars = ctx(&[
            ("customer_level", json!("NORMAL")),
            ("discount_rate", json!(0.0)),
        ]);

        let result = RuleEngine::execute(&rules, &mut vars);
        assert!(result.all_passed);
        assert!(!result.executions[0].triggered);
        assert_eq!(vars.get("discount_rate").unwrap(), &json!(0.0));
    }

    #[test]
    fn test_action_with_expression() {
        let rules = vec![BusinessRuleConfig {
            entity: "Order".to_string(),
            rule_name: "calc_total".to_string(),
            trigger: "onCreate".to_string(),
            condition: "quantity > 0".to_string(),
            action: "total = quantity * price".to_string(),
            priority: 100,
            error_message: "计算总价失败".to_string(),
            blocking: false,
        }];

        let mut vars = ctx(&[
            ("quantity", json!(5)),
            ("price", json!(20.0)),
            ("total", json!(0)),
        ]);

        let result = RuleEngine::execute(&rules, &mut vars);
        assert!(result.all_passed);
        assert_eq!(vars.get("total").unwrap(), &json!(100.0));
    }

    #[test]
    fn test_blocking_rule_blocks_when_condition_met() {
        let rules = vec![BusinessRuleConfig {
            entity: "Order".to_string(),
            rule_name: "block_test".to_string(),
            trigger: "onCreate".to_string(),
            condition: "code == 'test'".to_string(),
            action: "".to_string(),
            priority: 100,
            error_message: "不允许使用 test 编码".to_string(),
            blocking: true,
        }];

        let mut vars = ctx(&[("code", json!("test"))]);
        let result = RuleEngine::execute(&rules, &mut vars);
        assert!(!result.all_passed);
        assert!(result.executions[0].error.is_some());
        assert_eq!(
            result.executions[0].error.as_deref(),
            Some("不允许使用 test 编码")
        );
    }

    #[test]
    fn test_blocking_rule_passes_when_condition_not_met() {
        let rules = vec![BusinessRuleConfig {
            entity: "Order".to_string(),
            rule_name: "block_test".to_string(),
            trigger: "onCreate".to_string(),
            condition: "code == 'test'".to_string(),
            action: "".to_string(),
            priority: 100,
            error_message: "不允许使用 test 编码".to_string(),
            blocking: true,
        }];

        let mut vars = ctx(&[("code", json!("normal"))]);
        let result = RuleEngine::execute(&rules, &mut vars);
        assert!(result.all_passed);
        assert!(!result.executions[0].triggered);
    }

    #[test]
    fn test_non_blocking_rule_does_not_block_when_condition_met() {
        let rules = vec![BusinessRuleConfig {
            entity: "Order".to_string(),
            rule_name: "set_default".to_string(),
            trigger: "onCreate".to_string(),
            condition: "status == null".to_string(),
            action: "status = 'active'".to_string(),
            priority: 100,
            error_message: "设置默认状态".to_string(),
            blocking: false,
        }];

        let mut vars = ctx(&[("status", json!(null))]);
        let result = RuleEngine::execute(&rules, &mut vars);
        assert!(result.all_passed);
        assert!(result.executions[0].triggered);
        assert!(result.executions[0].error.is_none());
        assert_eq!(vars.get("status").unwrap(), &json!("active"));
    }
}
