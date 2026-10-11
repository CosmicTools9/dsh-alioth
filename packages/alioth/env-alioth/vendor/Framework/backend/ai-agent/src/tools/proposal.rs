//! 提案归一（change `deepen-chat-action-seam` C1）。
//!
//! 「提案」= draft 族工具的产出（`status = needs_confirmation`）——它是**待用户确认的
//! 动作记录**，不是自由文本。此前四个生产方（`ngac_policy_draft` /
//! `ngac_strategy_draft` / `system_config_draft` / nl-assist 管线）各自即兴拼 JSON、
//! 形状互不相同（`ngac_strategy_draft` 的产出连 `id` / `label` / `params` 都没有），
//! 且工具产出在服务端从不被提取——模型必须把提案抄回正文的 ```` ```json ```` 块，
//! 于是「用户确认的是模型的抄写，而不是工具已经校验过的提案」。
//!
//! 本 module 给出唯一归一：工具产出 → 统一提案（前端 `structured.actions[]` 元素形状）。
//! 未成形者（仅给 `operations[]` 的策略草案）由**服务端**补齐 `id` / `label` / `params`
//! ——补齐者 CANNOT 是模型（MUST NOT 要求模型复述，MUST NOT 信任模型复述）。

use serde_json::{json, Value};

/// 提案族工具名：其产出的 `status = needs_confirmation` 即待确认提案。
pub const PROPOSAL_TOOLS: &[&str] = &[
    "ngac_policy_draft",
    "ngac_strategy_draft",
    "system_config_draft",
];

/// 工具产出 → 统一提案（`structured.actions[]` 的一个元素）。
///
/// 返回 `None` = 该产出不是提案（非提案族工具 / 无 `needs_confirmation` /
/// 缺 `action_type` / 未成形且无 `operations`）——调用方 MUST 回落正文解析。
pub fn proposal_from_tool_output(tool_name: &str, output: &Value) -> Option<Value> {
    if !PROPOSAL_TOOLS.contains(&tool_name) {
        return None;
    }
    let o = output.as_object()?;
    if o.get("status").and_then(Value::as_str) != Some("needs_confirmation") {
        return None;
    }
    let action_type = o.get("action_type").and_then(Value::as_str)?;

    // ① 已成形（id + label 齐备）：原样归一，仅显式补齐 kind / target_ids。
    if let (Some(id), Some(label)) = (
        o.get("id").and_then(Value::as_str),
        o.get("label").and_then(Value::as_str),
    ) {
        return Some(json!({
            "id": id.to_string(),
            "label": label,
            "kind": "execute",
            "action_type": action_type,
            "target_ids": o.get("target_ids").cloned().unwrap_or_else(|| json!([])),
            "params": o.get("params").cloned().unwrap_or_else(|| json!({})),
        }));
    }

    // ② 未成形（策略草案只给 operations）：服务端补齐 id / label / params。
    let ops = o.get("operations")?.clone();
    let count = ops.as_array().map_or(0, Vec::len);
    Some(json!({
        "id": "ngac:strategy-apply",
        "label": format!("应用 {count} 项策略操作"),
        "kind": "execute",
        "action_type": action_type,
        "target_ids": [],
        "params": { "operations": ops },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formed_proposal_is_normalised_verbatim() {
        let out = json!({
            "id": "syscfg:apply",
            "label": "应用 1 项系统设置变更",
            "kind": "execute",
            "action_type": "system_config_apply",
            "target_ids": [],
            "params": { "operations": [{ "op": "create_config" }] },
            "status": "needs_confirmation",
            "confirm_url_hint": "/api/chat-sessions/1/execute-action",
        });
        let p = proposal_from_tool_output("system_config_draft", &out).expect("应为提案");
        assert_eq!(p["id"], "syscfg:apply");
        assert_eq!(p["kind"], "execute");
        assert_eq!(p["action_type"], "system_config_apply");
        assert_eq!(p["params"]["operations"][0]["op"], "create_config");
    }

    /// 未成形者由服务端补齐（不是模型）：策略草案只有 operations。
    #[test]
    fn shapeless_draft_is_completed_server_side() {
        let out = json!({
            "status": "needs_confirmation",
            "action_type": "ngac_strategy_apply",
            "confirm_url_hint": "/api/chat-sessions/1/execute-action",
            "message": "m",
            "operations": [{ "op": "create_association" }, { "op": "assign_ua" }],
        });
        let p = proposal_from_tool_output("ngac_strategy_draft", &out).expect("应为提案");
        assert_eq!(p["kind"], "execute");
        assert_eq!(p["action_type"], "ngac_strategy_apply");
        assert_eq!(p["label"], "应用 2 项策略操作");
        assert_eq!(p["params"]["operations"].as_array().unwrap().len(), 2);
        assert_eq!(p["target_ids"], json!([]));
    }

    #[test]
    fn non_proposal_outputs_are_rejected() {
        // 非提案族工具（action_type 用目录内真实 id：本用例的判据是工具名，不是动作名）
        assert!(proposal_from_tool_output(
            "query_sql",
            &json!({ "status": "needs_confirmation", "action_type": "system_config_apply" })
        )
        .is_none());
        // 提案族但未标记 needs_confirmation
        assert!(proposal_from_tool_output(
            "system_config_draft",
            &json!({ "status": "ok", "id": "a", "label": "b", "action_type": "system_config_apply" })
        )
        .is_none());
        // 提案族但缺 action_type
        assert!(proposal_from_tool_output(
            "system_config_draft",
            &json!({ "status": "needs_confirmation", "id": "a", "label": "b" })
        )
        .is_none());
        // 未成形且无 operations
        assert!(proposal_from_tool_output(
            "ngac_strategy_draft",
            &json!({ "status": "needs_confirmation", "action_type": "ngac_strategy_apply" })
        )
        .is_none());
    }
}
