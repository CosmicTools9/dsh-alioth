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

    /// 匹配 `from_state → to_state` 并返回**被接受**的 transition（唯一匹配实现）
    ///
    /// 匹配优先级：`event + from + to` 全部命中优先；未命中则退化按 `from + to` 取首个（忽略 event）。
    /// 命中集合内逐个检查 guard：首个无 guard 或 guard 通过者被接受；全部被阻止 ⇒ Err。
    ///
    /// **唯一来源**：`validate_transition` 与 transition `action` 执行（`extension.rs`）MUST 共用本函数，
    /// MUST NOT 出现第二套匹配序（否则「校验通过的转换」与「执行 action 的转换」可能不是同一条）。
    pub fn find_transition<'a>(
        transitions: &'a [Transition],
        from_state: &str,
        to_state: &str,
        event: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<&'a Transition, String> {
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

            let Some(transition) = fallback.first() else {
                return Err(format!(
                    "不允许的状态转换：'{}' → '{}'",
                    from_state, to_state
                ));
            };

            // 用第一个 fallback 检查 guard
            if let Some(guard) = &transition.guard {
                return match Self::evaluate_guard(guard, variables) {
                    Ok(true) => Ok(transition),
                    Ok(false) => Err(format!(
                        "状态转换 '{}' → '{}' 被守卫条件阻止：{}",
                        from_state, to_state, guard
                    )),
                    Err(e) => Err(format!("守卫条件求值失败：{}", e)),
                };
            }
            return Ok(transition);
        }

        // 检查匹配 transition 的 guard
        for transition in &matching {
            if let Some(guard) = &transition.guard {
                match Self::evaluate_guard(guard, variables) {
                    Ok(true) => return Ok(transition),
                    Ok(false) => continue, // 尝试下一个匹配
                    Err(e) => return Err(format!("守卫条件求值失败：{}", e)),
                }
            } else {
                return Ok(transition);
            }
        }

        Err(format!(
            "所有匹配的转换均被守卫条件阻止（事件：{}，'{}' → '{}'）",
            event, from_state, to_state
        ))
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
        match Self::find_transition(transitions, from_state, to_state, event, variables) {
            Ok(_) => StateMachineResult::Passed,
            Err(reason) => StateMachineResult::Failed(reason),
        }
    }

    /// 获取状态机的所有合法状态名
    pub fn get_state_names(states: &[State]) -> Vec<String> {
        states.iter().map(|s| s.name.clone()).collect()
    }

    /// 评估守卫表达式（唯一引擎：Rhai 沙箱）
    ///
    /// **单一文法**（2026-09-22 裁定）：guard MUST 是 Rhai 表达式。历史实现先按「布尔字段引用」
    /// 短路（`variables.get(guard)` 命中即返回）——双文法通道：guard 恰为上下文变量名时绕过 Rhai
    /// 与受控函数白名单，且零测试覆盖。已删除该短路；裸变量名仍可作表达式求值（Rhai 变量读取），
    /// 故迁移面为零，但语义归一到唯一引擎。
    fn evaluate_guard(guard: &str, variables: &HashMap<String, Value>) -> Result<bool, String> {
        match crate::expression::RhaiExpressionEngine::new().evaluate(guard, variables) {
            Ok(value) => Ok(crate::expression::is_truthy(&value)),
            Err(e) => Err(format!("Guard expression error: {e}")),
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
            guard: Some("payment_status == \"paid\"".to_string()),
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

    /// `find_transition` = 匹配唯一来源：`validate_transition` 与 transition action 执行共用，
    /// 故「校验通过的转换」必须与「取出来的转换」是同一条（否则可能执行未通过校验那条的 action）。
    #[test]
    fn test_find_transition_prefers_event_match_and_returns_accepted_one() {
        let transitions = vec![
            // 同一 from→to 的两条：event 不同的那条带 action A，event 匹配的那条带 action B
            Transition::new("escalate", "Pending", "Confirmed")
                .with_action("grade = \"from_other_event\""),
            Transition::new("confirm", "Pending", "Confirmed")
                .with_action("grade = \"from_matched_event\""),
        ];
        let vars = ctx(&[]);

        let found = StateMachineEngine::find_transition(
            &transitions,
            "Pending",
            "Confirmed",
            "confirm",
            &vars,
        )
        .expect("event 命中应返回被接受的转换");
        assert_eq!(found.event, "confirm", "event 命中优先于仅 from→to 命中");
        assert_eq!(
            found.action.as_deref(),
            Some("grade = \"from_matched_event\"")
        );

        // event 不命中 ⇒ 退化按 from→to 取首个（既有口径）
        let fallback = StateMachineEngine::find_transition(
            &transitions,
            "Pending",
            "Confirmed",
            "nope",
            &vars,
        )
        .expect("退化匹配应返回首个");
        assert_eq!(fallback.event, "escalate", "退化口径 = from→to 首个");
    }

    #[test]
    fn test_find_transition_reports_reason_when_unmatched_or_guard_blocked() {
        let vars = ctx(&[("paid", json!(false))]);

        let unmatched = StateMachineEngine::find_transition(
            &make_transitions(),
            "Pending",
            "Delivered",
            "deliver",
            &vars,
        )
        .expect_err("无匹配转换应 Err");
        assert!(unmatched.contains("不允许的状态转换"), "{unmatched}");

        let guarded =
            vec![Transition::new("confirm", "Pending", "Confirmed").with_guard("paid == true")];
        let blocked =
            StateMachineEngine::find_transition(&guarded, "Pending", "Confirmed", "confirm", &vars)
                .expect_err("guard 未过应 Err");
        assert!(blocked.contains("被守卫条件阻止"), "{blocked}");

        // guard 通过 ⇒ Ok（同一条被接受）
        let vars = ctx(&[("paid", json!(true))]);
        assert!(StateMachineEngine::find_transition(
            &guarded,
            "Pending",
            "Confirmed",
            "confirm",
            &vars
        )
        .is_ok());
    }
}
