//! 表达式解析与求值模块
//!
//! 提供约束/算术表达式的 AST 定义、解析器和求值器。
//! 原位于 alioth-gen::dsl::rules::constraint，现下沉至 Framework 作为共享运行时基础设施。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 约束表达式 AST
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConstraintExpr {
    /// 字段引用: fieldName
    FieldRef(String),
    /// 字面量
    Literal(ConstraintLiteral),
    /// 二元操作: left op right
    Binary(Box<ConstraintExpr>, BinaryOp, Box<ConstraintExpr>),
    /// 一元操作: op expr
    Unary(UnaryOp, Box<ConstraintExpr>),
    /// 函数调用: name(args)
    Call(String, Vec<ConstraintExpr>),
    /// 逻辑 AND
    And(Box<ConstraintExpr>, Box<ConstraintExpr>),
    /// 逻辑 OR
    Or(Box<ConstraintExpr>, Box<ConstraintExpr>),
    /// 逻辑 NOT
    Not(Box<ConstraintExpr>),
}

/// 二元运算符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinaryOp {
    Eq,  // =
    Ne,  // !=
    Lt,  // <
    Le,  // <=
    Gt,  // >
    Ge,  // >=
    Add, // +
    Sub, // -
    Mul, // *
    Div, // /
    Mod, // %
    /// 属于列表（flow 条件文法：`x in [a, b, c]`；逐项同型比较，任一相等即真）
    In,
    /// 子串包含（flow 条件文法：`x contains 'sub'`；左右均须为字符串）
    Contains,
}

impl std::fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BinaryOp::Eq => write!(f, "="),
            BinaryOp::Ne => write!(f, "!="),
            BinaryOp::Lt => write!(f, "<"),
            BinaryOp::Le => write!(f, "<="),
            BinaryOp::Gt => write!(f, ">"),
            BinaryOp::Ge => write!(f, ">="),
            BinaryOp::Add => write!(f, "+"),
            BinaryOp::Sub => write!(f, "-"),
            BinaryOp::Mul => write!(f, "*"),
            BinaryOp::Div => write!(f, "/"),
            BinaryOp::Mod => write!(f, "%"),
            BinaryOp::In => write!(f, "in"),
            BinaryOp::Contains => write!(f, "contains"),
        }
    }
}

/// 一元运算符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOp {
    Neg, // -
    Not, // !
}

/// 约束字面量
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConstraintLiteral {
    String(String),
    Integer(i64),
    Decimal(f64),
    Boolean(bool),
    Null,
    /// 列表字面量（`in` 右值：`[a, b, c]`）
    List(Vec<ConstraintLiteral>),
}

/// 解析错误
#[derive(Debug, Clone)]
pub enum ConstraintParseError {
    InvalidSyntax(String),
    UnexpectedToken(String),
    UnknownOperator(String),
    EmptyExpression,
}

impl std::fmt::Display for ConstraintParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConstraintParseError::InvalidSyntax(msg) => write!(f, "Invalid syntax: {}", msg),
            ConstraintParseError::UnexpectedToken(token) => {
                write!(f, "Unexpected token: {}", token)
            }
            ConstraintParseError::UnknownOperator(op) => write!(f, "Unknown operator: {}", op),
            ConstraintParseError::EmptyExpression => write!(f, "Empty expression"),
        }
    }
}

impl std::error::Error for ConstraintParseError {}

/// 约束定义（可序列化，供配置存储）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Constraint {
    #[serde(default)]
    pub name: Option<String>,
    pub expression: String,
    pub level: ConstraintLevel,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default = "default_true")]
    pub blocking: bool,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

fn default_true() -> bool {
    true
}

impl Constraint {
    pub fn new(expression: impl Into<String>) -> Self {
        Self {
            name: None,
            expression: expression.into(),
            level: ConstraintLevel::Field,
            error_message: None,
            error_code: None,
            active: true,
            blocking: true,
            metadata: HashMap::new(),
        }
    }

    pub fn field(expression: impl Into<String>) -> Self {
        Self::new(expression).at_level(ConstraintLevel::Field)
    }

    pub fn entity(expression: impl Into<String>) -> Self {
        Self::new(expression).at_level(ConstraintLevel::Entity)
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn at_level(mut self, level: ConstraintLevel) -> Self {
        self.level = level;
        self
    }

    pub fn with_error(mut self, message: impl Into<String>) -> Self {
        self.error_message = Some(message.into());
        self
    }

    pub fn parse_expression(&self) -> Result<ConstraintExpr, ConstraintParseError> {
        parse_constraint_expression(&self.expression)
    }

    pub fn referenced_fields(&self) -> Vec<String> {
        extract_field_references(&self.expression)
    }

    pub fn references_field(&self, field_name: &str) -> bool {
        self.referenced_fields().contains(&field_name.to_string())
    }

    pub fn default_error_message(&self) -> String {
        self.error_message
            .clone()
            .unwrap_or_else(|| format!("Constraint violated: {}", self.expression))
    }
}

/// 约束级别
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConstraintLevel {
    Field,
    Entity,
}

impl std::fmt::Display for ConstraintLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConstraintLevel::Field => write!(f, "field"),
            ConstraintLevel::Entity => write!(f, "entity"),
        }
    }
}

/// 约束违例
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintViolation {
    pub constraint_name: Option<String>,
    pub expression: String,
    pub message: String,
    #[serde(default)]
    pub entity: Option<String>,
    #[serde(default)]
    pub field: Option<String>,
    #[serde(default)]
    pub actual_values: HashMap<String, String>,
}

impl ConstraintViolation {
    pub fn new(expression: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            constraint_name: None,
            expression: expression.into(),
            message: message.into(),
            entity: None,
            field: None,
            actual_values: HashMap::new(),
        }
    }

    pub fn with_constraint_name(mut self, name: impl Into<String>) -> Self {
        self.constraint_name = Some(name.into());
        self
    }

    pub fn at_entity(mut self, entity: impl Into<String>) -> Self {
        self.entity = Some(entity.into());
        self
    }

    pub fn at_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    pub fn with_value(mut self, field: impl Into<String>, value: impl Into<String>) -> Self {
        self.actual_values.insert(field.into(), value.into());
        self
    }
}

/// 约束集合
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Constraints {
    #[serde(default)]
    pub constraints: Vec<Constraint>,
}

impl Constraints {
    pub fn new() -> Self {
        Self {
            constraints: Vec::new(),
        }
    }

    pub fn add(&mut self, constraint: Constraint) {
        self.constraints.push(constraint);
    }

    pub fn active(&self) -> Vec<&Constraint> {
        self.constraints.iter().filter(|c| c.active).collect()
    }

    pub fn field_constraints(&self) -> Vec<&Constraint> {
        self.constraints
            .iter()
            .filter(|c| c.active && c.level == ConstraintLevel::Field)
            .collect()
    }

    pub fn entity_constraints(&self) -> Vec<&Constraint> {
        self.constraints
            .iter()
            .filter(|c| c.active && c.level == ConstraintLevel::Entity)
            .collect()
    }

    pub fn for_field(&self, field_name: &str) -> Vec<&Constraint> {
        self.constraints
            .iter()
            .filter(|c| c.active && c.references_field(field_name))
            .collect()
    }
}

// ─────────────────────────────────────────────────────────────
// Parser
// ─────────────────────────────────────────────────────────────

/// 解析约束表达式字符串为 AST
pub fn parse_constraint_expression(input: &str) -> Result<ConstraintExpr, ConstraintParseError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(ConstraintParseError::EmptyExpression);
    }
    parse_or_expr(input)
}

/// 解析 OR 表达式（最低优先级）
fn parse_or_expr(input: &str) -> Result<ConstraintExpr, ConstraintParseError> {
    // flow 条件文法兼容：`||` 与 `OR` 等价
    if let Some(pos) = find_any_logical_op(input, &["OR", "||"]) {
        let op_len = 2; // `||` 与 `OR` 同为 2 字符，无需分支
        let left = &input[..pos].trim();
        let right = &input[pos + op_len..].trim();
        return Ok(ConstraintExpr::Or(
            Box::new(parse_or_expr(left)?),
            Box::new(parse_or_expr(right)?),
        ));
    }
    parse_and_expr(input)
}

/// 解析 AND 表达式
fn parse_and_expr(input: &str) -> Result<ConstraintExpr, ConstraintParseError> {
    // FEEL 子集（extend-dmn-decision-table-full D4）：`X between A and B` 三元
    // 比较。between 内含 and 关键字，必须先于普通 AND 拆分整体消费——否则
    // `y between 2 and 3` 会被拆成 `y between 2` AND `3`（错）。
    if let Some(btw) = try_parse_between(input) {
        return btw;
    }
    // flow 条件文法兼容：`&&` 与 `AND` 等价
    if let Some(pos) = find_any_logical_op(input, &["AND", "&&"]) {
        let op_len = if input[pos..].starts_with("&&") { 2 } else { 3 };
        let left = &input[..pos].trim();
        let right = &input[pos + op_len..].trim();
        return Ok(ConstraintExpr::And(
            Box::new(parse_and_expr(left)?),
            Box::new(parse_and_expr(right)?),
        ));
    }
    parse_comparison(input)
}

/// 尝试解析 FEEL between 三元形态：`X between A and B`（大小写不敏感），
/// 展开为 `X >= A and X <= B`（数值闭区间，A/B 可为字段引用——求值期取值）。
/// 仅当 between 位于括号外顶层且其左侧无更高层 AND/OR 组合时消费
/// （`a > 1 and b between 2 and 3` 的 between 属 AND 右操作数——外层先拆 AND，
/// 递归右段时本函数接管）。
fn try_parse_between(input: &str) -> Option<Result<ConstraintExpr, ConstraintParseError>> {
    let upper = input.to_uppercase();
    let between_pos = find_top_level_word(upper.as_str(), "BETWEEN")?;
    // between 左侧若含顶层 AND/OR → 该 and/or 优先级更高，先外层拆分
    if find_any_logical_op(&input[..between_pos], &["AND", "OR", "&&", "||"]).is_some() {
        return None;
    }
    let after = &input[between_pos + "BETWEEN".len()..];
    let and_rel = find_any_logical_op(after, &["AND", "&&"])?;
    let and_len = if after[and_rel..].starts_with("&&") {
        2
    } else {
        3
    };
    let left_src = input[..between_pos].trim();
    let mid_src = after[..and_rel].trim();
    let after_and = &after[and_rel + and_len..];
    // 右界 N 扫描至下一顶层逻辑 op（tail 保留原样继续递归组合）
    let tail_at =
        find_any_logical_op(after_and, &["AND", "OR", "&&", "||"]).unwrap_or(after_and.len());
    let right_src = after_and[..tail_at].trim();
    let tail = after_and[tail_at..].trim();
    if left_src.is_empty() || mid_src.is_empty() || right_src.is_empty() {
        return None;
    }
    // 展开为 `(X) >= (A) AND (X) <= (B)` 字符串再走统一递归（X 复用两次——
    // 纯表达式无副作用，文本重写语义无损）；tail 若存在则随外层 AND/OR 合并。
    let rewritten = if tail.is_empty() {
        format!("({left_src}) >= ({mid_src}) AND ({left_src}) <= ({right_src})")
    } else {
        format!("(({left_src}) >= ({mid_src}) AND ({left_src}) <= ({right_src})) {tail}")
    };
    Some(parse_and_expr(&rewritten))
}

/// 解析比较表达式
fn parse_comparison(input: &str) -> Result<ConstraintExpr, ConstraintParseError> {
    let input = input.trim();

    // 括号包裹：仅当整个串是单一括号表达式时剥壳。判定 = 首 `(` 开始计深，
    // 深度首次归零的位置必须是串尾——`(a) >= (b)` 中途归零 → 复合表达式
    // （含顶层比较），不剥壳（2026-09-07 FEEL between 展开暴露）。
    if input.starts_with('(') {
        let mut depth = 0i32;
        let mut is_single = false;
        for (i, c) in input.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth <= 0 {
                        is_single = i == input.len() - 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        if is_single {
            let inner = &input[1..input.len() - 1].trim();
            return parse_or_expr(inner);
        }
    }

    // 一元 NOT（flow 条件文法：`!expr`；`!=` 是二元运算符，须排除）
    if input.starts_with('!') && !input.starts_with("!=") {
        let inner = &input[1..].trim();
        if !inner.is_empty() {
            return Ok(ConstraintExpr::Not(Box::new(parse_comparison(inner)?)));
        }
    }
    // NOT 前缀
    if input.to_uppercase().starts_with("NOT ") {
        let inner = &input[4..].trim();
        return Ok(ConstraintExpr::Not(Box::new(parse_comparison(inner)?)));
    }

    // 比较运算符（从左到右扫描，跳过函数调用内部）；
    // flow 条件文法：`in`（列表包含）与 `contains`（子串）为单词运算符
    let ops = [
        ("<=", BinaryOp::Le),
        (">=", BinaryOp::Ge),
        ("!=", BinaryOp::Ne),
        ("==", BinaryOp::Eq),
        ("=", BinaryOp::Eq),
        ("<", BinaryOp::Lt),
        (">", BinaryOp::Gt),
        ("contains", BinaryOp::Contains),
        ("in", BinaryOp::In),
    ];

    let mut depth = 0i32;
    for (i, c) in input.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ if depth > 0 => continue,
            _ => {}
        }

        for (op_str, op) in &ops {
            if input[i..].starts_with(op_str) {
                let after_pos = i + op_str.len();
                if after_pos < input.len() {
                    let after_char = input.chars().nth(after_pos).unwrap();
                    if after_char.is_alphanumeric() || after_char == '_' {
                        continue;
                    }
                }

                let left = &input[..i].trim();
                let right = &input[after_pos..].trim();

                // FEEL 区间（extend-dmn-decision-table-full D4）：`X in (A..B]` /
                // `[A..B)` / `(A..B)` / `[A..B]` ——开区间端点严格比较。右值
                // 特征：以括号起止且内部含 `..`（区别于 `in [a, b, c]` 列表）。
                if *op == BinaryOp::In && right.contains("..") {
                    if let Some(rng) = parse_range_comparison(left, right) {
                        return rng;
                    }
                }

                return Ok(ConstraintExpr::Binary(
                    Box::new(parse_additive_expr(left)?),
                    *op,
                    Box::new(parse_additive_expr(right)?),
                ));
            }
        }
    }

    // 无比较运算符，按算术表达式解析
    parse_additive_expr(input)
}

/// FEEL 开闭区间 `X in (A..B]` 形态：右值须以 `(`/`[` 起、`)`/`]` 止（可混合），
/// 内部含顶层 `..` 分隔端点。展开为两个比较的 And（开端点严格 >/闭 >=，
/// 端止点严格 </闭 <=）。端点 A/B 经 parse_additive_expr 解析（可为数值/字段
/// 引用/算术——求值期取值）。语法形态不合法返回 None（调用方回落既有
/// `in [list]`/普通二元路径报错）。
fn parse_range_comparison(
    left: &str,
    right: &str,
) -> Option<Result<ConstraintExpr, ConstraintParseError>> {
    let right = right.trim();
    if right.len() < 5 {
        return None;
    }
    let (open_c, close_c) = (right.as_bytes()[0] as char, right.chars().last().unwrap());
    let open_ok = open_c == '(' || open_c == '[';
    let close_ok = close_c == ')' || close_c == ']';
    if !open_ok || !close_ok {
        return None;
    }
    let inner = &right[1..right.len() - 1];
    // 列表形态防护：`[` 起始的内层含逗号或引号 → 是 `[a, b]` 列表/字符串
    // 元素（如 `['a..b','c']`），回落普通路径；区间端点纯数值/字段/算术。
    if open_c == '[' && (inner.contains(',') || inner.contains('\'') || inner.contains('"')) {
        return None;
    }
    if (open_c == '(' || close_c == ')') && (inner.contains('\'') || inner.contains('"')) {
        return None;
    }
    // 顶层 `..` 定位（端点可含括号/字段点号；`..` 双点整体为分隔符）
    let mut depth = 0i32;
    let mut found = None;
    for (i, c) in inner.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '.' if depth == 0 && inner[i..].starts_with("..") => {
                found = Some(i);
                break;
            }
            _ => {}
        }
    }
    let sep = found?;
    let lo_src = inner[..sep].trim();
    let hi_src = inner[sep + 2..].trim();
    if lo_src.is_empty() || hi_src.is_empty() {
        return None;
    }
    let l_op = if open_c == '(' {
        BinaryOp::Gt
    } else {
        BinaryOp::Ge
    };
    let h_op = if close_c == ')' {
        BinaryOp::Lt
    } else {
        BinaryOp::Le
    };
    let left_expr = || parse_additive_expr(left);
    let lo_expr = || parse_additive_expr(lo_src);
    let hi_expr = || parse_additive_expr(hi_src);
    // X 重复出现两次（lo 与 hi 各一）——纯表达式复制无副作用；解析失败冒泡。
    Some(match (left_expr(), lo_expr()) {
        (Ok(lx), Ok(lo)) => {
            let lower = ConstraintExpr::Binary(Box::new(lx), l_op, Box::new(lo));
            match (left_expr(), hi_expr()) {
                (Ok(lx2), Ok(hi)) => Ok(ConstraintExpr::And(
                    Box::new(lower),
                    Box::new(ConstraintExpr::Binary(Box::new(lx2), h_op, Box::new(hi))),
                )),
                (Err(e), _) | (_, Err(e)) => Err(e),
            }
        }
        (Err(e), _) | (_, Err(e)) => Err(e),
    })
}

/// 解析加减表达式
fn parse_additive_expr(input: &str) -> Result<ConstraintExpr, ConstraintParseError> {
    let input = input.trim();

    // flow 条件文法：连字符标识符（如 `act-group`，WZ 叶表物理列名）整体
    // 解析为字段引用——无空格减法禁用（`a - b` 带空格仍按减法；`a-b` 按
    // 标识符；一元负号/数字字面量不受影响）
    if is_flow_identifier(input) {
        return parse_term_expr(input);
    }

    // 一元负号
    if let Some(rest) = input.strip_prefix('-') {
        let inner = rest.trim();
        if !inner.is_empty() {
            return Ok(ConstraintExpr::Unary(
                UnaryOp::Neg,
                Box::new(parse_additive_expr(inner)?),
            ));
        }
    }

    let mut depth = 0i32;
    for (i, c) in input.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => depth -= 1,
            _ if depth > 0 => continue,
            '+' => {
                let left = &input[..i].trim();
                let right = &input[i + 1..].trim();
                if !left.is_empty() && !right.is_empty() {
                    return Ok(ConstraintExpr::Binary(
                        Box::new(parse_additive_expr(left)?),
                        BinaryOp::Add,
                        Box::new(parse_multiplicative_expr(right)?),
                    ));
                }
            }
            '-' => {
                let left = &input[..i].trim();
                let right = &input[i + 1..].trim();
                if !left.is_empty() && !right.is_empty() {
                    return Ok(ConstraintExpr::Binary(
                        Box::new(parse_additive_expr(left)?),
                        BinaryOp::Sub,
                        Box::new(parse_multiplicative_expr(right)?),
                    ));
                }
            }
            _ => {}
        }
    }

    parse_multiplicative_expr(input)
}

/// 解析乘除模表达式
fn parse_multiplicative_expr(input: &str) -> Result<ConstraintExpr, ConstraintParseError> {
    let input = input.trim();

    let mut depth = 0i32;
    for (i, c) in input.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => depth -= 1,
            _ if depth > 0 => continue,
            '*' => {
                let left = &input[..i].trim();
                let right = &input[i + 1..].trim();
                if !left.is_empty() && !right.is_empty() {
                    return Ok(ConstraintExpr::Binary(
                        Box::new(parse_multiplicative_expr(left)?),
                        BinaryOp::Mul,
                        Box::new(parse_term_expr(right)?),
                    ));
                }
            }
            '/' => {
                let left = &input[..i].trim();
                let right = &input[i + 1..].trim();
                if !left.is_empty() && !right.is_empty() {
                    return Ok(ConstraintExpr::Binary(
                        Box::new(parse_multiplicative_expr(left)?),
                        BinaryOp::Div,
                        Box::new(parse_term_expr(right)?),
                    ));
                }
            }
            '%' => {
                let left = &input[..i].trim();
                let right = &input[i + 1..].trim();
                if !left.is_empty() && !right.is_empty() {
                    return Ok(ConstraintExpr::Binary(
                        Box::new(parse_multiplicative_expr(left)?),
                        BinaryOp::Mod,
                        Box::new(parse_term_expr(right)?),
                    ));
                }
            }
            _ => {}
        }
    }

    parse_term_expr(input)
}

/// 解析原子项（字面量、字段引用、函数调用、括号表达式）
fn parse_term_expr(input: &str) -> Result<ConstraintExpr, ConstraintParseError> {
    let input = input.trim();

    if input.is_empty() {
        return Err(ConstraintParseError::EmptyExpression);
    }

    // 字符串字面量
    if (input.starts_with('"') && input.ends_with('"'))
        || (input.starts_with('\'') && input.ends_with('\''))
    {
        let value = &input[1..input.len() - 1];
        return Ok(ConstraintExpr::Literal(ConstraintLiteral::String(
            value.to_string(),
        )));
    }

    // 数值字面量
    if let Ok(num) = input.parse::<i64>() {
        return Ok(ConstraintExpr::Literal(ConstraintLiteral::Integer(num)));
    }
    if let Ok(num) = input.parse::<f64>() {
        return Ok(ConstraintExpr::Literal(ConstraintLiteral::Decimal(num)));
    }

    // 布尔/Null 字面量
    if input.eq_ignore_ascii_case("true") {
        return Ok(ConstraintExpr::Literal(ConstraintLiteral::Boolean(true)));
    }
    if input.eq_ignore_ascii_case("false") {
        return Ok(ConstraintExpr::Literal(ConstraintLiteral::Boolean(false)));
    }
    if input.eq_ignore_ascii_case("null") {
        return Ok(ConstraintExpr::Literal(ConstraintLiteral::Null));
    }

    // 列表字面量（flow 条件文法：`in [a, b, c]` 右值）
    if input.starts_with('[') && input.ends_with(']') {
        let inner = &input[1..input.len() - 1].trim();
        if inner.is_empty() {
            return Ok(ConstraintExpr::Literal(ConstraintLiteral::List(Vec::new())));
        }
        let mut items = Vec::new();
        for arg in split_top_level_commas(inner) {
            match parse_term_expr(arg)? {
                ConstraintExpr::Literal(lit) => items.push(lit),
                _ => {
                    return Err(ConstraintParseError::InvalidSyntax(format!(
                        "list element must be a literal: {}",
                        arg
                    )))
                }
            }
        }
        return Ok(ConstraintExpr::Literal(ConstraintLiteral::List(items)));
    }

    // 括号表达式
    if input.starts_with('(') && input.ends_with(')') {
        let inner = &input[1..input.len() - 1].trim();
        return parse_or_expr(inner);
    }

    // 函数调用: name(arg1, arg2)
    if let Some(paren_pos) = input.find('(') {
        if input.ends_with(')') {
            let name = &input[..paren_pos].trim();
            let args_str = &input[paren_pos + 1..input.len() - 1].trim();
            let args = parse_arguments(args_str)?;
            return Ok(ConstraintExpr::Call(name.to_string(), args));
        }
    }

    // 字段引用
    if is_valid_identifier(input) {
        return Ok(ConstraintExpr::FieldRef(input.to_string()));
    }

    Err(ConstraintParseError::InvalidSyntax(format!(
        "Cannot parse: {}",
        input
    )))
}

/// 解析函数参数列表
/// 顶层逗号拆分（列表字面量元素；跳过嵌套括号/方括号）
fn split_top_level_commas(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in input.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(input[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(input[start..].trim());
    parts
}

fn parse_arguments(input: &str) -> Result<Vec<ConstraintExpr>, ConstraintParseError> {
    if input.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = Vec::new();
    for arg in split_top_level_commas(input) {
        args.push(parse_term_expr(arg)?);
    }
    Ok(args)
}

/// 在括号外部查找逻辑运算符位置
fn find_logical_op(input: &str, op: &str) -> Option<usize> {
    let upper = input.to_uppercase();
    let mut depth = 0;

    for (i, c) in input.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ if depth == 0 && upper[i..].starts_with(op) => {
                let after = i + op.len();
                if after >= input.len() || !input.chars().nth(after).unwrap().is_alphanumeric() {
                    return Some(i);
                }
            }
            _ => {}
        }
    }

    None
}

/// 多个候选逻辑运算符中取最左命中（flow 条件文法：`||`/`&&` 与 OR/AND 等价）
fn find_any_logical_op(input: &str, ops: &[&str]) -> Option<usize> {
    ops.iter().filter_map(|op| find_logical_op(input, op)).min()
}

/// 在括号外部查找单词关键字位置（大小写不敏感；词边界=前后非字母数字）。
/// 入参 input 须为与原文等长的 ascii 大写副本（BETWEEN 等长映射回原文索引）。
fn find_top_level_word(upper_input: &str, word: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = 0;
    while i < upper_input.len() {
        let c = upper_input[i..].chars().next().unwrap();
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ if depth == 0 && upper_input[i..].starts_with(word) => {
                let after = i + word.len();
                let before_ok =
                    i == 0 || !upper_input[..i].chars().last().unwrap().is_alphanumeric();
                let after_ok = after >= upper_input.len()
                    || !upper_input[after..]
                        .chars()
                        .next()
                        .unwrap()
                        .is_alphanumeric();
                if before_ok && after_ok {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += c.len_utf8();
    }
    None
}
/// 检查是否为合法标识符（flow 条件文法允许连字符：`act-group` 等叶表物理列名；
/// 2026-09-01 起允许点号——`_refs.ck_category.notice` 成员访问，外键列经
/// `_refs` 模式访问的模型设计规则）
fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let first = s.chars().next().unwrap();
    if !first.is_alphabetic() && first != '_' {
        return false;
    }
    s.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
}
/// 无空格连字符标识符（`act-group`；`a - b` 带空格按减法）
fn is_flow_identifier(s: &str) -> bool {
    s.contains('-') && !s.contains(char::is_whitespace) && is_valid_identifier(s)
}

/// 从表达式字符串中提取字段引用
pub fn extract_field_references(expression: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let tokens: Vec<&str> = expression.split_whitespace().collect();

    for token in tokens {
        let clean = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if is_valid_identifier(clean) && !is_keyword(clean) {
            fields.push(clean.to_string());
        }
    }

    fields.sort();
    fields.dedup();
    fields
}

fn is_keyword(s: &str) -> bool {
    let keywords = ["AND", "OR", "NOT", "true", "false", "null"];
    keywords.contains(&s.to_uppercase().as_str())
}
