//! 表达式引擎（平台唯一实现）。
//!
//! **单一引擎**（2026-09-22 用户裁决，change `unify-expression-dsl-on-rhai`）：
//! 平台所有表达式（流程条件/守卫、扩展 YAML、审批 loop 公式、模型约束表达式、
//! 计价规则脚本）MUST 经本引擎求值；MUST NOT 并存第二套文法。
//!
//! 上下文键契约（D2）：ctx 同时以两种形态注入 ——
//! - 可作标识符的键（`[A-Za-z_][A-Za-z0-9_]*`）逐名注入常量，脚本以**裸名**引用
//!   （`amount > 100`）；
//! - 整体映射注入常量 `ctx`，含连字符/点号的键（模型物理列 `act-group`、引用别名
//!   `cnt-status`）MUST 以**索引语法**引用 —— `ctx["act-group"]`、
//!   `ctx["_refs"]["cnt-status"]["code"]`。
//!
//! 注入期 MUST NOT 改写键名（禁止 `-` → `_` 归一化）。
//!
//! 沙箱：Rhai 默认安全模式（无文件/网络/进程/模块加载），本引擎不注册任何
//! IO/系统函数，并设表达式深度/调用层/总操作数上限（防失控脚本 DoS）。

use rhai::{Dynamic, Engine, Map, Scope};
use serde_json::Value;
use std::collections::HashMap;

/// Rhai 表达式求值器（沙箱：安全模式、无 IO 函数注册、深度/操作数上限）
pub struct RhaiExpressionEngine {
    engine: Engine,
}

/// 键是否为可直接引用的 Rhai 标识符（D2：否则走 `ctx["k"]` 索引语法）
fn is_bare_ident(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl Default for RhaiExpressionEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// 沙箱引擎构造（表达式深度 100/200、总操作数 100 万、调用层 32；无 IO 函数注册）。
/// `strict_variables` = 强校验模式（未知标识符在**编译期**即 Err）。
fn build_engine(strict_variables: bool) -> Engine {
    let mut engine = Engine::new();
    engine.set_max_expr_depths(100, 200);
    engine.set_max_operations(1_000_000);
    engine.set_max_call_levels(32);
    engine.set_strict_variables(strict_variables);
    // 受控内建函数（唯一白名单，见 `builtins.json`；不放开沙箱）
    crate::expression::builtins::register(&mut engine);
    engine
}

impl RhaiExpressionEngine {
    /// 构造沙箱引擎（求值用；强校验见 [`Self::validate`]）
    pub fn new() -> Self {
        Self {
            engine: build_engine(false),
        }
    }

    /// 上下文注入（D2 契约）：逐键常量（仅可作标识符者）+ 整体 `ctx` 映射。
    pub fn scope_of(variables: &HashMap<String, Value>) -> Result<Scope<'static>, String> {
        let mut map = Map::new();
        let mut scope = Scope::new();
        for (k, v) in variables {
            let dyn_v = rhai::serde::to_dynamic(v)
                .map_err(|e| format!("variable '{}' conversion: {e}", k))?;
            map.insert(k.as_str().into(), dyn_v.clone());
            if is_bare_ident(k) {
                scope.push_constant(k.as_str(), dyn_v);
            }
        }
        scope.push_constant("ctx", Dynamic::from_map(map));
        Ok(scope)
    }

    /// 求值 Rhai 表达式/脚本 → JSON Value。
    /// 变量映射：serde_json Value → rhai Dynamic（对象 → map、数组 → array、数值按 JSON）。
    pub fn evaluate(
        &self,
        script: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        let mut scope = Self::scope_of(variables)?;
        let out: Dynamic = self
            .engine
            .eval_with_scope::<Dynamic>(&mut scope, script)
            .map_err(|e| format!("rhai eval: {e}"))?;
        rhai::serde::from_dynamic(&out).map_err(|e| format!("result conversion: {e}"))
    }

    /// 布尔求值（条件语义：顶层非 bool 视同 false）
    pub fn evaluate_bool(
        &self,
        script: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<bool, String> {
        Ok(self.evaluate(script, variables)?.as_bool().unwrap_or(false))
    }

    /// 强校验（D3，fail-closed）：严格变量模式 + 已知键作用域 ⇒
    /// 语法错与**未知标识符**均 Err（写入侧 MUST 以此阻断落库）。
    /// `known_keys` = 该表达式可见的上下文键（模型列/引用别名 + 引擎注入键）。
    pub fn validate(&self, script: &str, known_keys: &[String]) -> Result<(), String> {
        let mut scope = Scope::new();
        for k in known_keys {
            if is_bare_ident(k) {
                scope.push_constant(k.as_str(), ());
            }
        }
        scope.push_constant("ctx", Map::new());
        let engine = build_engine(true);
        engine
            .compile_with_scope(&scope, script)
            .map(|_| ())
            .map_err(|e| format!("rhai syntax/unknown-identifier: {e}"))
    }

    /// **纯语法**校验（compile-only，仅内部/测试与「语法构造探针」用）。
    ///
    /// 写径/生成点**MUST NOT** 用本入口：无法获知上下文键集时应退化为 [`Self::validate_functions`]
    /// （语法 + 白名单），能获知键集时用 [`Self::validate_all`]——仅语法层会放行未注册自由函数
    /// （Rhai 编译期不报未注册函数 ⇒ 运行期才 `Function not found`）。
    pub fn validate_syntax(&self, script: &str) -> Result<(), String> {
        self.engine
            .compile(script)
            .map(|_| ())
            .map_err(|e| format!("rhai syntax: {e}"))
    }

    /// **上下文无关全量校验**（标识符集未知时的正确退化）：语法 + **自由函数 ∈ 白名单**。
    ///
    /// 用途：写径/生成点**无法获知上下文键集**时（如发布期无法解析流程绑定的实体叶表）——
    /// 那时 MUST 用本入口，**不得**退回 [`Self::validate_syntax`]（仅语法 ⇒ 放行未注册函数，
    /// 即 R8 盲区：Rhai 对未注册函数不在编译期报错，只在求值期 `Function not found`）。
    pub fn validate_functions(&self, script: &str) -> Result<(), String> {
        self.validate_syntax(script)?;
        Self::check_free_functions(script)
    }

    /// 自由函数白名单层（[`Self::validate_all`] 与 [`Self::validate_functions`] 共用同一判定）。
    fn check_free_functions(script: &str) -> Result<(), String> {
        let unknown: Vec<String> = crate::expression::analysis::collect_calls(script)
            .map_err(|e| format!("rhai parse: {e}"))?
            .into_iter()
            .filter(|name| !crate::expression::builtins::is_registered(name))
            .collect();
        if unknown.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "未登记的自由函数（白名单见 runtime-engine/expression/builtins.json）: {}",
                unknown.join(", ")
            ))
        }
    }

    /// **全量校验**（写径 SHOULD 用本入口）：语法 + 标识符 ⊆ `known_keys` + **自由函数 ∈ 白名单**。
    ///
    /// 三层缺一不可：Rhai 对未注册函数**不在编译期报错**（仅求值期 `Function not found`），
    /// 故标识符层校验过后仍可能运行期失败——白名单层即为此补位（清单见 `builtins.json`）。
    pub fn validate_all(&self, script: &str, known_keys: &[String]) -> Result<(), String> {
        self.validate(script, known_keys)?;
        Self::check_free_functions(script)
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
        let eng = RhaiExpressionEngine::new();
        let ctx = vars(&[("a", json!(10)), ("b", json!(3))]);
        assert_eq!(eng.evaluate("a + b", &ctx).unwrap(), json!(13));
        assert_eq!(eng.evaluate("a * b", &ctx).unwrap(), json!(30));
        assert_eq!(eng.evaluate("a - b", &ctx).unwrap(), json!(7));
        // rhai INT 除法为整数除法（10 / 3 = 3）
        assert_eq!(eng.evaluate("a / b", &ctx).unwrap(), json!(3));
        assert_eq!(eng.evaluate("a / 2.0", &ctx).unwrap(), json!(5.0));
    }

    #[test]
    fn test_logic_control_flow() {
        // Rhai 图灵完备：if-else / 循环
        let eng = RhaiExpressionEngine::new();
        let ctx = vars(&[("amount", json!(5000))]);
        assert_eq!(
            eng.evaluate("if amount > 1000 { \"big\" } else { \"small\" }", &ctx)
                .unwrap(),
            json!("big")
        );
        assert_eq!(
            eng.evaluate("let total = 0; for i in 0..5 { total += i; } total", &ctx)
                .unwrap(),
            json!(10)
        );
    }

    #[test]
    fn test_object_and_array() {
        let eng = RhaiExpressionEngine::new();
        let ctx = vars(&[("order", json!({"qty": 3, "price": 25.0}))]);
        assert_eq!(
            eng.evaluate("order.qty * order.price", &ctx).unwrap(),
            json!(75.0)
        );
        let ctx2 = vars(&[("items", json!([1, 2, 3]))]);
        assert_eq!(eng.evaluate("items.len()", &ctx2).unwrap(), json!(3));
    }

    #[test]
    fn validate_all_enforces_free_function_whitelist() {
        let eng = RhaiExpressionEngine::new();
        let known = vec!["rows".to_string()];
        // 白名单内 ⇒ 通过（标识符亦在已知键内）
        eng.validate_all("sum_by(rows, \"additions\") > 0", &known)
            .expect("sum_by 在受控白名单内");
        // 白名单外 ⇒ Err：Rhai 编译期**不**报未注册函数（仅求值期 Function not found），
        // 故白名单层是写入侧唯一拦截点——本断言即钉住该层（去掉即回归盲区）
        let err = eng
            .validate_all("median([1, 2, 3]) > 0", &known)
            .unwrap_err();
        assert!(err.contains("median"), "错误应指名函数：{err}");
    }

    #[test]
    fn validate_functions_is_context_free_but_enforces_whitelist() {
        let eng = RhaiExpressionEngine::new();
        // 上下文无关：标识符不判（那是 `validate` / `validate_all` 的职责）
        eng.validate_functions("ghost > 100")
            .expect("上下文无关入口 MUST NOT 判标识符");
        // 但未登记自由函数 MUST 拦下（Rhai 编译期不报未注册函数 ⇒ 唯一拦截点）
        let err = eng.validate_functions("median([1, 2, 3]) > 0").unwrap_err();
        assert!(err.contains("median"), "{err}");
        // 已登记函数放行；语法错仍拦下
        eng.validate_functions("sum([1, 2, 3]) > 0")
            .expect("sum 已登记");
        assert!(eng.validate_functions("ghost > ((").is_err());
    }

    #[test]
    fn test_bool_semantics() {
        let eng = RhaiExpressionEngine::new();
        let ctx = vars(&[("a", json!(1))]);
        assert!(eng.evaluate_bool("a > 0", &ctx).unwrap());
        assert!(!eng.evaluate_bool("a > 5", &ctx).unwrap());
        assert!(!eng.evaluate_bool("a", &ctx).unwrap());
    }

    #[test]
    fn test_sandbox_no_io() {
        // 安全模式 + 未注册 IO 函数：文件/进程访问必须失败
        let eng = RhaiExpressionEngine::new();
        let empty = vars(&[]);
        assert!(eng.evaluate("let f = read_file('x'); 1", &empty).is_err());
        assert!(eng.evaluate("let s = process(); 1", &empty).is_err());
    }

    // ── D2 上下文键契约 ──

    #[test]
    fn test_hyphen_key_requires_index_syntax() {
        let eng = RhaiExpressionEngine::new();
        let ctx = vars(&[
            ("act-group", json!(1)),
            ("_refs", json!({ "cnt-status": { "code": "CNT-1" } })),
        ]);
        // 含连字符的键 MUST 经索引语法（裸名在 Rhai 中解析为减法 ⇒ 严格模式下未知标识符）
        assert_eq!(
            eng.evaluate("ctx[\"act-group\"] == 1", &ctx).unwrap(),
            json!(true)
        );
        assert_eq!(
            eng.evaluate("ctx[\"_refs\"][\"cnt-status\"][\"code\"]", &ctx)
                .unwrap(),
            json!("CNT-1")
        );
        assert!(eng.evaluate("act-group == 1", &ctx).is_err());
    }

    #[test]
    fn test_plain_key_bare_and_index_both_work() {
        let eng = RhaiExpressionEngine::new();
        let ctx = vars(&[
            ("amount", json!(500)),
            ("_refs", json!({"bill": {"code": "B-1"}})),
        ]);
        assert_eq!(eng.evaluate("amount > 100", &ctx).unwrap(), json!(true));
        assert_eq!(
            eng.evaluate("ctx[\"amount\"] > 100", &ctx).unwrap(),
            json!(true)
        );
        assert_eq!(
            eng.evaluate("_refs[\"bill\"][\"code\"]", &ctx).unwrap(),
            json!("B-1")
        );
    }

    // ── D3 强校验（fail-closed）──

    #[test]
    fn test_validate_rejects_unknown_identifier() {
        let eng = RhaiExpressionEngine::new();
        let known = vec!["amount".to_string(), "cursor".to_string()];
        assert!(eng.validate("amount > 100 && cursor < 3", &known).is_ok());
        assert!(eng.validate("amount > 100 && ghost > 0", &known).is_err());
        // 语法错在两级入口都拦下（严格 validate / 仅语法 validate_syntax）
        assert!(eng.validate("amount > ((", &known).is_err());
        assert!(eng.validate_syntax("amount > ((").is_err());
        // 仅语法入口不判标识符（写径无法获知键集时的合法退化，见规约写入侧校验）
        assert!(eng.validate_syntax("ghost > 100").is_ok());
        // 含连字符键经 ctx 索引不受 known_keys 影响（ctx 恒可见）
        assert!(eng.validate("ctx[\"act-group\"] == 1", &known).is_ok());
    }

    #[test]
    fn test_migrated_authored_idioms() {
        // 扩展 YAML 迁移后的写法（design D6）：方法调用/if-else/in/长度。
        // 注意 ctx 值为**常量**（D2）：原地变异方法（`trim` 等）MUST 作用于本地副本。
        let eng = RhaiExpressionEngine::new();
        let ctx = vars(&[
            ("code", json!("  X-1  ")),
            ("comments", Value::Null),
            ("status", json!("Reviewed")),
            ("orderStatus", json!("已支付")),
        ]);
        assert_eq!(
            eng.evaluate("let s = code; s.trim(); s.len() > 0", &ctx)
                .unwrap(),
            json!(true)
        );
        assert_eq!(
            eng.evaluate(
                "if comments == () { \"[review-started]\" } else { comments + \" [review-started]\" }",
                &ctx
            )
            .unwrap(),
            json!("[review-started]")
        );
        assert_eq!(
            eng.evaluate("orderStatus in [\"待支付\", \"已支付\"]", &ctx)
                .unwrap(),
            json!(true)
        );
        assert_eq!(
            eng.evaluate("status == \"Reviewed\"", &ctx).unwrap(),
            json!(true)
        );
    }

    #[test]
    fn test_canonical_migration_forms() {
        // 扩展 YAML 迁移的规范形态（design D6）：块表达式 / 动作 RHS 多语句 / 布尔与或
        let eng = RhaiExpressionEngine::new();
        let ctx = vars(&[
            ("code", json!("  X-1 ")),
            ("title", json!("  T  ")),
            ("comments", Value::Null),
            ("quantity", json!(5)),
            ("unitPrice", json!(20.0)),
            ("_f_", json!("personal")),
        ]);
        // 1. 块表达式（清理后判空）
        assert_eq!(
            eng.evaluate(
                "code != () && { let s = code; s.trim(); s.len() > 0 }",
                &ctx
            )
            .unwrap(),
            json!(true)
        );
        // 2. 动作 RHS 多语句（结果 = 最后表达式）
        assert_eq!(
            eng.evaluate("let t = title; t.trim(); t", &ctx).unwrap(),
            json!("T")
        );
        // 3. 布尔与或 + 数值乘算（迁移自 `and`/`or`）
        assert_eq!(
            eng.evaluate("quantity > 0 && unitPrice >= 0", &ctx)
                .unwrap(),
            json!(true)
        );
        assert_eq!(
            eng.evaluate("comments == () || _f_ == \"company\"", &ctx)
                .unwrap(),
            json!(true)
        );
        assert_eq!(
            eng.evaluate("quantity * unitPrice", &ctx).unwrap(),
            json!(100.0)
        );
    }

    #[test]
    fn test_ctx_values_are_immutable() {
        // D2 契约：ctx 以常量注入 ⇒ 脚本 MUST NOT 原地修改上下文（无副作用保证）
        let eng = RhaiExpressionEngine::new();
        let ctx = vars(&[("code", json!("x "))]);
        assert!(eng.evaluate("code.trim(); code", &ctx).is_err());
        // 本地副本可变
        assert_eq!(
            eng.evaluate("let s = code; s.trim(); s", &ctx).unwrap(),
            json!("x")
        );
    }
}
