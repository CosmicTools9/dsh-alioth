//! SWRL (Semantic Web Rule Language) 类型定义
//!
//! 原位于 alioth-gen::dsl::rules::engine，现下沉至 Framework 作为共享运行时基础设施。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// SWRL 规则定义
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SwrlRule {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub body: Vec<RuleAtom>,
    pub head: Vec<RuleAtom>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

fn default_true() -> bool {
    true
}

impl SwrlRule {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            body: Vec::new(),
            head: Vec::new(),
            priority: 0,
            active: true,
            metadata: HashMap::new(),
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn add_condition(mut self, atom: RuleAtom) -> Self {
        self.body.push(atom);
        self
    }

    pub fn add_conclusion(mut self, atom: RuleAtom) -> Self {
        self.head.push(atom);
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn set_active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn is_class_inference(&self) -> bool {
        self.head
            .iter()
            .any(|atom| matches!(atom, RuleAtom::ClassAssertion(_, _)))
    }

    pub fn is_property_inference(&self) -> bool {
        self.head
            .iter()
            .any(|atom| matches!(atom, RuleAtom::PropertyAssertion(_, _, _)))
    }

    pub fn variables(&self) -> Vec<String> {
        let mut vars = Vec::new();
        for atom in &self.body {
            vars.extend(atom.variables());
        }
        for atom in &self.head {
            vars.extend(atom.variables());
        }
        vars.sort();
        vars.dedup();
        vars
    }
}

/// 规则原子公式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RuleAtom {
    ClassAssertion(String, Term),
    PropertyAssertion(String, Term, Term),
    Comparison(Term, ComparisonOp, Term),
    Builtin(String, Vec<Term>),
}

impl RuleAtom {
    pub fn variables(&self) -> Vec<String> {
        match self {
            RuleAtom::ClassAssertion(_, term) => term.variables(),
            RuleAtom::PropertyAssertion(_, t1, t2) => {
                let mut vars = t1.variables();
                vars.extend(t2.variables());
                vars
            }
            RuleAtom::Comparison(t1, _, t2) => {
                let mut vars = t1.variables();
                vars.extend(t2.variables());
                vars
            }
            RuleAtom::Builtin(_, terms) => {
                let mut vars = Vec::new();
                for term in terms {
                    vars.extend(term.variables());
                }
                vars
            }
        }
    }

    pub fn has_variables(&self) -> bool {
        !self.variables().is_empty()
    }

    pub fn class(class_name: impl Into<String>, term: Term) -> Self {
        RuleAtom::ClassAssertion(class_name.into(), term)
    }

    pub fn property(prop_name: impl Into<String>, subject: Term, object: Term) -> Self {
        RuleAtom::PropertyAssertion(prop_name.into(), subject, object)
    }

    pub fn comparison(left: Term, op: ComparisonOp, right: Term) -> Self {
        RuleAtom::Comparison(left, op, right)
    }
}

/// 项（变量、个体、字面量）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Term {
    Variable(String),
    Individual(String),
    Literal(LiteralValue),
}

impl Term {
    pub fn variables(&self) -> Vec<String> {
        match self {
            Term::Variable(name) => vec![name.clone()],
            _ => Vec::new(),
        }
    }

    pub fn is_variable(&self) -> bool {
        matches!(self, Term::Variable(_))
    }

    pub fn variable(name: impl Into<String>) -> Self {
        Term::Variable(name.into())
    }

    pub fn individual(name: impl Into<String>) -> Self {
        Term::Individual(name.into())
    }

    pub fn string(value: impl Into<String>) -> Self {
        Term::Literal(LiteralValue::String(value.into()))
    }

    pub fn integer(value: i64) -> Self {
        Term::Literal(LiteralValue::Integer(value))
    }

    pub fn decimal(value: f64) -> Self {
        Term::Literal(LiteralValue::Decimal(value))
    }

    pub fn boolean(value: bool) -> Self {
        Term::Literal(LiteralValue::Boolean(value))
    }
}

/// 字面量值
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LiteralValue {
    String(String),
    Integer(i64),
    Decimal(f64),
    Boolean(bool),
    DateTime(String),
}

/// 比较运算符
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ComparisonOp {
    Eq, // =
    Ne, // !=
    Lt, // <
    Le, // <=
    Gt, // >
    Ge, // >=
}

impl std::fmt::Display for ComparisonOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComparisonOp::Eq => write!(f, "="),
            ComparisonOp::Ne => write!(f, "!="),
            ComparisonOp::Lt => write!(f, "<"),
            ComparisonOp::Le => write!(f, "<="),
            ComparisonOp::Gt => write!(f, ">"),
            ComparisonOp::Ge => write!(f, ">="),
        }
    }
}

impl std::str::FromStr for ComparisonOp {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "=" | "==" => Ok(ComparisonOp::Eq),
            "!=" | "<>" => Ok(ComparisonOp::Ne),
            "<" => Ok(ComparisonOp::Lt),
            "<=" => Ok(ComparisonOp::Le),
            ">" => Ok(ComparisonOp::Gt),
            ">=" => Ok(ComparisonOp::Ge),
            _ => Err(format!("Unknown comparison operator: {}", s)),
        }
    }
}

/// SWRL 规则集合
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SwrlRuleSet {
    #[serde(default)]
    pub rules: HashMap<String, SwrlRule>,
    #[serde(skip)]
    pub by_conclusion: HashMap<String, Vec<String>>,
}

impl SwrlRuleSet {
    pub fn new() -> Self {
        Self {
            rules: HashMap::new(),
            by_conclusion: HashMap::new(),
        }
    }

    pub fn add(&mut self, rule: SwrlRule) {
        let name = rule.name.clone();
        for atom in &rule.head {
            if let RuleAtom::ClassAssertion(class_name, _) = atom {
                self.by_conclusion
                    .entry(class_name.clone())
                    .or_default()
                    .push(name.clone());
            }
        }
        self.rules.insert(name, rule);
    }

    pub fn get(&self, name: &str) -> Option<&SwrlRule> {
        self.rules.get(name)
    }

    pub fn remove(&mut self, name: &str) -> Option<SwrlRule> {
        self.rules.remove(name)
    }

    pub fn get_active_rules(&self) -> Vec<&SwrlRule> {
        let mut rules: Vec<_> = self.rules.values().filter(|r| r.active).collect();
        rules.sort_by_key(|r| -r.priority);
        rules
    }

    pub fn get_rules_for_class(&self, class_name: &str) -> Vec<&SwrlRule> {
        self.by_conclusion
            .get(class_name)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| self.rules.get(name))
                    .filter(|r| r.active)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn rule_names(&self) -> Vec<&String> {
        self.rules.keys().collect()
    }

    pub fn has_rule(&self, name: &str) -> bool {
        self.rules.contains_key(name)
    }

    pub fn rebuild_index(&mut self) {
        self.by_conclusion.clear();
        for (name, rule) in &self.rules {
            for atom in &rule.head {
                if let RuleAtom::ClassAssertion(class_name, _) = atom {
                    self.by_conclusion
                        .entry(class_name.clone())
                        .or_default()
                        .push(name.clone());
                }
            }
        }
    }
}

/// 规则解析错误
#[derive(Debug, Clone)]
pub enum RuleParseError {
    InvalidSyntax(String),
    UnknownAtom(String),
    MissingArrow,
    EmptyBody,
    EmptyHead,
}

impl std::fmt::Display for RuleParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuleParseError::InvalidSyntax(msg) => write!(f, "Invalid syntax: {}", msg),
            RuleParseError::UnknownAtom(atom) => write!(f, "Unknown atom: {}", atom),
            RuleParseError::MissingArrow => write!(f, "Missing IF or THEN"),
            RuleParseError::EmptyBody => write!(f, "Rule body (IF) cannot be empty"),
            RuleParseError::EmptyHead => write!(f, "Rule head (THEN) cannot be empty"),
        }
    }
}

impl std::error::Error for RuleParseError {}

// ─────────────────────────────────────────────────────────────
// Parser
// ─────────────────────────────────────────────────────────────

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
        return Ok(Term::Literal(LiteralValue::String(value.to_string())));
    }

    if let Ok(num) = input.parse::<i64>() {
        return Ok(Term::Literal(LiteralValue::Integer(num)));
    }
    if let Ok(num) = input.parse::<f64>() {
        return Ok(Term::Literal(LiteralValue::Decimal(num)));
    }

    if input == "true" {
        return Ok(Term::Literal(LiteralValue::Boolean(true)));
    }
    if input == "false" {
        return Ok(Term::Literal(LiteralValue::Boolean(false)));
    }

    Ok(Term::Individual(input.to_string()))
}
