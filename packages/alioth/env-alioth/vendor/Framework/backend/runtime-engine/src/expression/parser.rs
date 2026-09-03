//! 表达式解析器（统一实现位于 runtime-contract；本模块 re-export 保持路径兼容）
//!
//! 文法（runtime-contract expression.rs）：算术（+ - * / %）、比较（== != < <= > >=）、
//! 逻辑（AND/OR/NOT 与 `&&`/`||`/`!` 等价）、函数调用、flow 条件扩展
//! （`in [list]`、`contains`、连字符标识符如 `act-group`）。
//!
//! 单一事实源：`runtime_contract::expression::parse_constraint_expression`——
//! 双实现历史（本文件曾复制 runtime-contract parser）已消除（统一引擎审计）。

pub use runtime_contract::expression::{
    extract_field_references, parse_constraint_expression, ConstraintParseError,
};

#[cfg(test)]
mod tests {
    use runtime_contract::expression::{BinaryOp, ConstraintExpr, ConstraintLiteral};

    use super::*;

    #[test]
    fn test_parse_simple_comparison() {
        let expr = parse_constraint_expression("price > 0").unwrap();
        match expr {
            ConstraintExpr::Binary(left, BinaryOp::Gt, right) => match (&*left, &*right) {
                (
                    ConstraintExpr::FieldRef(name),
                    ConstraintExpr::Literal(ConstraintLiteral::Integer(0)),
                ) => {
                    assert_eq!(name, "price");
                }
                _ => panic!("Unexpected structure"),
            },
            _ => panic!("Expected Binary"),
        }
    }

    #[test]
    fn test_parse_and_or() {
        let expr = parse_constraint_expression("price > 0 AND price < 100").unwrap();
        assert!(matches!(expr, ConstraintExpr::And(_, _)));

        let expr = parse_constraint_expression("status = \"A\" OR status = \"B\"").unwrap();
        assert!(matches!(expr, ConstraintExpr::Or(_, _)));
    }

    #[test]
    fn test_parse_not() {
        let expr = parse_constraint_expression("NOT deleted").unwrap();
        assert!(matches!(expr, ConstraintExpr::Not(_)));
    }

    #[test]
    fn test_parse_arithmetic() {
        assert!(matches!(
            parse_constraint_expression("a + b").unwrap(),
            ConstraintExpr::Binary(_, BinaryOp::Add, _)
        ));
        assert!(matches!(
            parse_constraint_expression("a - b").unwrap(),
            ConstraintExpr::Binary(_, BinaryOp::Sub, _)
        ));
        assert!(matches!(
            parse_constraint_expression("a * b").unwrap(),
            ConstraintExpr::Binary(_, BinaryOp::Mul, _)
        ));
        assert!(matches!(
            parse_constraint_expression("a / b").unwrap(),
            ConstraintExpr::Binary(_, BinaryOp::Div, _)
        ));
        assert!(matches!(
            parse_constraint_expression("a % b").unwrap(),
            ConstraintExpr::Binary(_, BinaryOp::Mod, _)
        ));
    }

    #[test]
    fn test_parse_complex_formula() {
        let expr =
            parse_constraint_expression("quantity * unit_price * (1 - discount_rate)").unwrap();
        assert!(matches!(expr, ConstraintExpr::Binary(_, BinaryOp::Mul, _)));
    }

    #[test]
    fn test_extract_fields() {
        let fields = extract_field_references("price > 0 AND quantity < stock");
        assert!(fields.contains(&"price".to_string()));
        assert!(fields.contains(&"quantity".to_string()));
        assert!(fields.contains(&"stock".to_string()));
    }

    #[test]
    fn test_parse_string_literal() {
        let expr = parse_constraint_expression(r#"status = "active""#).unwrap();
        match expr {
            ConstraintExpr::Binary(_, BinaryOp::Eq, right) => match &*right {
                ConstraintExpr::Literal(ConstraintLiteral::String(s)) => assert_eq!(s, "active"),
                _ => panic!("Expected String literal"),
            },
            _ => panic!("Expected Binary"),
        }
    }

    #[test]
    fn test_parse_decimal() {
        let expr = parse_constraint_expression("price < 99.99").unwrap();
        match expr {
            ConstraintExpr::Binary(_, BinaryOp::Lt, right) => match &*right {
                ConstraintExpr::Literal(ConstraintLiteral::Decimal(v)) => {
                    assert!((*v - 99.99).abs() < 0.001);
                }
                _ => panic!("Expected Decimal literal"),
            },
            _ => panic!("Expected Binary"),
        }
    }

    // ── flow 条件文法扩展（统一引擎：expr.rs 能力迁移）────────────────

    #[test]
    fn test_parse_flow_logical_symbols() {
        assert!(matches!(
            parse_constraint_expression("a > 1 && b < 2").unwrap(),
            ConstraintExpr::And(_, _)
        ));
        assert!(matches!(
            parse_constraint_expression("a > 1 || b < 2").unwrap(),
            ConstraintExpr::Or(_, _)
        ));
        assert!(matches!(
            parse_constraint_expression("!deleted").unwrap(),
            ConstraintExpr::Not(_)
        ));
        assert!(matches!(
            parse_constraint_expression("!(a && b)").unwrap(),
            ConstraintExpr::Not(_)
        ));
    }

    #[test]
    fn test_parse_flow_in_contains() {
        let expr = parse_constraint_expression("code in ['VIP', 'SVIP']").unwrap();
        match expr {
            ConstraintExpr::Binary(left, BinaryOp::In, right) => {
                assert!(matches!(&*left, ConstraintExpr::FieldRef(f) if f == "code"));
                match &*right {
                    ConstraintExpr::Literal(ConstraintLiteral::List(items)) => {
                        assert_eq!(items.len(), 2);
                    }
                    _ => panic!("Expected List literal"),
                }
            }
            _ => panic!("Expected In"),
        }

        let expr = parse_constraint_expression("name contains '张'").unwrap();
        assert!(matches!(
            expr,
            ConstraintExpr::Binary(_, BinaryOp::Contains, _)
        ));
    }

    #[test]
    fn test_parse_flow_dash_identifier() {
        // 连字符标识符（WZ 叶表物理列名 act-group）整体解析为字段引用
        let expr = parse_constraint_expression("act-group == 1").unwrap();
        match expr {
            ConstraintExpr::Binary(left, BinaryOp::Eq, _) => match &*left {
                ConstraintExpr::FieldRef(name) => assert_eq!(name, "act-group"),
                _ => panic!("Expected FieldRef with dash"),
            },
            _ => panic!("Expected Binary"),
        }
        // 带空格减法保持二元减法
        assert!(matches!(
            parse_constraint_expression("a - b").unwrap(),
            ConstraintExpr::Binary(_, BinaryOp::Sub, _)
        ));
    }

    // ── 模型设计规则（2026-09-01）：外键列经 `_refs` 模式访问 ──
    #[test]
    fn test_parse_refs_member_access() {
        let expr = parse_constraint_expression("_refs.ck_category.notice == '合同类'").unwrap();
        match expr {
            ConstraintExpr::Binary(left, BinaryOp::Eq, _) => match &*left {
                ConstraintExpr::FieldRef(name) => {
                    assert_eq!(name, "_refs.ck_category.notice");
                }
                _ => panic!("Expected FieldRef with member path"),
            },
            _ => panic!("Expected Binary"),
        }
        // 数值字面量不受点号标识符影响（1.5 仍解析为 Decimal）
        let dec = parse_constraint_expression("amount < 1.5").unwrap();
        assert!(matches!(dec, ConstraintExpr::Binary(_, BinaryOp::Lt, _)));
        // 字段提取保留点号路径
        let fields = extract_field_references("_refs.ck_category.notice == 'x'");
        assert!(fields.contains(&"_refs.ck_category.notice".to_string()));
    }
}
