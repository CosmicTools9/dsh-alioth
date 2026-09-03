//! SWRL 规则求值器
//!
//! 执行 SWRL 规则的条件-动作模式。

use runtime_contract::swrl::{ComparisonOp, LiteralValue, RuleAtom, SwrlRule, SwrlRuleSet, Term};
use serde_json::Value;
use std::collections::HashMap;

/// SWRL 规则求值引擎
pub struct SwrlRuleEngine;

impl SwrlRuleEngine {
    /// 评估规则集合中所有适用的规则
    pub fn evaluate_rules(
        rule_set: &SwrlRuleSet,
        variables: &HashMap<String, Value>,
    ) -> SwrlEvaluationResult {
        let mut result = SwrlEvaluationResult::new();

        for rule in rule_set.get_active_rules() {
            match Self::evaluate_rule(rule, variables) {
                Ok(passed) => {
                    result.add(SwrlEvaluation {
                        rule_name: rule.name.clone(),
                        passed,
                        error: if passed {
                            None
                        } else {
                            Some(format!("Rule '{}' condition not satisfied", rule.name))
                        },
                        evaluated_at: chrono::Utc::now().to_rfc3339(),
                    });
                }
                Err(e) => {
                    result.add(SwrlEvaluation {
                        rule_name: rule.name.clone(),
                        passed: false,
                        error: Some(format!("Error evaluating rule: {}", e)),
                        evaluated_at: chrono::Utc::now().to_rfc3339(),
                    });
                }
            }
        }

        result
    }

    /// 评估单条规则
    fn evaluate_rule(rule: &SwrlRule, variables: &HashMap<String, Value>) -> Result<bool, String> {
        for atom in &rule.body {
            if !Self::evaluate_atom(atom, variables)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// 评估原子公式
    fn evaluate_atom(atom: &RuleAtom, variables: &HashMap<String, Value>) -> Result<bool, String> {
        match atom {
            RuleAtom::Comparison(left, op, right) => {
                Self::evaluate_comparison(left, *op, right, variables)
            }
            RuleAtom::ClassAssertion(_class_name, _term) => Ok(true), // Placeholder: needs ontology
            RuleAtom::PropertyAssertion(_prop, _subject, _object) => Ok(true), // Placeholder
            RuleAtom::Builtin(_name, _args) => Ok(true),              // Placeholder
        }
    }

    /// 评估比较
    fn evaluate_comparison(
        left: &Term,
        op: ComparisonOp,
        right: &Term,
        variables: &HashMap<String, Value>,
    ) -> Result<bool, String> {
        let left_val = Self::resolve_term(left, variables)?;
        let right_val = Self::resolve_term(right, variables)?;

        match (left_val, right_val) {
            (Value::Number(n1), Value::Number(n2)) => {
                let v1 = n1.as_f64().unwrap_or(0.0);
                let v2 = n2.as_f64().unwrap_or(0.0);
                Ok(match op {
                    ComparisonOp::Eq => (v1 - v2).abs() < f64::EPSILON,
                    ComparisonOp::Ne => (v1 - v2).abs() >= f64::EPSILON,
                    ComparisonOp::Lt => v1 < v2,
                    ComparisonOp::Le => v1 <= v2,
                    ComparisonOp::Gt => v1 > v2,
                    ComparisonOp::Ge => v1 >= v2,
                })
            }
            (Value::String(s1), Value::String(s2)) => Ok(match op {
                ComparisonOp::Eq => s1 == s2,
                ComparisonOp::Ne => s1 != s2,
                ComparisonOp::Lt => s1 < s2,
                ComparisonOp::Le => s1 <= s2,
                ComparisonOp::Gt => s1 > s2,
                ComparisonOp::Ge => s1 >= s2,
            }),
            (Value::Bool(b1), Value::Bool(b2)) => Ok(match op {
                ComparisonOp::Eq => b1 == b2,
                ComparisonOp::Ne => b1 != b2,
                _ => false,
            }),
            _ => Ok(false),
        }
    }

    /// 解析项为值
    pub fn resolve_term(term: &Term, variables: &HashMap<String, Value>) -> Result<Value, String> {
        match term {
            Term::Variable(name) => variables
                .get(name)
                .cloned()
                .ok_or_else(|| format!("Variable '{}' not found", name)),
            Term::Individual(name) => Ok(Value::String(name.clone())),
            Term::Literal(lit) => match lit {
                LiteralValue::String(s) => Ok(Value::String(s.clone())),
                LiteralValue::Integer(i) => Ok(Value::Number((*i).into())),
                LiteralValue::Decimal(d) => Ok(Value::Number(
                    serde_json::Number::from_f64(*d).unwrap_or(0.into()),
                )),
                LiteralValue::Boolean(b) => Ok(Value::Bool(*b)),
                LiteralValue::DateTime(dt) => Ok(Value::String(dt.clone())),
            },
        }
    }
}

/// 单条规则评估结果
#[derive(Debug, Clone)]
pub struct SwrlEvaluation {
    pub rule_name: String,
    pub passed: bool,
    pub error: Option<String>,
    pub evaluated_at: String,
}

/// 批量评估结果
#[derive(Debug, Clone, Default)]
pub struct SwrlEvaluationResult {
    pub evaluations: Vec<SwrlEvaluation>,
    pub all_passed: bool,
}

impl SwrlEvaluationResult {
    pub fn new() -> Self {
        Self {
            evaluations: Vec::new(),
            all_passed: true,
        }
    }

    pub fn add(&mut self, eval: SwrlEvaluation) {
        if !eval.passed {
            self.all_passed = false;
        }
        self.evaluations.push(eval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vars(data: &[(&str, Value)]) -> HashMap<String, Value> {
        data.iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_resolve_term() {
        let ctx = vars(&[("age", json!(25))]);
        let var_term = Term::Variable("age".to_string());
        let result = SwrlRuleEngine::resolve_term(&var_term, &ctx).unwrap();
        assert_eq!(result, json!(25));

        let literal_term = Term::Literal(LiteralValue::Integer(10));
        let result = SwrlRuleEngine::resolve_term(&literal_term, &ctx).unwrap();
        assert_eq!(result, json!(10));
    }

    #[test]
    fn test_evaluate_comparison() {
        let ctx = vars(&[("age", json!(25)), ("min_age", json!(18))]);
        let atom = RuleAtom::Comparison(Term::variable("age"), ComparisonOp::Ge, Term::integer(18));
        assert!(SwrlRuleEngine::evaluate_atom(&atom, &ctx).unwrap());

        let atom = RuleAtom::Comparison(Term::variable("age"), ComparisonOp::Lt, Term::integer(18));
        assert!(!SwrlRuleEngine::evaluate_atom(&atom, &ctx).unwrap());
    }

    #[test]
    fn test_evaluate_rules() {
        let rule = SwrlRule::new("adult")
            .add_condition(RuleAtom::Comparison(
                Term::variable("age"),
                ComparisonOp::Ge,
                Term::integer(18),
            ))
            .add_conclusion(RuleAtom::class("Adult", Term::variable("x")));

        let mut set = SwrlRuleSet::new();
        set.add(rule);

        let ctx = vars(&[("age", json!(25))]);
        let result = SwrlRuleEngine::evaluate_rules(&set, &ctx);
        assert!(result.all_passed);

        let ctx = vars(&[("age", json!(15))]);
        let result = SwrlRuleEngine::evaluate_rules(&set, &ctx);
        assert!(!result.all_passed);
    }
}
