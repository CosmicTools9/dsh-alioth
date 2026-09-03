//! Rhai 公式引擎（复杂公式通道；统一引擎审计结论：简单流程条件走
//! ConstraintExpr DSL（evaluator.rs），复杂公式/规则脚本走 Rhai——纯 Rust
//! 嵌入式脚本，默认安全模式，零 C 依赖）。
//!
//! 路由约定：`zc_id_formula` 复杂公式的 `expression` 存 Rhai 脚本；调用方按
//! 公式上下文（如 context jsonb `{"engine": "rhai"}`）或公式能力需求选择本引擎。
//!
//! 沙箱：Rhai 默认安全模式（无文件/网络/进程/模块加载），本引擎不注册任何
//! IO/系统函数，并设表达式深度/调用层/总操作数上限（防失控脚本 DoS）。

use rhai::Engine;
use serde_json::Value;
use std::collections::HashMap;

/// Rhai 公式求值器（沙箱：安全模式、无 IO 函数注册、深度/操作数上限）
pub struct RhaiExpressionEngine {
    engine: Engine,
}

impl Default for RhaiExpressionEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RhaiExpressionEngine {
    /// 构造沙箱引擎：安全模式 + 表达式深度（100/200）与总操作数（100 万）上限
    pub fn new() -> Self {
        let mut engine = Engine::new();
        engine.set_max_expr_depths(100, 200);
        engine.set_max_operations(1_000_000);
        engine.set_max_call_levels(32);
        Self { engine }
    }

    /// 求值 Rhai 表达式/脚本 → JSON Value。
    /// 变量映射：serde_json Value → rhai Dynamic（对象 → map、数组 → array、数值按 JSON）。
    pub fn evaluate(
        &self,
        script: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        let mut scope = rhai::Scope::new();
        for (k, v) in variables {
            let dyn_v = rhai::serde::to_dynamic(v)
                .map_err(|e| format!("variable '{}' conversion: {e}", k))?;
            scope.push_constant(k, dyn_v);
        }
        let out: rhai::Dynamic = self
            .engine
            .eval_with_scope::<rhai::Dynamic>(&mut scope, script)
            .map_err(|e| format!("rhai eval: {e}"))?;
        rhai::serde::from_dynamic(&out).map_err(|e| format!("result conversion: {e}"))
    }

    /// 布尔求值（公式条件语义：顶层非 bool 视同 false）
    pub fn evaluate_bool(
        &self,
        script: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<bool, String> {
        Ok(self.evaluate(script, variables)?.as_bool().unwrap_or(false))
    }

    /// 语法校验（compile-only，不执行——变量未定义在执行期才报错，
    /// 编译通过即语法合法；AI 生成公式的强校验通道）
    pub fn validate(&self, script: &str) -> Result<(), String> {
        self.engine
            .compile(script)
            .map(|_| ())
            .map_err(|e| format!("rhai syntax: {e}"))
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
        // Rhai 图灵完备：if-else / 循环（复杂公式通道能力）
        let eng = RhaiExpressionEngine::new();
        let ctx = vars(&[("amount", json!(5000))]);
        // rhai 字符串字面量须双引号（单引号为 char）
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
}
