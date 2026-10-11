//! NGAC 策略操作词汇（change `consolidate-ngac-write-path`）。
//!
//! **单一真相源**：nl-assist 管线的操作校验（`admin_ngac_assist::validate_operations`）、
//! LLM 工具层的提案校验（`ngac_strategy_draft`）、执行体的分支集合
//! （`chat_sessions::adapters::strategy_action`）MUST 同源于本常量。
//!
//! 此前是三份独立声明且已分叉：`OP_WHITELIST` 8 项（含 `update_*`）vs
//! `NGAC_STRATEGY_OPS` 6 项 vs 执行体 match 6 臂 —— LLM 可以生成 `update_association`
//! 而执行体没有对应分支，确认后必然失败。
//!
//! 放置理由：这是**动作面**词汇（LLM 可提案什么、执行体能应用什么），与同 crate 的
//! 动作目录（参数 schema 的单一真相源）同层；Gateway 已依赖本 crate，无需新增依赖边。

/// 策略操作白名单（边级 + 指派级；**不含** UA/OA 节点结构与层级编辑）。
pub const POLICY_OPS: &[&str] = &[
    "create_association",
    "update_association",
    "delete_association",
    "create_prohibition",
    "update_prohibition",
    "delete_prohibition",
    "assign_ua",
    "remove_ua_assignment",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_ops_are_unique_and_non_empty() {
        assert!(!POLICY_OPS.is_empty());
        let mut seen = POLICY_OPS.to_vec();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "操作名 MUST 唯一");
    }

    /// 节点结构编辑（UA/OA 本体）不在策略操作面内。
    #[test]
    fn structural_edits_are_out_of_scope() {
        for structural in [
            "create_user_attribute",
            "delete_object_attribute",
            "update_policy_class",
        ] {
            assert!(
                !POLICY_OPS.contains(&structural),
                "{structural} 属结构编辑，MUST NOT 出现在策略操作面"
            );
        }
    }
}
