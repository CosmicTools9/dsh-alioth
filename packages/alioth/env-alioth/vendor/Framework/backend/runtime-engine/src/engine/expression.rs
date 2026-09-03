//! 表达式计算引擎
//!
//! 通用算术/逻辑表达式求值引擎，面向业务场景的高级封装。

use serde_json::Value;
use std::collections::HashMap;

/// 计算引擎错误
#[derive(Debug, thiserror::Error)]
pub enum ExpressionError {
    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Evaluation error: {0}")]
    Evaluation(String),

    #[error("Division by zero")]
    DivisionByZero,

    #[error("Unknown variable: {0}")]
    UnknownVariable(String),
}

/// 通用表达式计算引擎
pub struct ExpressionEngine;

impl ExpressionEngine {
    /// 评估单个表达式
    pub fn evaluate(
        formula: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, ExpressionError> {
        crate::expression::ExpressionEvaluator::evaluate_expression(formula, variables)
            .map_err(ExpressionError::Evaluation)
    }

    /// 批量评估多个表达式
    pub fn evaluate_batch(
        formulas: &[(String, String)],
        variables: &HashMap<String, Value>,
    ) -> HashMap<String, Result<Value, ExpressionError>> {
        formulas
            .iter()
            .map(|(target, formula)| {
                let result = Self::evaluate(formula, variables);
                (target.clone(), result)
            })
            .collect()
    }

    /// 验证公式语法
    pub fn validate_syntax(formula: &str) -> Result<(), ExpressionError> {
        crate::expression::parser::parse_constraint_expression(formula)
            .map(|_| ())
            .map_err(|e| ExpressionError::Parse(e.to_string()))
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
    fn test_basic_arithmetic() {
        let vars = ctx(&[("a", json!(10)), ("b", json!(3))]);

        let r = ExpressionEngine::evaluate("a + b", &vars).unwrap();
        assert_eq!(r, json!(13.0));

        let r = ExpressionEngine::evaluate("a - b", &vars).unwrap();
        assert_eq!(r, json!(7.0));

        let r = ExpressionEngine::evaluate("a * b", &vars).unwrap();
        assert_eq!(r, json!(30.0));
    }

    #[test]
    fn test_pricing_formula() {
        let vars = ctx(&[
            ("quantity", json!(10)),
            ("unit_price", json!(25.5)),
            ("discount_rate", json!(0.15)),
        ]);

        let r = ExpressionEngine::evaluate("quantity * unit_price * (1 - discount_rate)", &vars)
            .unwrap();
        let expected = 10.0 * 25.5 * (1.0 - 0.15);
        assert!((r.as_f64().unwrap() - expected).abs() < 0.001);
    }

    #[test]
    fn test_division_by_zero() {
        let vars = ctx(&[("a", json!(10)), ("b", json!(0))]);
        let r = ExpressionEngine::evaluate("a / b", &vars);
        assert!(r.is_err());
    }

    #[test]
    fn test_builtin_functions() {
        let vars = ctx(&[("x", json!(-5.5))]);
        let r = ExpressionEngine::evaluate("abs(x)", &vars).unwrap();
        assert_eq!(r, json!(5.5));
    }

    /// 验证 constraints.yaml 中所有表达式能正确解析和求值
    #[test]
    fn test_constraint_name_not_null_not_empty() {
        let vars = ctx(&[("name", json!("Acme"))]);
        let r = ExpressionEngine::evaluate("name != null AND name != ''", &vars).unwrap();
        assert_eq!(r, json!(true), "name 非空应通过");

        let vars = ctx(&[("name", json!(null))]);
        let r = ExpressionEngine::evaluate("name != null AND name != ''", &vars).unwrap();
        assert_eq!(r, json!(false), "name 为 null 应不通过");

        let vars = ctx(&[("name", json!(""))]);
        let r = ExpressionEngine::evaluate("name != null AND name != ''", &vars).unwrap();
        assert_eq!(r, json!(false), "name 为空字符串应不通过");
    }

    #[test]
    fn test_constraint_code_not_invalid() {
        let vars = ctx(&[("code", json!("valid_code"))]);
        let r =
            ExpressionEngine::evaluate("code == null OR code == '' OR code != 'INVALID'", &vars)
                .unwrap();
        assert_eq!(r, json!(true), "有效编码应通过");

        let vars = ctx(&[("code", json!("INVALID"))]);
        let r =
            ExpressionEngine::evaluate("code == null OR code == '' OR code != 'INVALID'", &vars)
                .unwrap();
        assert_eq!(r, json!(false), "INVALID 编码应不通过");

        let vars = ctx(&[("code", json!(null))]);
        let r =
            ExpressionEngine::evaluate("code == null OR code == '' OR code != 'INVALID'", &vars)
                .unwrap();
        assert_eq!(r, json!(true), "null 编码应通过（可选字段）");
    }

    #[test]
    fn test_constraint_public_must_have_name() {
        let vars = ctx(&[("public", json!(true)), ("name", json!("Acme"))]);
        let r = ExpressionEngine::evaluate("public != true OR name != null", &vars).unwrap();
        assert_eq!(r, json!(true), "公开+有名称应通过");

        let vars = ctx(&[("public", json!(true)), ("name", json!(null))]);
        let r = ExpressionEngine::evaluate("public != true OR name != null", &vars).unwrap();
        assert_eq!(r, json!(false), "公开+名称为空应不通过");

        let vars = ctx(&[("public", json!(false)), ("name", json!(null))]);
        let r = ExpressionEngine::evaluate("public != true OR name != null", &vars).unwrap();
        assert_eq!(r, json!(true), "非公开+名称为空应通过");
    }

    #[test]
    fn test_constraint_f_enum() {
        let vars = ctx(&[("_f_", json!("personal"))]);
        let r = ExpressionEngine::evaluate(
            "_f_ == 'personal' OR _f_ == 'company' OR _f_ == 'government' OR _f_ == null OR _f_ == ''",
            &vars,
        ).unwrap();
        assert_eq!(r, json!(true), "'personal' 应通过");

        let vars = ctx(&[("_f_", json!("invalid"))]);
        let r = ExpressionEngine::evaluate(
            "_f_ == 'personal' OR _f_ == 'company' OR _f_ == 'government' OR _f_ == null OR _f_ == ''",
            &vars,
        ).unwrap();
        assert_eq!(r, json!(false), "'invalid' 应不通过");

        let vars = ctx(&[("_f_", json!(null))]);
        let r = ExpressionEngine::evaluate(
            "_f_ == 'personal' OR _f_ == 'company' OR _f_ == 'government' OR _f_ == null OR _f_ == ''",
            &vars,
        ).unwrap();
        assert_eq!(r, json!(true), "null 应通过（可选字段）");
    }

    /// 验证 rules.yaml 中的条件表达式能正确求值
    #[test]
    fn test_rule_auto_company_for_public() {
        let vars = ctx(&[("public", json!(true)), ("_f_", json!(null))]);
        let r = ExpressionEngine::evaluate("public == true AND (_f_ == null OR _f_ == '')", &vars)
            .unwrap();
        assert_eq!(r, json!(true), "公开+无形态 应触发规则");

        let vars = ctx(&[("public", json!(true)), ("_f_", json!("company"))]);
        let r = ExpressionEngine::evaluate("public == true AND (_f_ == null OR _f_ == '')", &vars)
            .unwrap();
        assert_eq!(r, json!(false), "公开+已有形态 不触发");
    }

    #[test]
    fn test_rule_block_test_code() {
        let vars = ctx(&[("code", json!("test"))]);
        let r = ExpressionEngine::evaluate("code == 'test'", &vars).unwrap();
        assert_eq!(r, json!(true), "code == 'test' 应触发阻塞");

        let vars = ctx(&[("code", json!("normal"))]);
        let r = ExpressionEngine::evaluate("code == 'test'", &vars).unwrap();
        assert_eq!(r, json!(false), "code == 'normal' 不应触发");
    }

    /// 验证序列化约束/规则 YAML 能正确反序列化
    #[test]
    fn test_constraint_yaml_roundtrip() {
        use runtime_contract::extension::ConstraintExtension;

        let yaml = r#"
- entity: Subject
  field: name
  expression: "name != null AND name != ''"
  level: Error
  message: "客户名称不能为空"
"#;
        let constraints: Vec<ConstraintExtension> = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(constraints.len(), 1);
        assert_eq!(constraints[0].entity, "Subject");
        assert_eq!(constraints[0].field.as_deref(), Some("name"));
    }

    #[test]
    fn test_rule_yaml_roundtrip() {
        use runtime_contract::extension::RuleExtension;

        let yaml = r#"
- entity: Subject
  name: auto_company_for_public
  trigger: onCreate
  condition: "public == true AND (_f_ == null OR _f_ == '')"
  action: "_f_ = 'company'"
  priority: 100
  error_message: "公开客户自动设为公司形态"
  blocking: false
"#;
        let rules: Vec<RuleExtension> = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].entity, "Subject");
        assert_eq!(rules[0].name, "auto_company_for_public");
        assert!(!rules[0].blocking);
    }

    #[test]
    fn test_blocking_rule_yaml_default() {
        use runtime_contract::extension::RuleExtension;

        let yaml = r#"
- entity: Subject
  name: block_test_code
  trigger: onCreate
  condition: "code == 'test'"
  action: ""
  priority: 300
  error_message: "不允许使用 'test' 作为客户编码"
"#;
        let rules: Vec<RuleExtension> = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(rules.len(), 1);
        // blocking 未指定时，serde(default = "default_true") 应返回 true
        assert!(rules[0].blocking, "blocking 默认值应为 true");
    }
}
