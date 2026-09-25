//! 表达式计算引擎（唯一引擎：Rhai 沙箱）
//!
//! 业务场景的高级封装：单个/批量求值 + 语法与标识符校验。

use serde_json::Value;
use std::collections::HashMap;

/// 计算引擎错误
///
/// 仅保留可达变体：唯一构造点 = [`ExpressionEngine::evaluate`] 的求值错误
/// （原 `Parse`/`DivisionByZero`/`UnknownVariable` 三变体零构造、零匹配，已删除）。
#[derive(Debug, thiserror::Error)]
pub enum ExpressionError {
    #[error("Evaluation error: {0}")]
    Evaluation(String),
}

/// 通用表达式计算引擎
pub struct ExpressionEngine;

impl ExpressionEngine {
    /// 评估单个表达式（Rhai 沙箱；ctx 值以常量注入 ⇒ 表达式无副作用）
    pub fn evaluate(
        formula: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, ExpressionError> {
        crate::expression::RhaiExpressionEngine::new()
            .evaluate(formula, variables)
            .map_err(ExpressionError::Evaluation)
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

        assert_eq!(
            ExpressionEngine::evaluate("a + b", &vars).unwrap(),
            json!(13)
        );
        assert_eq!(
            ExpressionEngine::evaluate("a - b", &vars).unwrap(),
            json!(7)
        );
        assert_eq!(
            ExpressionEngine::evaluate("a * b", &vars).unwrap(),
            json!(30)
        );
    }

    #[test]
    fn test_pricing_formula() {
        let vars = ctx(&[
            ("quantity", json!(10)),
            ("unit_price", json!(25.5)),
            ("discount_rate", json!(0.15)),
        ]);

        let r = ExpressionEngine::evaluate("quantity * unit_price * (1.0 - discount_rate)", &vars)
            .unwrap();
        let expected = 10.0 * 25.5 * (1.0 - 0.15);
        assert!((r.as_f64().unwrap() - expected).abs() < 0.001);
    }

    #[test]
    fn test_division_by_zero() {
        let vars = ctx(&[("a", json!(10)), ("b", json!(0))]);
        assert!(ExpressionEngine::evaluate("a / b", &vars).is_err());
    }

    #[test]
    fn test_builtin_functions() {
        let vars = ctx(&[("x", json!(-5.5))]);
        assert_eq!(
            ExpressionEngine::evaluate("abs(x)", &vars).unwrap(),
            json!(5.5)
        );
    }

    /// 空值/空串语义（迁移自 DSL 约束断言；`()` = Rhai 单元 = JSON null）
    #[test]
    fn test_null_and_empty_semantics() {
        let expr = "name != () && name != \"\"";
        assert_eq!(
            ExpressionEngine::evaluate(expr, &ctx(&[("name", json!("Acme"))])).unwrap(),
            json!(true)
        );
        assert_eq!(
            ExpressionEngine::evaluate(expr, &ctx(&[("name", json!(null))])).unwrap(),
            json!(false)
        );
        assert_eq!(
            ExpressionEngine::evaluate(expr, &ctx(&[("name", json!(""))])).unwrap(),
            json!(false)
        );
    }

    /// 枚举白名单语义（`in` 列表 + 空值分支）
    #[test]
    fn test_enum_whitelist() {
        let expr = "_f_ == () || _f_ == \"\" || _f_ in [\"personal\", \"company\", \"government\"]";
        assert_eq!(
            ExpressionEngine::evaluate(expr, &ctx(&[("_f_", json!("personal"))])).unwrap(),
            json!(true)
        );
        assert_eq!(
            ExpressionEngine::evaluate(expr, &ctx(&[("_f_", json!("invalid"))])).unwrap(),
            json!(false)
        );
        assert_eq!(
            ExpressionEngine::evaluate(expr, &ctx(&[("_f_", json!(null))])).unwrap(),
            json!(true)
        );
    }

    /// 时间比较（ISO 字符串按字典序比较，Rhai 原生支持）
    #[test]
    fn test_date_string_comparison() {
        let vars = ctx(&[
            ("order_date", json!("2024-01-01")),
            ("delivery_date", json!("2024-01-10")),
        ]);
        assert_eq!(
            ExpressionEngine::evaluate("delivery_date >= order_date", &vars).unwrap(),
            json!(true)
        );
    }

    #[test]
    fn test_constraint_yaml_roundtrip() {
        use runtime_contract::extension::ConstraintExtension;

        let yaml = r#"
- entity: Subject
  field: name
  expression: "name != () && name != \"\""
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
  condition: "public == true && (_f_ == () || _f_ == \"\")"
  action: "_f_ = \"company\""
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
  condition: "code == \"test\""
  action: ""
  priority: 300
  error_message: "不允许使用 test 作为客户编码"
"#;
        let rules: Vec<RuleExtension> = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(rules.len(), 1);
        assert!(rules[0].blocking, "blocking 默认值应为 true");
    }
}
