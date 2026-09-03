//! 表达式求值器
//!
//! 将解析后的 AST (ConstraintExpr) 在给定变量上下文中求值，返回 JSON Value。
//! 支持算术运算、比较运算、逻辑运算和内置函数。

use runtime_contract::expression::{BinaryOp, ConstraintExpr, ConstraintLiteral, UnaryOp};
use serde_json::Value;
use std::collections::HashMap;

/// 通用表达式求值器
pub struct ExpressionEvaluator;

impl ExpressionEvaluator {
    /// 解析并求值表达式字符串
    pub fn evaluate_expression(
        formula: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        let expr = super::parser::parse_constraint_expression(formula)
            .map_err(|e| format!("Parse error: {}", e))?;
        Self::eval_expr_to_json(&expr, variables)
    }

    /// 将 AST 求值为 JSON Value
    pub fn eval_expr_to_json(
        expr: &ConstraintExpr,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        Self::eval_expr_to_json_inner(expr, variables, false)
    }

    /// 严格求值（flow 条件 fail-closed）：FieldRef 缺失 → Err（未定义标识符）
    pub fn eval_expr_to_json_strict(
        expr: &ConstraintExpr,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        Self::eval_expr_to_json_inner(expr, variables, true)
    }

    fn eval_expr_to_json_inner(
        expr: &ConstraintExpr,
        variables: &HashMap<String, Value>,
        strict: bool,
    ) -> Result<Value, String> {
        match expr {
            ConstraintExpr::FieldRef(name) if name.contains('.') => {
                // `_refs.xxx.yyy` 成员访问（模型设计规则：外键列不直接选入
                // 条件/计算，引用值经 `_refs` 模式访问——ctx 携带 _refs 嵌套对象）。
                // 任一成员缺失 → 严格模式 Err（fail-closed），非严格模式 Null。
                let mut cur: Option<&Value> = None;
                for (i, seg) in name.split('.').enumerate() {
                    let next = if i == 0 {
                        variables.get(seg)
                    } else {
                        match cur {
                            Some(Value::Object(m)) => m.get(seg),
                            _ => None,
                        }
                    };
                    match next {
                        Some(v) => cur = Some(v),
                        None if strict => return Err(format!("undefined identifier: {}", name)),
                        None => return Ok(Value::Null),
                    }
                }
                Ok(cur.cloned().unwrap_or(Value::Null))
            }
            ConstraintExpr::FieldRef(name) => match variables.get(name) {
                Some(v) => Ok(v.clone()),
                None if strict => Err(format!("undefined identifier: {}", name)),
                None => Ok(Value::Null),
            },
            ConstraintExpr::Literal(lit) => Ok(Self::literal_to_json(lit)),
            ConstraintExpr::Binary(left, op, right) => {
                let lv = Self::eval_expr_to_json_inner(left, variables, strict)?;
                let rv = Self::eval_expr_to_json_inner(right, variables, strict)?;
                Self::eval_binary_op(&lv, *op, &rv)
            }
            ConstraintExpr::Unary(op, expr) => {
                let v = Self::eval_expr_to_json_inner(expr, variables, strict)?;
                Self::eval_unary_op(*op, &v)
            }
            ConstraintExpr::And(left, right) => {
                let lv = Self::eval_expr_to_json_inner(left, variables, strict)?;
                let rv = Self::eval_expr_to_json_inner(right, variables, strict)?;
                Ok(Value::Bool(Self::is_truthy(&lv) && Self::is_truthy(&rv)))
            }
            ConstraintExpr::Or(left, right) => {
                let lv = Self::eval_expr_to_json_inner(left, variables, strict)?;
                let rv = Self::eval_expr_to_json_inner(right, variables, strict)?;
                Ok(Value::Bool(Self::is_truthy(&lv) || Self::is_truthy(&rv)))
            }
            ConstraintExpr::Not(expr) => {
                let v = Self::eval_expr_to_json_inner(expr, variables, strict)?;
                Ok(Value::Bool(!Self::is_truthy(&v)))
            }
            ConstraintExpr::Call(name, args) => {
                let eval_args: Result<Vec<_>, _> = args
                    .iter()
                    .map(|a| Self::eval_expr_to_json_inner(a, variables, strict))
                    .collect();
                Self::eval_builtin(name, eval_args?)
            }
        }
    }

    /// `in`：左值与列表逐项同型比较，任一相等即真；
    /// 项类型与左值不匹配 → Err（fail-closed，flow 条件语义）
    fn eval_in(left: &Value, right: &Value) -> Result<Value, String> {
        let Value::Array(items) = right else {
            return Err(format!(
                "'in' right operand must be a list, got {:?}",
                right
            ));
        };
        for item in items {
            match Self::same_type_equal(left, item) {
                Some(true) => return Ok(Value::Bool(true)),
                Some(false) => {}
                None => {
                    return Err(format!(
                        "in: type mismatch between {:?} and {:?}",
                        left, item
                    ))
                }
            }
        }
        Ok(Value::Bool(false))
    }

    /// `contains`：左右均须为字符串，子串包含
    fn eval_contains(left: &Value, right: &Value) -> Result<Value, String> {
        match (left, right) {
            (Value::String(s), Value::String(sub)) => Ok(Value::Bool(s.contains(sub.as_str()))),
            _ => Err(format!(
                "contains: both operands must be strings, got {:?} and {:?}",
                left, right
            )),
        }
    }

    /// 同型相等比较（None = 类型不匹配）
    fn same_type_equal(a: &Value, b: &Value) -> Option<bool> {
        match (a, b) {
            (Value::Number(n1), Value::Number(n2)) => Some(n1 == n2),
            (Value::String(s1), Value::String(s2)) => Some(s1 == s2),
            (Value::Bool(b1), Value::Bool(b2)) => Some(b1 == b2),
            _ => None,
        }
    }

    /// 求值二元操作
    fn eval_binary_op(left: &Value, op: BinaryOp, right: &Value) -> Result<Value, String> {
        // flow 条件文法：in（列表包含）与 contains（子串）
        match op {
            BinaryOp::In => return Self::eval_in(left, right),
            BinaryOp::Contains => return Self::eval_contains(left, right),
            _ => {}
        }
        // 算术运算
        if matches!(
            op,
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod
        ) {
            return Self::eval_arithmetic(left, op, right);
        }

        // 比较运算
        let cmp = Self::json_compare(left, right);
        let result = match op {
            BinaryOp::Eq => Self::json_equal(left, right),
            BinaryOp::Ne => !Self::json_equal(left, right),
            BinaryOp::Lt => cmp.is_some_and(|c| c < 0),
            BinaryOp::Le => cmp.is_some_and(|c| c <= 0),
            BinaryOp::Gt => cmp.is_some_and(|c| c > 0),
            BinaryOp::Ge => cmp.is_some_and(|c| c >= 0),
            _ => false,
        };
        Ok(Value::Bool(result))
    }

    /// 算术运算
    fn eval_arithmetic(left: &Value, op: BinaryOp, right: &Value) -> Result<Value, String> {
        match (left, right) {
            (Value::Number(n1), Value::Number(n2)) => {
                let v1 = n1.as_f64().unwrap_or(0.0);
                let v2 = n2.as_f64().unwrap_or(0.0);
                let result = match op {
                    BinaryOp::Add => v1 + v2,
                    BinaryOp::Sub => v1 - v2,
                    BinaryOp::Mul => v1 * v2,
                    BinaryOp::Div => {
                        if v2 == 0.0 {
                            return Err("Division by zero".to_string());
                        }
                        v1 / v2
                    }
                    BinaryOp::Mod => {
                        if v2 == 0.0 {
                            return Err("Modulo by zero".to_string());
                        }
                        v1 % v2
                    }
                    _ => return Err("Non-arithmetic operator in arithmetic context".to_string()),
                };
                Ok(Value::Number(
                    serde_json::Number::from_f64(result).unwrap_or(0.into()),
                ))
            }
            (Value::String(s1), Value::String(s2)) if op == BinaryOp::Add => {
                Ok(Value::String(format!("{}{}", s1, s2)))
            }
            _ => Err(format!(
                "Cannot perform arithmetic {:?} on {:?} and {:?}",
                op, left, right
            )),
        }
    }

    /// 一元运算
    fn eval_unary_op(op: UnaryOp, value: &Value) -> Result<Value, String> {
        match op {
            UnaryOp::Neg => match value {
                Value::Number(n) => {
                    let v = n.as_f64().unwrap_or(0.0);
                    Ok(Value::Number(
                        serde_json::Number::from_f64(-v).unwrap_or(0.into()),
                    ))
                }
                _ => Err("Cannot negate non-numeric value".to_string()),
            },
            UnaryOp::Not => Ok(Value::Bool(!Self::is_truthy(value))),
        }
    }

    /// 内置函数
    fn eval_builtin(name: &str, args: Vec<Value>) -> Result<Value, String> {
        match name {
            "abs" => {
                if let Some(Value::Number(n)) = args.first() {
                    let v = n.as_f64().unwrap_or(0.0);
                    Ok(Value::Number(
                        serde_json::Number::from_f64(v.abs()).unwrap_or(0.into()),
                    ))
                } else {
                    Err("abs() requires a numeric argument".to_string())
                }
            }
            "min" => {
                let min = args
                    .iter()
                    .filter_map(|v| v.as_f64())
                    .fold(f64::INFINITY, f64::min);
                Ok(Value::Number(
                    serde_json::Number::from_f64(min).unwrap_or(0.into()),
                ))
            }
            "max" => {
                let max = args
                    .iter()
                    .filter_map(|v| v.as_f64())
                    .fold(f64::NEG_INFINITY, f64::max);
                Ok(Value::Number(
                    serde_json::Number::from_f64(max).unwrap_or(0.into()),
                ))
            }
            "round" => {
                if let Some(Value::Number(n)) = args.first() {
                    let v = n.as_f64().unwrap_or(0.0);
                    Ok(Value::Number(
                        serde_json::Number::from_f64(v.round()).unwrap_or(0.into()),
                    ))
                } else {
                    Err("round() requires a numeric argument".to_string())
                }
            }
            _ => Err(format!("Unknown function: {}", name)),
        }
    }

    /// 字面量转 JSON
    fn literal_to_json(lit: &ConstraintLiteral) -> Value {
        match lit {
            ConstraintLiteral::String(s) => Value::String(s.clone()),
            ConstraintLiteral::Integer(i) => Value::Number((*i).into()),
            ConstraintLiteral::Decimal(d) => {
                Value::Number(serde_json::Number::from_f64(*d).unwrap_or(0.into()))
            }
            ConstraintLiteral::Boolean(b) => Value::Bool(*b),
            ConstraintLiteral::Null => Value::Null,
            ConstraintLiteral::List(items) => {
                Value::Array(items.iter().map(Self::literal_to_json).collect())
            }
        }
    }

    /// 判断 JSON 值的真值性
    pub fn is_truthy(val: &Value) -> bool {
        match val {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
        }
    }

    /// JSON 值相等比较
    fn json_equal(a: &Value, b: &Value) -> bool {
        a == b
    }

    /// JSON 值大小比较
    fn json_compare(a: &Value, b: &Value) -> Option<i32> {
        match (a, b) {
            (Value::Number(n1), Value::Number(n2)) => {
                let v1 = n1.as_f64()?;
                let v2 = n2.as_f64()?;
                Some(if v1 < v2 {
                    -1
                } else if v1 > v2 {
                    1
                } else {
                    0
                })
            }
            (Value::String(s1), Value::String(s2)) => Some(s1.cmp(s2) as i32),
            _ => None,
        }
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
    fn test_arithmetic() {
        let ctx = vars(&[("a", json!(10)), ("b", json!(3))]);
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("a + b", &ctx).unwrap(),
            json!(13.0)
        );
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("a - b", &ctx).unwrap(),
            json!(7.0)
        );
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("a * b", &ctx).unwrap(),
            json!(30.0)
        );
        assert!(
            ExpressionEvaluator::evaluate_expression("a / b", &ctx)
                .unwrap()
                .as_f64()
                .unwrap()
                > 3.3
        );
    }

    #[test]
    fn test_comparison() {
        let ctx = vars(&[("x", json!(5))]);
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("x > 3", &ctx).unwrap(),
            json!(true)
        );
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("x == 5", &ctx).unwrap(),
            json!(true)
        );
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("x < 3", &ctx).unwrap(),
            json!(false)
        );
    }

    #[test]
    fn test_logic() {
        let ctx = vars(&[("a", json!(true)), ("b", json!(false))]);
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("a AND b", &ctx).unwrap(),
            json!(false)
        );
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("a OR b", &ctx).unwrap(),
            json!(true)
        );
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("NOT b", &ctx).unwrap(),
            json!(true)
        );
    }

    #[test]
    fn test_builtin() {
        let ctx = vars(&[("x", json!(-5.5))]);
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("abs(x)", &ctx).unwrap(),
            json!(5.5)
        );
    }

    #[test]
    fn test_pricing_formula() {
        let ctx = vars(&[
            ("quantity", json!(10)),
            ("unit_price", json!(25.5)),
            ("discount_rate", json!(0.15)),
        ]);
        let result = ExpressionEvaluator::evaluate_expression(
            "quantity * unit_price * (1 - discount_rate)",
            &ctx,
        )
        .unwrap();
        let expected = 10.0 * 25.5 * (1.0 - 0.15);
        assert!((result.as_f64().unwrap() - expected).abs() < 0.001);
    }

    #[test]
    fn test_division_by_zero() {
        let ctx = vars(&[("a", json!(10)), ("b", json!(0))]);
        assert!(ExpressionEvaluator::evaluate_expression("a / b", &ctx).is_err());
    }

    // ── flow 条件求值（统一引擎：expr.rs 能力迁移）────────────────

    #[test]
    fn test_flow_in() {
        let ctx = vars(&[("code", json!("VIP"))]);
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("code in ['VIP', 'SVIP']", &ctx).unwrap(),
            json!(true)
        );
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("code in ['NORMAL']", &ctx).unwrap(),
            json!(false)
        );
        // 类型不匹配 → fail-closed Err
        let ctx_num = vars(&[("code", json!(1))]);
        assert!(ExpressionEvaluator::evaluate_expression("code in ['VIP']", &ctx_num).is_err());
    }

    #[test]
    fn test_flow_contains() {
        let ctx = vars(&[("name", json!("张三丰"))]);
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("name contains '张三'", &ctx).unwrap(),
            json!(true)
        );
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("name contains '李'", &ctx).unwrap(),
            json!(false)
        );
        // 非字符串 → Err
        let ctx_num = vars(&[("name", json!(123))]);
        assert!(ExpressionEvaluator::evaluate_expression("name contains '1'", &ctx_num).is_err());
    }

    #[test]
    fn test_flow_logical_symbols_eval() {
        let ctx = vars(&[("a", json!(1)), ("b", json!(0))]);
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("a > 0 && b == 0", &ctx).unwrap(),
            json!(true)
        );
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("a > 1 || b == 0", &ctx).unwrap(),
            json!(true)
        );
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("!(a > 1)", &ctx).unwrap(),
            json!(true)
        );
    }

    #[test]
    fn test_flow_dash_identifier_eval() {
        let ctx = vars(&[("act-group", json!(2))]);
        assert_eq!(
            ExpressionEvaluator::evaluate_expression("act-group == 2", &ctx).unwrap(),
            json!(true)
        );
    }

    // ── 模型设计规则（2026-09-01）：外键列经 `_refs` 模式访问 ──
    #[test]
    fn test_refs_member_access() {
        let ctx = vars(&[(
            "_refs",
            json!({
                "ck_category": { "id": 7, "label": "合同类", "color": "#4ade80" },
                "qk_qty": { "id": 3, "label": "吨", "color": null },
            }),
        )]);
        let ast = super::super::parser::parse_constraint_expression(
            "_refs.ck_category.label == '合同类'",
        )
        .unwrap();
        assert_eq!(
            ExpressionEvaluator::eval_expr_to_json_strict(&ast, &ctx).unwrap(),
            json!(true)
        );
        // 成员缺失 → 严格 Err（fail-closed）
        let missing =
            super::super::parser::parse_constraint_expression("_refs.ck_category.nonexistent == 1")
                .unwrap();
        assert!(ExpressionEvaluator::eval_expr_to_json_strict(&missing, &ctx).is_err());
        // 顶层 _refs 缺失 → 严格 Err
        let no_refs =
            super::super::parser::parse_constraint_expression("_refs.ck_category.label == 'x'")
                .unwrap();
        assert!(ExpressionEvaluator::eval_expr_to_json_strict(&no_refs, &vars(&[])).is_err());
        // 非对象中间成员 → 严格 Err
        let not_obj =
            super::super::parser::parse_constraint_expression("_refs.qk_qty.id.foo == 1").unwrap();
        assert!(ExpressionEvaluator::eval_expr_to_json_strict(&not_obj, &ctx).is_err());
    }

    #[test]
    fn test_refs_member_access_non_strict() {
        let ctx = vars(&[("_refs", json!({ "ck_category": { "label": "合同类" } }))]);
        let ast =
            super::super::parser::parse_constraint_expression("_refs.missing.notice == 1").unwrap();
        // 非严格：缺失成员 → Null（比较 → false）
        assert_eq!(
            ExpressionEvaluator::eval_expr_to_json(&ast, &ctx).unwrap(),
            json!(false)
        );
    }
}
