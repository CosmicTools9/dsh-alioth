//! SWRL 规则解析器
//!
//! 从字符串解析 SWRL 规则为 AST。

use runtime_contract::swrl::{ComparisonOp, RuleAtom, RuleParseError, SwrlRule, Term};

/// 解析 SWRL 规则字符串
pub fn parse_swrl_rule(input: &str) -> Result<SwrlRule, RuleParseError> {
    let mut rule = SwrlRule::new("temp");

    if let Some(name_start) = input.find('"') {
        if let Some(name_end) = input[name_start + 1..].find('"') {
            let name = &input[name_start + 1..name_start + 1 + name_end];
            rule.name = name.to_string();
        }
    }

    let if_pos = input
        .to_lowercase()
        .find("if")
        .ok_or(RuleParseError::MissingArrow)?;
    let then_pos = input
        .to_lowercase()
        .find("then")
        .ok_or(RuleParseError::MissingArrow)?;

    if if_pos >= then_pos {
        return Err(RuleParseError::InvalidSyntax(
            "IF must come before THEN".to_string(),
        ));
    }

    let body_str = &input[if_pos + 2..then_pos].trim();
    let head_str = &input[then_pos + 4..].trim();

    if body_str.is_empty() {
        return Err(RuleParseError::EmptyBody);
    }
    if head_str.is_empty() {
        return Err(RuleParseError::EmptyHead);
    }

    for atom_str in body_str.split("AND").map(|s| s.trim()) {
        if let Ok(atom) = parse_atom(atom_str) {
            rule.body.push(atom);
        }
    }

    for atom_str in head_str.split("AND").map(|s| s.trim()) {
        if let Ok(atom) = parse_atom(atom_str) {
            rule.head.push(atom);
        }
    }

    if rule.body.is_empty() {
        return Err(RuleParseError::EmptyBody);
    }
    if rule.head.is_empty() {
        return Err(RuleParseError::EmptyHead);
    }

    Ok(rule)
}

/// 解析原子公式
fn parse_atom(input: &str) -> Result<RuleAtom, RuleParseError> {
    let input = input.trim();

    if let Some(paren_pos) = input.find('(') {
        if input.ends_with(')') {
            let class_name = &input[..paren_pos].trim();
            let inner = &input[paren_pos + 1..input.len() - 1].trim();

            if let Some(op_pos) = find_comparison_op(inner) {
                let left = &inner[..op_pos].trim();
                let rest = &inner[op_pos..].trim();
                let (op, right) = parse_op_and_right(rest)?;
                return Ok(RuleAtom::Comparison(
                    parse_term(left)?,
                    op,
                    parse_term(right)?,
                ));
            }

            if inner.contains(',') {
                let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();
                if parts.len() == 2 {
                    return Ok(RuleAtom::PropertyAssertion(
                        class_name.to_string(),
                        parse_term(parts[0])?,
                        parse_term(parts[1])?,
                    ));
                }
            }

            return Ok(RuleAtom::ClassAssertion(
                class_name.to_string(),
                parse_term(inner)?,
            ));
        }
    }

    if let Some(op_pos) = find_comparison_op(input) {
        let left = &input[..op_pos].trim();
        let rest = &input[op_pos..].trim();
        let (op, right) = parse_op_and_right(rest)?;
        return Ok(RuleAtom::Comparison(
            parse_term(left)?,
            op,
            parse_term(right)?,
        ));
    }

    Err(RuleParseError::UnknownAtom(input.to_string()))
}

/// 查找比较运算符位置
fn find_comparison_op(input: &str) -> Option<usize> {
    let ops = [">=", "<=", "!=", "=", "<", ">"];
    for op in &ops {
        if let Some(pos) = input.find(op) {
            return Some(pos);
        }
    }
    None
}

/// 解析运算符和右侧
fn parse_op_and_right(input: &str) -> Result<(ComparisonOp, &str), RuleParseError> {
    let input = input.trim();
    let ops = [
        ("<=", ComparisonOp::Le),
        (">=", ComparisonOp::Ge),
        ("!=", ComparisonOp::Ne),
        ("=", ComparisonOp::Eq),
        ("<", ComparisonOp::Lt),
        (">", ComparisonOp::Gt),
    ];
    for (op_str, op) in &ops {
        if let Some(rest) = input.strip_prefix(op_str) {
            let right = rest.trim();
            return Ok((*op, right));
        }
    }
    Err(RuleParseError::InvalidSyntax(format!(
        "Expected comparison operator, got: {}",
        input
    )))
}

/// 解析项
fn parse_term(input: &str) -> Result<Term, RuleParseError> {
    let input = input.trim();

    if let Some(rest) = input.strip_prefix('?') {
        return Ok(Term::Variable(rest.to_string()));
    }

    if (input.starts_with('"') && input.ends_with('"'))
        || (input.starts_with('\'') && input.ends_with('\''))
    {
        let value = &input[1..input.len() - 1];
        return Ok(Term::Literal(super::LiteralValue::String(
            value.to_string(),
        )));
    }

    if let Ok(num) = input.parse::<i64>() {
        return Ok(Term::Literal(super::LiteralValue::Integer(num)));
    }
    if let Ok(num) = input.parse::<f64>() {
        return Ok(Term::Literal(super::LiteralValue::Decimal(num)));
    }

    if input == "true" {
        return Ok(Term::Literal(super::LiteralValue::Boolean(true)));
    }
    if input == "false" {
        return Ok(Term::Literal(super::LiteralValue::Boolean(false)));
    }

    Ok(Term::Individual(input.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LiteralValue, SwrlRuleSet};

    #[test]
    fn test_parse_simple_rule() {
        let input =
            r#"rule "adultDefinition" IF Person(?x) AND age(?x, ?a) AND ?a >= 18 THEN Adult(?x)"#;
        let rule = parse_swrl_rule(input).unwrap();
        assert_eq!(rule.name, "adultDefinition");
        assert_eq!(rule.body.len(), 3);
        assert_eq!(rule.head.len(), 1);
    }

    #[test]
    fn test_parse_class_assertion() {
        let atom = parse_atom("Person(?x)").unwrap();
        assert!(
            matches!(atom, RuleAtom::ClassAssertion(name, Term::Variable(var))
            if name == "Person" && var == "x")
        );
    }

    #[test]
    fn test_parse_property_assertion() {
        let atom = parse_atom("hasAge(?x, 25)").unwrap();
        assert!(
            matches!(atom, RuleAtom::PropertyAssertion(name, Term::Variable(var), Term::Literal(_))
            if name == "hasAge" && var == "x")
        );
    }

    #[test]
    fn test_parse_comparison() {
        let atom = parse_atom("?x >= 18").unwrap();
        assert!(matches!(
            atom,
            RuleAtom::Comparison(
                Term::Variable(_),
                ComparisonOp::Ge,
                Term::Literal(LiteralValue::Integer(18))
            )
        ));
    }

    #[test]
    fn test_term_variables() {
        let var = Term::variable("x");
        assert_eq!(var.variables(), vec!["x"]);
        assert!(var.is_variable());

        let ind = Term::individual("john");
        assert!(ind.variables().is_empty());
        assert!(!ind.is_variable());
    }

    #[test]
    fn test_rule_variables() {
        let rule = SwrlRule::new("test")
            .add_condition(RuleAtom::class("Person", Term::variable("x")))
            .add_condition(RuleAtom::property(
                "hasAge",
                Term::variable("x"),
                Term::variable("a"),
            ))
            .add_conclusion(RuleAtom::class("Adult", Term::variable("x")));

        let vars = rule.variables();
        assert!(vars.contains(&"x".to_string()));
        assert!(vars.contains(&"a".to_string()));
    }

    #[test]
    fn test_swrl_rule_set() {
        let mut set = SwrlRuleSet::new();
        let rule1 = SwrlRule::new("rule1").with_priority(10);
        let rule2 = SwrlRule::new("rule2").with_priority(5);
        set.add(rule1);
        set.add(rule2);
        assert_eq!(set.rule_names().len(), 2);
        let active = set.get_active_rules();
        assert_eq!(active[0].name, "rule1");
    }

    #[test]
    fn test_comparison_op_display() {
        assert_eq!(ComparisonOp::Eq.to_string(), "=");
        assert_eq!(ComparisonOp::Ge.to_string(), ">=");
        assert_eq!(ComparisonOp::Lt.to_string(), "<");
    }

    #[test]
    fn test_comparison_op_parse() {
        assert_eq!("=".parse::<ComparisonOp>().unwrap(), ComparisonOp::Eq);
        assert_eq!(">=".parse::<ComparisonOp>().unwrap(), ComparisonOp::Ge);
        assert_eq!("!=".parse::<ComparisonOp>().unwrap(), ComparisonOp::Ne);
    }
}
