//! 状态机引擎
//!
//! 验证实体生命周期中的状态转换合法性。
//! 支持守卫条件（guard expression）求值和初始状态验证。

use runtime_contract::behavior::{State, Transition};
use serde_json::Value;
use std::collections::HashMap;

/// 状态机验证结果
#[derive(Debug, Clone)]
pub enum StateMachineResult {
    /// 验证通过
    Passed,
    /// 验证失败，附错误描述
    Failed(String),
}

/// 状态机引擎
pub struct StateMachineEngine;

impl StateMachineEngine {
    /// 验证初始状态
    ///
    /// 实体创建时检查设置的初始状态是否在状态列表中。
    pub fn validate_initial_state(states: &[State], initial_state: &str) -> StateMachineResult {
        if states.iter().any(|s| s.name == initial_state) {
            StateMachineResult::Passed
        } else {
            StateMachineResult::Failed(format!("'{}' 不在合法状态列表中", initial_state))
        }
    }

    /// 验证状态转换
    ///
    /// 检查 `from_state → to_state` 是否被任何已定义的 transition 覆盖。
    /// 若有 guard 条件，同时评估 guard 表达式。
    pub fn validate_transition(
        transitions: &[Transition],
        from_state: &str,
        to_state: &str,
        event: &str,
        variables: &HashMap<String, Value>,
    ) -> StateMachineResult {
        // 查找匹配的 transition
        let matching: Vec<&Transition> = transitions
            .iter()
            .filter(|t| {
                t.event == event && t.from.iter().any(|f| f == from_state) && t.to == to_state
            })
            .collect();

        if matching.is_empty() {
            // 尝试仅按 from→to 匹配（忽略 event）
            let fallback: Vec<&Transition> = transitions
                .iter()
                .filter(|t| t.from.iter().any(|f| f == from_state) && t.to == to_state)
                .collect();

            if fallback.is_empty() {
                return StateMachineResult::Failed(format!(
                    "不允许的状态转换：'{}' → '{}'",
                    from_state, to_state
                ));
            }

            // 用第一个 fallback 检查 guard
            if let Some(ref guard) = fallback[0].guard {
                return match Self::evaluate_guard(guard, variables) {
                    Ok(true) => StateMachineResult::Passed,
                    Ok(false) => StateMachineResult::Failed(format!(
                        "状态转换 '{}' → '{}' 被守卫条件阻止：{}",
                        from_state, to_state, guard
                    )),
                    Err(e) => StateMachineResult::Failed(format!("守卫条件求值失败：{}", e)),
                };
            }
            return StateMachineResult::Passed;
        }

        // 检查匹配 transition 的 guard
        for transition in &matching {
            if let Some(ref guard) = transition.guard {
                match Self::evaluate_guard(guard, variables) {
                    Ok(true) => return StateMachineResult::Passed,
                    Ok(false) => continue, // 尝试下一个匹配
                    Err(e) => {
                        return StateMachineResult::Failed(format!("守卫条件求值失败：{}", e))
                    }
                }
            } else {
                return StateMachineResult::Passed;
            }
        }

        StateMachineResult::Failed(format!(
            "所有匹配的转换均被守卫条件阻止（事件：{}，'{}' → '{}'）",
            event, from_state, to_state
        ))
    }

    /// 获取状态机的所有合法状态名
    pub fn get_state_names(states: &[State]) -> Vec<String> {
        states.iter().map(|s| s.name.clone()).collect()
    }

    /// 评估守卫表达式
    fn evaluate_guard(guard: &str, variables: &HashMap<String, Value>) -> Result<bool, String> {
        // guard 可以是简单的布尔字段引用，也可以是完整表达式
        // 1. 尝试作为字段引用解析
        if let Some(value) = variables.get(guard) {
            return Ok(crate::expression::evaluator::ExpressionEvaluator::is_truthy(value));
        }

        // 2. 尝试作为表达式解析
        match crate::expression::ExpressionEvaluator::evaluate_expression(guard, variables) {
            Ok(value) => Ok(crate::expression::evaluator::ExpressionEvaluator::is_truthy(&value)),
            Err(e) => Err(format!("Guard expression error: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx(vars: &[(&str, Value)]) -> HashMap<String, Value> {
        vars.iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn make_states() -> Vec<State> {
        vec![
            State::new("Pending"),
            State::new("Confirmed"),
            State::new("Shipped"),
            State::new("Delivered"),
            State::new("Cancelled"),
        ]
    }

    fn make_transitions() -> Vec<Transition> {
        vec![
            Transition::new("confirm", "Pending", "Confirmed"),
            Transition::new("ship", "Confirmed", "Shipped"),
            Transition::new("deliver", "Shipped", "Delivered"),
            Transition {
                event: "cancel".to_string(),
                from: vec!["Pending".to_string(), "Confirmed".to_string()],
                to: "Cancelled".to_string(),
                guard: None,
                action: None,
                is_default: false,
            },
        ]
    }

    #[test]
    fn test_validate_initial_state() {
        let states = make_states();
        assert!(matches!(
            StateMachineEngine::validate_initial_state(&states, "Pending"),
            StateMachineResult::Passed
        ));
        assert!(matches!(
            StateMachineEngine::validate_initial_state(&states, "Invalid"),
            StateMachineResult::Failed(_)
        ));
    }

    #[test]
    fn test_validate_valid_transition() {
        let transitions = make_transitions();
        let vars = ctx(&[]);

        assert!(matches!(
            StateMachineEngine::validate_transition(
                &transitions,
                "Pending",
                "Confirmed",
                "confirm",
                &vars
            ),
            StateMachineResult::Passed
        ));
    }

    #[test]
    fn test_validate_invalid_transition() {
        let transitions = make_transitions();
        let vars = ctx(&[]);

        let result = StateMachineEngine::validate_transition(
            &transitions,
            "Pending",
            "Delivered",
            "deliver",
            &vars,
        );
        assert!(matches!(result, StateMachineResult::Failed(_)));
        if let StateMachineResult::Failed(msg) = result {
            assert!(msg.contains("不允许的状态转换"));
        }
    }

    #[test]
    fn test_validate_multi_from_transition() {
        let transitions = make_transitions();
        let vars = ctx(&[]);

        // Pending → Cancelled 允许
        assert!(matches!(
            StateMachineEngine::validate_transition(
                &transitions,
                "Pending",
                "Cancelled",
                "cancel",
                &vars
            ),
            StateMachineResult::Passed
        ));

        // Confirmed → Cancelled 允许
        assert!(matches!(
            StateMachineEngine::validate_transition(
                &transitions,
                "Confirmed",
                "Cancelled",
                "cancel",
                &vars
            ),
            StateMachineResult::Passed
        ));
    }

    #[test]
    fn test_guard_condition() {
        let transitions = vec![Transition {
            event: "confirm".to_string(),
            from: vec!["Pending".to_string()],
            to: "Confirmed".to_string(),
            guard: Some("payment_status == 'paid'".to_string()),
            action: None,
            is_default: false,
        }];

        // Guard 通过
        let vars = ctx(&[("payment_status", json!("paid"))]);
        assert!(matches!(
            StateMachineEngine::validate_transition(
                &transitions,
                "Pending",
                "Confirmed",
                "confirm",
                &vars
            ),
            StateMachineResult::Passed
        ));

        // Guard 不通过
        let vars = ctx(&[("payment_status", json!("unpaid"))]);
        let result = StateMachineEngine::validate_transition(
            &transitions,
            "Pending",
            "Confirmed",
            "confirm",
            &vars,
        );
        assert!(matches!(result, StateMachineResult::Failed(_)));
    }
}
