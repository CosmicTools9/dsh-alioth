//! Plan 面（`flow-plan.json`）表达式承载键的**唯一键集实现**。
//!
//! 键集与组合期（`composer.rs::validate_extension_expressions`）消费面一致：
//! `constraints[].expression` / `business_rules[].condition` / `business_rules[].action`。
//! `core_constraints[]`（自然语言描述）与 `computations[].formula`（SQL 承载）**不属**表达式面
//! ——实测语义面，见 `docs/specs/ALIOTH_ONTOLOGY_SPEC.md` §表达式唯一引擎。
//!
//! 消费方（禁第二份实现）：方言测试 `tests/extension_yaml_dialect.rs::plan_face_expressions_are_valid_rhai`
//! 与提案写径（`Meta/backend/app-agent/src/dialog_tools/patch_assets.rs`）。

use serde_json::Value;

/// 抽取 plan 面表达式：`(<键路径>, <表达式>)`，键路径形如 `constraints[0].expression`。
pub fn plan_face_expressions(plan: &Value) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let arr = |key: &str| -> Vec<Value> {
        plan.get(key)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    };
    for (i, c) in arr("constraints").iter().enumerate() {
        if let Some(e) = c.get("expression").and_then(|v| v.as_str()) {
            out.push((format!("constraints[{i}].expression"), e.to_string()));
        }
    }
    for (i, r) in arr("business_rules").iter().enumerate() {
        if let Some(c) = r.get("condition").and_then(|v| v.as_str()) {
            out.push((format!("business_rules[{i}].condition"), c.to_string()));
        }
        if let Some(a) = r.get("action").and_then(|v| v.as_str()) {
            out.push((format!("business_rules[{i}].action"), a.to_string()));
        }
    }
    out
}

/// 校验 plan 面表达式合法性（语法 + 受控函数白名单），返回 `(<键路径>, <表达式与错误明细>)`。
pub fn plan_face_violations(plan: &Value) -> Vec<(String, String)> {
    let engine = crate::expression::RhaiExpressionEngine::new();
    plan_face_expressions(plan)
        .into_iter()
        .filter(|(_, expr)| !expr.trim().is_empty())
        .filter_map(|(label, expr)| {
            engine
                .validate_functions(&expr)
                .err()
                .map(|e| (label, format!("{expr}\n      {e}")))
        })
        .collect()
}
