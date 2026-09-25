//! 受控内建函数注册表（表达式可用**自由函数**的唯一白名单）。
//!
//! 单一真相源 = 同目录 `builtins.json`：
//! - Rust 侧 `include_str!` 解析并**据此注册**（求值引擎与校验引擎同源，避免"校验过、求值崩"）；
//! - 门禁 `scripts/check/check-expression-dialect.ts`（R8）读取**同一文件**判定
//!   「表达式自由函数 MUST ∈ 白名单」；文档亦以该清单为准。
//!
//! 冻结面（MUST NOT 登记）：IO / 进程 / 时间 / 随机 / 反射类。注册**不放开沙箱**——
//! 安全模式与"无 IO 函数"约束不变（测试锚定 `read_file`/`process` 仍失败）。
//!
//! 起因（2026-09-22 探针）：引擎此前**零注册**，而授权面已用 `sum(...)` ⇒ 运行期
//! `Function not found`，且写径与门禁都拦不住（`validate` 只查标识符）。

use rhai::{Array, Dynamic, Engine, ImmutableString, Map};
use serde::Deserialize;
use std::sync::LazyLock;

/// 白名单条目（字段名与 `builtins.json` 一一对应）。
#[derive(Debug, Clone, Deserialize)]
pub struct BuiltinFn {
    pub name: String,
    pub arity: usize,
    /// `platform`（本仓实现）| `rhai`（Rhai 内建，显式登记以便枚举）
    pub origin: String,
    pub doc: String,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    functions: Vec<BuiltinFn>,
}

static MANIFEST: LazyLock<Vec<BuiltinFn>> = LazyLock::new(|| {
    let raw = include_str!("builtins.json");
    serde_json::from_str::<Manifest>(raw)
        .expect("builtins.json 必须可解析（注册表单一真相源）")
        .functions
});

/// 全部登记的自由函数（门禁/文档/测试共同消费）。
pub fn manifest() -> &'static [BuiltinFn] {
    &MANIFEST
}

/// 登记的自由函数名清单。
pub fn names() -> Vec<String> {
    MANIFEST.iter().map(|f| f.name.clone()).collect()
}

/// 名称是否在受控白名单内（R8 判据的唯一实现）。
pub fn is_registered(name: &str) -> bool {
    MANIFEST.iter().any(|f| f.name == name)
}

/// 注册受控函数面。**唯一注册点**（`build_engine` 调用；其他处 MUST NOT 注册函数）。
///
/// **MUST NOT 注册与 Rhai 原地变异方法同名者**（`trim`、`make_lower` 等）：同名注册会参与
/// 方法解析，使 `code.trim()` 不再原地变异 ⇒ 常量契约（ctx 值不可变）失效、且
/// `let t = x; t.trim(); t` 规范形态静默失真（2026-09-22 实测回归，由 `rhai.rs` 既有测试捕获）。
/// 字符串/数组方法沿用 Rhai 方法形态，不入白名单。
/// `origin = rhai` 的条目（`abs`/`type_of`）为 Rhai 内建自由函数，**不在此重复注册**，仅登记以便枚举。
pub fn register(engine: &mut Engine) {
    engine.register_fn("sum", |values: Array| sum_of(&values));
    engine.register_fn("sum_by", |rows: Array, field: ImmutableString| {
        sum_by(&rows, &field)
    });
    engine.register_fn("avg", |values: Array| avg_of(&values));
    engine.register_fn("count", |values: Array| values.len() as i64);
    engine.register_fn("min", |values: Array| min_of(&values));
    engine.register_fn("max", |values: Array| max_of(&values));
    engine.register_fn("round", |x: f64| x.round());
    engine.register_fn("floor", |x: f64| x.floor());
    engine.register_fn("ceil", |x: f64| x.ceil());
}

// ── 聚合实现（整数优先：全整数 → i64；含小数 → f64）──

/// 数值视图：整数 / 浮点（其他类型视为非数值，忽略）。
enum Num {
    Int(i64),
    Float(f64),
}

fn as_num(value: &Dynamic) -> Option<Num> {
    if let Some(i) = value.clone().try_cast::<i64>() {
        return Some(Num::Int(i));
    }
    value.clone().try_cast::<f64>().map(Num::Float)
}

fn fold(values: &[Dynamic]) -> Option<(i64, f64, bool, usize)> {
    // (整数和, 浮点和, 是否全整数, 数值个数)
    let mut int_sum = 0i64;
    let mut float_sum = 0f64;
    let mut all_int = true;
    let mut n = 0usize;
    for v in values {
        match as_num(v) {
            Some(Num::Int(i)) => {
                int_sum = int_sum.saturating_add(i);
                float_sum += i as f64;
                n += 1;
            }
            Some(Num::Float(f)) => {
                float_sum += f;
                all_int = false;
                n += 1;
            }
            None => {}
        }
    }
    if n == 0 {
        return None;
    }
    Some((int_sum, float_sum, all_int, n))
}

fn sum_of(values: &[Dynamic]) -> Dynamic {
    match fold(values) {
        Some((i, _f, true, _)) => Dynamic::from(i),
        Some((_, f, false, _)) => Dynamic::from(f),
        None => Dynamic::from(0i64),
    }
}

fn avg_of(values: &[Dynamic]) -> Dynamic {
    match fold(values) {
        Some((i, _f, true, n)) => Dynamic::from(i as f64 / n as f64),
        Some((_, f, false, n)) => Dynamic::from(f / n as f64),
        None => Dynamic::from(0i64),
    }
}

fn min_of(values: &[Dynamic]) -> Dynamic {
    match fold(values) {
        Some((_, _f, true, _)) => Dynamic::from(
            values
                .iter()
                .filter_map(|v| {
                    as_num(v).map(|n| match n {
                        Num::Int(i) => i,
                        Num::Float(x) => x as i64,
                    })
                })
                .min()
                .unwrap_or(0),
        ),
        Some((_, f, false, _)) => Dynamic::from(f_min(values).unwrap_or(f)),
        None => Dynamic::UNIT,
    }
}

fn max_of(values: &[Dynamic]) -> Dynamic {
    match fold(values) {
        Some((_, _f, true, _)) => Dynamic::from(
            values
                .iter()
                .filter_map(|v| {
                    as_num(v).map(|n| match n {
                        Num::Int(i) => i,
                        Num::Float(x) => x as i64,
                    })
                })
                .max()
                .unwrap_or(0),
        ),
        Some((_, f, false, _)) => Dynamic::from(f_max(values).unwrap_or(f)),
        None => Dynamic::UNIT,
    }
}

fn f_min(values: &[Dynamic]) -> Option<f64> {
    values
        .iter()
        .filter_map(|v| {
            as_num(v).map(|n| match n {
                Num::Int(i) => i as f64,
                Num::Float(x) => x,
            })
        })
        .fold(None, |acc: Option<f64>, x| {
            Some(acc.map_or(x, |a| a.min(x)))
        })
}

fn f_max(values: &[Dynamic]) -> Option<f64> {
    values
        .iter()
        .filter_map(|v| {
            as_num(v).map(|n| match n {
                Num::Int(i) => i as f64,
                Num::Float(x) => x,
            })
        })
        .fold(None, |acc: Option<f64>, x| {
            Some(acc.map_or(x, |a| a.max(x)))
        })
}

/// 行集合按字段聚合：`sum_by(rows, "field")`（rows = 映射数组；字段缺失/非数值忽略）。
fn sum_by(rows: &[Dynamic], field: &str) -> Dynamic {
    let mut picked: Vec<Dynamic> = Vec::new();
    for row in rows {
        if let Some(map) = row.clone().try_cast::<Map>() {
            if let Some(v) = map.get(field) {
                picked.push(v.clone());
            }
        }
    }
    sum_of(&picked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expression::RhaiExpressionEngine;
    use serde_json::json;
    use std::collections::HashMap;

    fn vars(data: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        data.iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn manifest_is_parseable_and_enumerable() {
        let names = names();
        assert!(names.contains(&"sum".to_string()), "{names:?}");
        assert!(names.contains(&"sum_by".to_string()), "{names:?}");
        assert!(
            manifest().iter().all(|f| !f.doc.is_empty()),
            "每条白名单函数 MUST 有 doc（供规约与门禁展示）"
        );
    }

    #[test]
    fn registered_functions_evaluate() {
        let eng = RhaiExpressionEngine::new();
        let ctx = vars(&[
            ("values", json!([1, 2, 3])),
            (
                "rows",
                json!([{ "additions": 3 }, { "additions": 4 }, { "other": 9 }]),
            ),
        ]);
        assert_eq!(eng.evaluate("sum(values)", &ctx).unwrap(), json!(6));
        assert_eq!(eng.evaluate("count(values)", &ctx).unwrap(), json!(3));
        assert_eq!(eng.evaluate("avg(values)", &ctx).unwrap(), json!(2.0));
        assert_eq!(eng.evaluate("min(values)", &ctx).unwrap(), json!(1));
        assert_eq!(eng.evaluate("max(values)", &ctx).unwrap(), json!(3));
        assert_eq!(
            eng.evaluate("sum_by(rows, \"additions\")", &ctx).unwrap(),
            json!(7)
        );
        assert_eq!(eng.evaluate("abs(-5.5)", &ctx).unwrap(), json!(5.5));
        assert_eq!(eng.evaluate("round(2.6)", &ctx).unwrap(), json!(3.0));
        assert_eq!(eng.evaluate("floor(2.6)", &ctx).unwrap(), json!(2.0));
        assert_eq!(eng.evaluate("ceil(2.1)", &ctx).unwrap(), json!(3.0));
        // 字符串能力沿用 Rhai **方法**形态（不入白名单，见 register 的 MUST NOT 说明）
        assert_eq!(
            eng.evaluate("\"AB\".to_lower()", &ctx).unwrap(),
            json!("ab")
        );
        assert_eq!(
            eng.evaluate("let s = \"  x \"; s.trim(); s", &ctx).unwrap(),
            json!("x")
        );
    }

    #[test]
    fn must_not_shadow_rhai_mutating_methods() {
        // 反例锚点（2026-09-22 实测）：曾把 `trim` 注册为自由函数 ⇒ 参与方法解析 ⇒
        // ① 常量契约失效（`code.trim()` 不再报错）；② 规范形态失真（`let t = x; t.trim(); t` 返回未 trim 值）。
        // 本测试与 `rhai.rs` 的 `test_ctx_values_are_immutable` / `test_canonical_migration_forms` 互为锚点。
        let eng = RhaiExpressionEngine::new();
        let ctx = vars(&[("code", json!("  x  ")), ("title", json!("  T  "))]);
        assert_eq!(
            eng.evaluate("let t = title; t.trim(); t", &ctx).unwrap(),
            json!("T")
        );
        assert!(
            eng.evaluate("code.trim(); code", &ctx).is_err(),
            "ctx 值为常量，原地变异 MUST 报错"
        );
        // 可持的不变量：`trim`（与 Rhai 变异方法同名者）MUST NOT 被本仓注册
        // （Rhai 自身提供自由 `trim` 形态，是否可用不由我们决定——"不注册"才是我们能守的约束）
        assert!(
            !is_registered("trim"),
            "MUST NOT 登记与 Rhai 变异方法同名的自由函数（会改变方法解析）"
        );
    }

    #[test]
    fn unregistered_free_function_is_not_available() {
        let eng = RhaiExpressionEngine::new();
        let err = eng.evaluate("median([1,2,3])", &vars(&[])).unwrap_err();
        assert!(err.contains("median"), "{err}");
        assert!(!is_registered("median"));
    }

    #[test]
    fn sandbox_remains_closed() {
        let eng = RhaiExpressionEngine::new();
        assert!(eng.evaluate("read_file(\"x\")", &vars(&[])).is_err());
        assert!(eng.evaluate("process()", &vars(&[])).is_err());
    }
}
