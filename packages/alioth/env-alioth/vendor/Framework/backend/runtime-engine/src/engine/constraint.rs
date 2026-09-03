//! 约束验证引擎
//!
//! 支持字段级约束和跨字段约束验证。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// 约束配置（由 LLM-Agent 生成，存储于 YAML/DB）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintConfig {
    pub entity: String,
    pub field: Option<String>,
    pub expression: String,
    pub level: String,
    pub message: String,
}

/// 单个约束验证结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintResult {
    pub entity: String,
    pub field: Option<String>,
    pub expression: String,
    pub passed: bool,
    pub level: String,
    pub message: String,
}

/// 批量约束验证结果
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConstraintValidationResult {
    pub results: Vec<ConstraintResult>,
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ConstraintValidationResult {
    pub fn new() -> Self {
        Self {
            results: Vec::new(),
            is_valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn add(&mut self, result: ConstraintResult) {
        if !result.passed {
            self.is_valid = false;
            if result.level == "error" {
                self.errors.push(result.message.clone());
            } else {
                self.warnings.push(result.message.clone());
            }
        }
        self.results.push(result);
    }
}

/// 约束验证引擎
pub struct ConstraintEngine;

impl ConstraintEngine {
    /// 验证一组约束
    pub fn validate(
        constraints: &[ConstraintConfig],
        variables: &HashMap<String, Value>,
    ) -> ConstraintValidationResult {
        let mut result = ConstraintValidationResult::new();

        for config in constraints {
            let passed = match Self::evaluate_constraint(config, variables) {
                Ok(passed) => passed,
                Err(e) => {
                    log::warn!(
                        "Constraint evaluation failed for {}: {} (error: {})",
                        config.entity,
                        config.expression,
                        e
                    );
                    config.level == "warning"
                }
            };

            result.add(ConstraintResult {
                entity: config.entity.clone(),
                field: config.field.clone(),
                expression: config.expression.clone(),
                passed,
                level: config.level.clone(),
                message: if passed {
                    String::new()
                } else {
                    config.message.clone()
                },
            });
        }

        result
    }

    /// 验证单个约束
    fn evaluate_constraint(
        config: &ConstraintConfig,
        variables: &HashMap<String, Value>,
    ) -> Result<bool, String> {
        let value = crate::expression::ExpressionEvaluator::evaluate_expression(
            &config.expression,
            variables,
        )
        .map_err(|e| format!("Evaluation error: {}", e))?;
        Ok(crate::expression::evaluator::ExpressionEvaluator::is_truthy(&value))
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
    fn test_field_constraint() {
        let constraints = vec![ConstraintConfig {
            entity: "Order".to_string(),
            field: Some("number".to_string()),
            expression: "number > 0".to_string(),
            level: "error".to_string(),
            message: "订单金额必须大于0".to_string(),
        }];

        let vars = ctx(&[("number", json!(100.0))]);
        let result = ConstraintEngine::validate(&constraints, &vars);
        assert!(result.is_valid);
        assert!(result.errors.is_empty());

        let vars = ctx(&[("number", json!(-10.0))]);
        let result = ConstraintEngine::validate(&constraints, &vars);
        assert!(!result.is_valid);
        assert_eq!(result.errors, vec!["订单金额必须大于0"]);
    }

    #[test]
    fn test_cross_field_constraint() {
        let constraints = vec![ConstraintConfig {
            entity: "Order".to_string(),
            field: None,
            expression: "delivery_date >= order_date".to_string(),
            level: "error".to_string(),
            message: "交货日期不能早于下单日期".to_string(),
        }];

        let vars = ctx(&[
            ("order_date", json!("2024-01-01")),
            ("delivery_date", json!("2024-01-10")),
        ]);
        let result = ConstraintEngine::validate(&constraints, &vars);
        assert!(result.is_valid);
    }

    #[test]
    fn test_warning_level() {
        let constraints = vec![ConstraintConfig {
            entity: "Order".to_string(),
            field: Some("quantity".to_string()),
            expression: "quantity >= 10".to_string(),
            level: "warning".to_string(),
            message: "建议批量订购不少于10件".to_string(),
        }];

        let vars = ctx(&[("quantity", json!(5))]);
        let result = ConstraintEngine::validate(&constraints, &vars);
        assert!(!result.is_valid);
        assert!(result.errors.is_empty());
        assert_eq!(result.warnings.len(), 1);
    }
}
