//! 表达式静态分析（唯一引擎 Rhai 的 AST 只读遍历）
//!
//! 面向**发布期静态检查**（边可达性 / DMN 规则死亡判定）与**作者面字段校验**
//! （AI 助手变量清单、DMN 单元格字段提取）。
//!
//! 依赖 rhai `internals` feature：读 `AST`/`Expr`/`Stmt` 的公开结构。
//! 约定：**不支持的形态一律保守返回**（`None` / 空集），MUST NOT 猜测语义
//! ——静态检查只做"能证明"的判定，证明不了则放行（与旧 DSL 分析同口径）。

use rhai::{Expr, Stmt};
use serde_json::Value;

/// 比较运算（静态分析支持的比较子集；`contains`/字段间比较等不支持）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    fn of(name: &str) -> Option<Self> {
        match name {
            "==" => Some(Self::Eq),
            "!=" => Some(Self::Ne),
            "<" => Some(Self::Lt),
            "<=" => Some(Self::Le),
            ">" => Some(Self::Gt),
            ">=" => Some(Self::Ge),
            _ => None,
        }
    }

    /// 取反（`!` 下沉）
    fn negate(self) -> Self {
        match self {
            Self::Eq => Self::Ne,
            Self::Ne => Self::Eq,
            Self::Lt => Self::Ge,
            Self::Le => Self::Gt,
            Self::Gt => Self::Le,
            Self::Ge => Self::Lt,
        }
    }
}

/// 比较原子：`字段 CmpOp 字面量`
#[derive(Debug, Clone, PartialEq)]
pub struct ComparisonAtom {
    pub field: String,
    pub op: CmpOp,
    pub literal: Value,
}

/// 解析表达式为 Rhai AST（语法错 → Err）
pub fn parse(script: &str) -> Result<rhai::AST, String> {
    rhai::Engine::new()
        .compile(script)
        .map_err(|e| format!("rhai parse: {e}"))
}

/// AST 根表达式（顶层单表达式；多语句脚本取最后一个表达式/调用语句）
///
/// 注意：Rhai 把「单个函数调用构成的语句」存为 `Stmt::FnCall`（比较运算亦为
/// 函数调用），故两类都要取。
fn root_expr(ast: &rhai::AST) -> Option<Expr> {
    for stmt in ast.statements().iter().rev() {
        match stmt {
            Stmt::Expr(e) => return Some((**e).clone()),
            Stmt::FnCall(fc, pos) => return Some(Expr::FnCall(fc.clone(), *pos)),
            _ => {}
        }
    }
    None
}

/// 字面量 → JSON（非字面量 → None）
fn literal_of(e: &Expr) -> Option<Value> {
    match e {
        Expr::BoolConstant(b, _) => Some(Value::Bool(*b)),
        Expr::IntegerConstant(i, _) => Some(Value::from(*i)),
        Expr::FloatConstant(f, _) => serde_json::Number::from_f64(**f).map(Value::Number),
        Expr::StringConstant(s, _) => Some(Value::String(s.to_string())),
        Expr::CharConstant(c, _) => Some(Value::String(c.to_string())),
        Expr::Unit(_) => Some(Value::Null),
        Expr::DynamicConstant(d, _) => rhai::serde::from_dynamic::<Value>(d).ok(),
        Expr::Array(items, _) => items
            .iter()
            .map(literal_of)
            .collect::<Option<Vec<Value>>>()
            .map(Value::Array),
        _ => None,
    }
}

/// 变量引用名（含 `a.b.c` / `a["k"]` 路径的字符串还原；`ctx["x"]` → `x`）。
/// 非变量/属性链 → None。
fn var_path(e: &Expr) -> Option<String> {
    let (root, keys) = index_path(e)?;
    if keys.is_empty() {
        return Some(root);
    }
    let joined = keys.join(".");
    // `ctx[...]` 归一为键名本身（ctx 为整体映射容器）
    if root == "ctx" {
        Some(joined)
    } else {
        Some(format!("{root}.{joined}"))
    }
}

/// 展开「变量 + 键链」：`a["k1"]["k2"]` / `a.b.c` ⇒ ("a", ["k1","k2"] / ["b","c"])
fn index_path(e: &Expr) -> Option<(String, Vec<String>)> {
    match e {
        Expr::Variable(v, ..) => Some((v.1.to_string(), Vec::new())),
        Expr::Dot(b, _, _) => {
            let (root, mut keys) = index_path(&b.lhs)?;
            match &b.rhs {
                Expr::Property(p, _) => keys.push(p.2.to_string()),
                _ => return None,
            }
            Some((root, keys))
        }
        Expr::Index(b, _, _) => {
            let (root, mut keys) = index_path(&b.lhs)?;
            // Rhai 对链式索引把「后续键」压进 rhs（`a["k1"]["k2"]` ⇒
            // Index { lhs: a, rhs: Index { lhs: "k1", rhs: "k2" } }）
            match index_keys(&b.rhs) {
                Some(mut more) => {
                    keys.append(&mut more);
                    Some((root, keys))
                }
                None => None,
            }
        }
        _ => None,
    }
}

/// 索引键序列（`"k"` / `"k1"` ++ `"k2"` 链）
fn index_keys(e: &Expr) -> Option<Vec<String>> {
    match e {
        Expr::StringConstant(s, _) => Some(vec![s.to_string()]),
        Expr::Index(b, _, _) => {
            let mut keys = index_keys(&b.lhs)?;
            keys.append(&mut index_keys(&b.rhs)?);
            Some(keys)
        }
        _ => None,
    }
}

/// 遍历表达式，收集全部变量引用路径（去重、保持首次出现顺序）。
pub fn collect_variables(script: &str) -> Result<Vec<String>, String> {
    let ast = parse(script)?;
    let mut out: Vec<String> = Vec::new();
    for stmt in ast.statements() {
        stmt_vars(stmt, &mut out);
    }
    Ok(out)
}

fn push_var(name: String, out: &mut Vec<String>) {
    if !out.contains(&name) {
        out.push(name);
    }
}

fn stmt_vars(stmt: &Stmt, out: &mut Vec<String>) {
    match stmt {
        Stmt::Expr(e) => expr_vars(e, out),
        Stmt::FnCall(fc, _) => fc.args.iter().for_each(|a| expr_vars(a, out)),
        Stmt::Var(v, _, _) => expr_vars(&v.1, out),
        Stmt::Assignment(a) => {
            expr_vars(&a.1.lhs, out);
            expr_vars(&a.1.rhs, out);
        }
        Stmt::If(f, _) | Stmt::While(f, _) | Stmt::Do(f, _, _) => {
            expr_vars(&f.expr, out);
            f.body.iter().for_each(|s| stmt_vars(s, out));
            f.branch.iter().for_each(|s| stmt_vars(s, out));
        }
        Stmt::For(f, _) => {
            expr_vars(&f.2.expr, out);
            f.2.body.iter().for_each(|s| stmt_vars(s, out));
        }
        Stmt::Block(b) => b.iter().for_each(|s| stmt_vars(s, out)),
        Stmt::Return(Some(e), _, _) => expr_vars(e, out),
        _ => {}
    }
}

fn expr_vars(e: &Expr, out: &mut Vec<String>) {
    // 属性/索引链整体作为一个路径（`_refs.a.b` / `ctx["k"]`）；其余形态逐层下钻
    if let Some(p) = var_path(e) {
        push_var(p, out);
        return;
    }
    match e {
        Expr::And(list, _) | Expr::Or(list, _) | Expr::Coalesce(list, _) => {
            list.iter().for_each(|x| expr_vars(x, out));
        }
        Expr::Array(items, _) => items.iter().for_each(|x| expr_vars(x, out)),
        Expr::InterpolatedString(items, _) => items.iter().for_each(|x| expr_vars(x, out)),
        Expr::FnCall(fc, _) | Expr::MethodCall(fc, _) => {
            fc.args.iter().for_each(|a| expr_vars(a, out));
        }
        Expr::Dot(b, _, _) | Expr::Index(b, _, _) => {
            expr_vars(&b.lhs, out);
            expr_vars(&b.rhs, out);
        }
        _ => {}
    }
}

/// 是否「可作自由函数名」的标识符形态。
/// 用于排除 Rhai 把运算符/`in` 降级成的 `FnCall`（`&&`/`==`/`!`/`+` 等非标识符形态，
/// 以及 `x in [..]` 降级的 `contains([..], x)` 之流）——本仓库的 R8 白名单只约束**自由函数**，
/// 运算符由语言本身承担（见 `ALIOTH_ONTOLOGY_SPEC.md` 作者面文法）。
fn is_fn_ident(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 遍历 AST，收集**自由函数**调用名（去重、保持首现顺序）。
/// 方法调用（`Expr::MethodCall`，如 `s.trim()`）不计：方法面由 Rhai 类型系统承担。
pub fn collect_calls(script: &str) -> Result<Vec<String>, String> {
    let ast = parse(script)?;
    let mut out: Vec<String> = Vec::new();
    for stmt in ast.statements() {
        stmt_calls(stmt, &mut out);
    }
    Ok(out)
}

fn push_call(name: &str, out: &mut Vec<String>) {
    if is_fn_ident(name) && !out.iter().any(|n| n == name) {
        out.push(name.to_string());
    }
}

fn stmt_calls(stmt: &Stmt, out: &mut Vec<String>) {
    match stmt {
        Stmt::Expr(e) => expr_calls(e, out),
        Stmt::FnCall(fc, _) => {
            push_call(fc.name.as_str(), out);
            fc.args.iter().for_each(|a| expr_calls(a, out));
        }
        Stmt::Var(v, _, _) => expr_calls(&v.1, out),
        Stmt::Assignment(a) => {
            expr_calls(&a.1.lhs, out);
            expr_calls(&a.1.rhs, out);
        }
        Stmt::If(f, _) | Stmt::While(f, _) | Stmt::Do(f, _, _) => {
            expr_calls(&f.expr, out);
            f.body.iter().for_each(|s| stmt_calls(s, out));
            f.branch.iter().for_each(|s| stmt_calls(s, out));
        }
        Stmt::For(f, _) => {
            expr_calls(&f.2.expr, out);
            f.2.body.iter().for_each(|s| stmt_calls(s, out));
        }
        Stmt::Block(b) => b.iter().for_each(|s| stmt_calls(s, out)),
        Stmt::Return(Some(e), _, _) => expr_calls(e, out),
        _ => {}
    }
}

fn expr_calls(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::And(list, _) | Expr::Or(list, _) | Expr::Coalesce(list, _) => {
            list.iter().for_each(|x| expr_calls(x, out));
        }
        Expr::Array(items, _) => items.iter().for_each(|x| expr_calls(x, out)),
        Expr::InterpolatedString(items, _) => items.iter().for_each(|x| expr_calls(x, out)),
        Expr::FnCall(fc, _) => {
            push_call(fc.name.as_str(), out);
            fc.args.iter().for_each(|a| expr_calls(a, out));
        }
        Expr::MethodCall(fc, _) => {
            fc.args.iter().for_each(|a| expr_calls(a, out));
        }
        Expr::Dot(b, _, _) | Expr::Index(b, _, _) => {
            expr_calls(&b.lhs, out);
            expr_calls(&b.rhs, out);
        }
        _ => {}
    }
}

/// AND 链分解为 `字段 CmpOp 字面量` 原子。
///
/// 语义口径（保守，与旧 DSL 分析等价并修一处旧缺陷）：
/// - `&&` 链递归分解；`!` 仅支持对 Eq/Ne 取反（`!` 下沉）
/// - `x in [a, b]`（Rhai 编译为 `contains([a,b], x)`）展开为 `x == a` / `x == b`
/// - 非 Eq/Ne 比较、OR、方法调用、函数调用、块语句等 → `Ok(None)`（不可判定，放行）
/// - 恒假字面量 → `Ok(None)`（真/假常量由调用方先经常量求值处理）
pub fn and_atoms(script: &str) -> Result<Option<Vec<ComparisonAtom>>, String> {
    let ast = parse(script)?;
    let Some(root) = root_expr(&ast) else {
        return Ok(None);
    };
    let mut out = Vec::new();
    if collect_and(&root, &mut out) {
        Ok(Some(out))
    } else {
        Ok(None)
    }
}

/// 收集 `&&` 链原子；返回 false = 不支持形态（保守放弃）
fn collect_and(e: &Expr, out: &mut Vec<ComparisonAtom>) -> bool {
    match e {
        Expr::And(list, _) => list.iter().all(|x| collect_and(x, out)),
        Expr::FnCall(fc, _) => {
            let name = fc.name.as_str();
            // 取反下沉：`!` 仅支持比较（Eq ↔ Ne）
            if name == "!" && fc.args.len() == 1 {
                return negate_and_collect(&fc.args[0], out);
            }
            if let Some(op) = CmpOp::of(name) {
                return collect_comparison(fc.args.first(), fc.args.get(1), op, out);
            }
            // `x in [..]` → contains(array, x)：逐元素展开为 Eq
            // （字面量数组会被 Rhai 优化器折叠为 DynamicConstant，两种形态都要认）
            if name == "contains" && fc.args.len() == 2 {
                let (first, second) = (&fc.args[0], &fc.args[1]);
                let elements: Option<Vec<Value>> = match first {
                    Expr::Array(items, _) => items.iter().map(literal_of).collect(),
                    other => match literal_of(other) {
                        Some(Value::Array(items)) => Some(items),
                        _ => None,
                    },
                };
                if let Some(elements) = elements {
                    let Some(field) = var_path(second) else {
                        return false;
                    };
                    for lit in elements {
                        out.push(ComparisonAtom {
                            field: field.clone(),
                            op: CmpOp::Eq,
                            literal: lit,
                        });
                    }
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

fn negate_and_collect(inner: &Expr, out: &mut Vec<ComparisonAtom>) -> bool {
    let Expr::FnCall(fc, _) = inner else {
        return false;
    };
    match CmpOp::of(fc.name.as_str()) {
        Some(op) => collect_comparison(fc.args.first(), fc.args.get(1), op.negate(), out),
        // `!(x in [..])` → contains(array, x) 取反：exclude 语义，超出原子表达力 → 放弃
        None => false,
    }
}

fn collect_comparison(
    lhs: Option<&Expr>,
    rhs: Option<&Expr>,
    op: CmpOp,
    out: &mut Vec<ComparisonAtom>,
) -> bool {
    let (Some(lhs), Some(rhs)) = (lhs, rhs) else {
        return false;
    };
    // 仅支持 `字段 OP 字面量`（镜像形态 `字面量 OP 字段` 保守放弃）
    let (Some(field), Some(literal)) = (var_path(lhs), literal_of(rhs)) else {
        return false;
    };
    out.push(ComparisonAtom { field, op, literal });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn collect_calls_reports_only_identifier_shaped_calls() {
        assert_eq!(
            collect_calls("sum(values)").unwrap(),
            vec!["sum".to_string()]
        );
        assert_eq!(
            collect_calls("sum_by(rows, \"additions\") + count(rows)").unwrap(),
            vec!["sum_by".to_string(), "count".to_string()]
        );
        // 方法调用不计（字符串面走 Rhai 方法形态）
        assert!(collect_calls("let t = title; t.trim(); t")
            .unwrap()
            .is_empty());
        // 运算符降级不计（`&&`/`==`/`!` 非标识符形态）
        assert!(collect_calls("a > 1 && b != ()").unwrap().is_empty());
        // `x in [..]` 由 Rhai 降级为 `contains(..)` ⇒ 计入（白名单 MUST 可枚举它）
        assert_eq!(
            collect_calls("x in [\"a\", \"b\"]").unwrap(),
            vec!["contains".to_string()]
        );
    }

    #[test]
    fn test_collect_variables_paths() {
        let vars =
            collect_variables("_refs[\"cnt-status\"][\"code\"] == \"X\" && amount > 100").unwrap();
        assert!(vars.contains(&"_refs.cnt-status.code".to_string()));
        assert!(vars.contains(&"amount".to_string()));
    }

    #[test]
    fn test_collect_variables_ctx_index_normalized() {
        let vars = collect_variables("ctx[\"act-group\"] == 1").unwrap();
        assert_eq!(vars, vec!["act-group".to_string()]);
    }

    #[test]
    fn test_collect_variables_multi_statement() {
        let vars = collect_variables("let s = title; s.trim(); s != \"\"").unwrap();
        assert!(vars.contains(&"title".to_string()));
    }

    #[test]
    fn test_and_atoms_eq_ne() {
        let atoms = and_atoms("amount == 100 && cursor != 3")
            .unwrap()
            .expect("可判定");
        assert_eq!(
            atoms,
            vec![
                ComparisonAtom {
                    field: "amount".into(),
                    op: CmpOp::Eq,
                    literal: json!(100)
                },
                ComparisonAtom {
                    field: "cursor".into(),
                    op: CmpOp::Ne,
                    literal: json!(3)
                },
            ]
        );
    }

    #[test]
    fn test_and_atoms_negation_pushdown() {
        let atoms = and_atoms("!(amount == 100)").unwrap().unwrap();
        assert_eq!(atoms[0].op, CmpOp::Ne);
        let atoms = and_atoms("!(status != \"X\")").unwrap().unwrap();
        assert_eq!(atoms[0].op, CmpOp::Eq);
    }

    #[test]
    fn test_and_atoms_in_list_expansion() {
        let atoms = and_atoms("status in [\"A\", \"B\"]").unwrap().unwrap();
        assert_eq!(atoms.len(), 2);
        assert!(atoms
            .iter()
            .all(|a| a.field == "status" && a.op == CmpOp::Eq));
        assert_eq!(atoms[0].literal, json!("A"));
        assert_eq!(atoms[1].literal, json!("B"));
    }

    #[test]
    fn test_and_atoms_comparators_and_conservative() {
        // 六种比较子均产出原子（区间分析由调用方使用）
        assert_eq!(and_atoms("amount >= 5").unwrap().unwrap()[0].op, CmpOp::Ge);
        assert_eq!(and_atoms("amount < 3").unwrap().unwrap()[0].op, CmpOp::Lt);
        // 不支持形态 → 不可判定（None）
        assert!(and_atoms("a == 1 || b == 2").unwrap().is_none());
        assert!(and_atoms("s.contains(\"x\")").unwrap().is_none());
        assert!(and_atoms("amount == 5 && { let x = 1; x > 0 }")
            .unwrap()
            .is_none());
        // 恒假字面量 → 交由常量求值处理
        assert!(and_atoms("false").unwrap().is_none());
        // 字段对字段比较 → 放弃
        assert!(and_atoms("a == b").unwrap().is_none());
    }

    #[test]
    fn test_parse_rejects_syntax_error() {
        assert!(collect_variables("amount > ((").is_err());
        assert!(and_atoms("amount == ").is_err());
    }
}
