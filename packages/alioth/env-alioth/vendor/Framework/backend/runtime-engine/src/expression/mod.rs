//! 表达式引擎模块（平台唯一实现 = Rhai 沙箱）
//!
//! 见 change `unify-expression-dsl-on-rhai`：单一引擎、上下文键契约、写侧强校验。

pub mod analysis;
pub mod builtins;
pub mod plan_face;
pub mod rhai;

pub use analysis::{and_atoms, collect_calls, collect_variables, CmpOp, ComparisonAtom};
pub use plan_face::{plan_face_expressions, plan_face_violations};
pub use rhai::RhaiExpressionEngine;

/// 真值语义（条件求值统一口径）：Null / false / 0 / 空串 / 空数组 / 空对象 → false，其余 true
pub fn is_truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        serde_json::Value::String(s) => !s.is_empty(),
        serde_json::Value::Array(a) => !a.is_empty(),
        serde_json::Value::Object(o) => !o.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_is_truthy_semantics() {
        assert!(!is_truthy(&json!(null)));
        assert!(!is_truthy(&json!(false)));
        assert!(!is_truthy(&json!(0)));
        assert!(!is_truthy(&json!("")));
        assert!(!is_truthy(&json!([])));
        assert!(!is_truthy(&json!({})));
        assert!(is_truthy(&json!(true)));
        assert!(is_truthy(&json!(1)));
        assert!(is_truthy(&json!("x")));
        assert!(is_truthy(&json!([0])));
    }
}
