//! 业务动作目录（单一真相源，change `add-action-catalogue`）。
//!
//! 此前「业务动作」清单在平台上存在三份手工同步副本且已分歧：
//! ① `execute_action` 工具面向 LLM 的 `enum`（4 项，`executor.rs`）；
//! ② Gateway `run_action` 的 7 臂字符串 `match`（`tool_bridge.rs`）；
//! ③ `confirmation_level` 的 3 臂 + `_ => Explicit` 通配（`tool_bridge.rs`）。
//!
//! 本模块把三者收拢为**唯一目录**：每动作一条 [`ActionSpec`]，含
//! `id` / 描述 / 参数 schema / 确认级 / 目标 / 是否暴露给 LLM。
//!
//! - LLM 可见枚举：`ExecuteActionTool::definition()` 由 [`ActionCatalogue::llm_exposed_ids`]
//!   生成（`expose_to_llm` 过滤）——NGAC 规范/策略、系统设置三个动作**只经提案
//!   `actions[]` 可达**（`ngac_*_draft` / `system_config_draft` 工具产出），
//!   MUST NOT 经 `execute_action` 的 `action_type` 枚举暴露给模型（否则改变行为）。
//! - Gateway 派发：`run_action` 按 `id` 查目录取 [`ActionDispatch`] 后穷尽 match。
//! - 确认级：`ActionHandler::confirmation_level` 默认实现由目录派生；未知 id
//!   fail-safe 归 `Explicit`（宁可多确认不裸执行）——无手写通配。

use crate::agents::ConfirmationLevel;
use serde_json::{json, Value};

/// 动作派发标识：Gateway 侧按此枚举选执行体。新增动作时在此加变体，
/// Gateway 的穷尽 match 会在编译期强制同步（MUST NOT 退化为字符串手写清单）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionDispatch {
    /// 批量审批 / 驳回（approvals 服务层）。
    BatchApprove,
    /// 发送系统通知（system_push 落库服务）。
    SendNotification,
    /// 通用状态推进（trigger_crud 通用 UPDATE + 触发器 NGAC）。
    StatusTransition,
    /// 生成单据草稿（assistant 消息落库）。
    GenerateDocument,
    /// NGAC 权限规范写入（SSO org-policy 服务层）。
    NgacOrgPolicyApply,
    /// NGAC 策略操作（association/prohibition/UA 指派）。
    NgacStrategyApply,
    /// 系统设置变更（凭证加密 + system_config.* 审计）。
    SystemConfigApply,
}

/// 动作作用目标（描述性元数据：供文档与一致性判据，不参与执行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionTarget {
    Approval,
    Notification,
    EntityRecord,
    Document,
    NgacPolicy,
    NgacStrategy,
    SystemConfig,
}

/// 单个动作的目录条目。
pub struct ActionSpec {
    /// 动作 id（线上协议字段 `action_type` 的值）。
    pub id: &'static str,
    /// 面向 LLM / 文档的一句话描述。
    pub description: &'static str,
    /// 作用目标（描述性）。
    pub target: ActionTarget,
    /// 该动作 `params` 的 JSON Schema（对象；描述性元数据，供文档与一致性判据）。
    pub parameters: fn() -> Value,
    /// 确认级（唯一来源；未知 id fail-safe 见 [`ActionCatalogue::confirmation_level`]）。
    pub confirmation: ConfirmationLevel,
    /// 执行体派发标识。
    pub dispatch: ActionDispatch,
    /// 是否经 `execute_action` 工具的 `action_type` 枚举暴露给 LLM。
    pub expose_to_llm: bool,
}

fn params_status_transition() -> Value {
    json!({
        "type": "object",
        "properties": {
            "table": { "type": "string", "description": "目标表名（须落在 agent allowed_schemas 内）" },
            "status": { "type": "string", "description": "目标状态值" },
            "column": { "type": "string", "description": "状态列名（默认 status）" }
        },
        "required": ["table", "status"]
    })
}

fn params_batch_approve() -> Value {
    json!({
        "type": "object",
        "properties": {
            "action": { "type": "string", "enum": ["approve", "reject"], "description": "审批动作（默认 approve）" },
            "opinion": { "type": "string", "description": "可选审批意见" }
        }
    })
}

fn params_generate_document() -> Value {
    json!({
        "type": "object",
        "properties": {
            "draft_text": { "type": "string", "description": "草稿正文（或 content）" },
            "content": { "type": "string", "description": "草稿正文（draft_text 的别名）" },
            "doc_type": { "type": "string", "description": "单据类型标签（默认 document）" }
        },
        "required": ["draft_text"]
    })
}

fn params_send_notification() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string", "description": "通知标题" },
            "content": { "type": "string", "description": "通知正文" },
            "user_id": { "type": "integer", "description": "接收人（缺省用 target_ids[0] 或当前用户）" }
        },
        "required": ["title", "content"]
    })
}

fn params_ngac_org_policy_apply() -> Value {
    json!({
        "type": "object",
        "properties": {
            "operations": { "type": "array", "description": "规范操作清单（upsert_class/patch_class/create_rule/…）" }
        },
        "required": ["operations"]
    })
}

fn params_ngac_strategy_apply() -> Value {
    json!({
        "type": "object",
        "properties": {
            "operations": { "type": "array", "description": "策略操作清单（association/prohibition/UA 指派）" }
        },
        "required": ["operations"]
    })
}

fn params_system_config_apply() -> Value {
    json!({
        "type": "object",
        "properties": {
            "operations": { "type": "array", "description": "逐项配置变更（每项 op ∈ create_config/update_config/delete_config）" }
        },
        "required": ["operations"]
    })
}

/// 动作目录（唯一真相源）：7 个动作。
/// **只读分析**（NGAC dry-run / explain-paths）MUST NOT 入目录——它们经
/// `ActionHandler::analyze` 独立可达（`wire-org-policy-declaration-fields` tasks 3.3），
/// 以免与可执行动作共用 `execute_action` 枚举/确认级面（能力外溢）。
pub const ACTION_CATALOGUE: &[ActionSpec] = &[
    ActionSpec {
        id: "status_transition",
        description: "推进实体状态（通用 UPDATE + 触发器 NGAC 域门禁）",
        target: ActionTarget::EntityRecord,
        parameters: params_status_transition,
        confirmation: ConfirmationLevel::Explicit,
        dispatch: ActionDispatch::StatusTransition,
        expose_to_llm: true,
    },
    ActionSpec {
        id: "batch_approve",
        description: "批量审批 / 驳回（approvals 服务层）",
        target: ActionTarget::Approval,
        parameters: params_batch_approve,
        confirmation: ConfirmationLevel::Explicit,
        dispatch: ActionDispatch::BatchApprove,
        expose_to_llm: true,
    },
    ActionSpec {
        id: "generate_document",
        description: "生成单据草稿（assistant 消息落库）",
        target: ActionTarget::Document,
        parameters: params_generate_document,
        confirmation: ConfirmationLevel::None,
        dispatch: ActionDispatch::GenerateDocument,
        expose_to_llm: true,
    },
    ActionSpec {
        id: "send_notification",
        description: "发送系统通知（system_push 落库服务）",
        target: ActionTarget::Notification,
        parameters: params_send_notification,
        confirmation: ConfirmationLevel::Preview,
        dispatch: ActionDispatch::SendNotification,
        expose_to_llm: true,
    },
    // 以下三个动作**不暴露**给 LLM 的 execute_action 枚举——只经提案 actions[]
    // 可达（ngac_policy_draft / ngac_strategy_draft / system_config_draft 产出）。
    // 保持既有行为：MUST NOT 因收拢而把它们暴露给模型。
    ActionSpec {
        id: "ngac_org_policy_apply",
        description: "NGAC 权限规范写入（SSO org-policy 服务层，admin 门控）",
        target: ActionTarget::NgacPolicy,
        parameters: params_ngac_org_policy_apply,
        confirmation: ConfirmationLevel::Explicit,
        dispatch: ActionDispatch::NgacOrgPolicyApply,
        expose_to_llm: false,
    },
    ActionSpec {
        id: "ngac_strategy_apply",
        description: "NGAC 策略操作（association/prohibition/UA 指派，admin 门控）",
        target: ActionTarget::NgacStrategy,
        parameters: params_ngac_strategy_apply,
        confirmation: ConfirmationLevel::Explicit,
        dispatch: ActionDispatch::NgacStrategyApply,
        expose_to_llm: false,
    },
    ActionSpec {
        id: "system_config_apply",
        description: "系统设置变更（凭证加密 + system_config.* 审计，admin 门控）",
        target: ActionTarget::SystemConfig,
        parameters: params_system_config_apply,
        confirmation: ConfirmationLevel::Explicit,
        dispatch: ActionDispatch::SystemConfigApply,
        expose_to_llm: false,
    },
];

/// 目录访问器。
pub struct ActionCatalogue;

impl ActionCatalogue {
    /// 全部条目。
    pub fn all() -> &'static [ActionSpec] {
        ACTION_CATALOGUE
    }

    /// 按 id 查条目。
    pub fn get(id: &str) -> Option<&'static ActionSpec> {
        ACTION_CATALOGUE.iter().find(|s| s.id == id)
    }

    /// LLM 经 `execute_action` 的 `action_type` 枚举可见的 id（目录中 `expose_to_llm` 项）。
    pub fn llm_exposed_ids() -> Vec<&'static str> {
        ACTION_CATALOGUE
            .iter()
            .filter(|s| s.expose_to_llm)
            .map(|s| s.id)
            .collect()
    }

    /// 确认级——目录派生；未知 id fail-safe 归 `Explicit`（无手写 match/通配）。
    pub fn confirmation_level(id: &str) -> ConfirmationLevel {
        Self::get(id)
            .map(|s| s.confirmation.clone())
            .unwrap_or(ConfirmationLevel::Explicit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::executor::ExecuteActionTool;

    /// 全部 `ActionDispatch` 变体（测试本地清单）：用于断言目录穷尽覆盖派发面。
    const ALL_DISPATCHES: &[ActionDispatch] = &[
        ActionDispatch::BatchApprove,
        ActionDispatch::SendNotification,
        ActionDispatch::StatusTransition,
        ActionDispatch::GenerateDocument,
        ActionDispatch::NgacOrgPolicyApply,
        ActionDispatch::NgacStrategyApply,
        ActionDispatch::SystemConfigApply,
    ];

    /// 目录 = 派发可达动作集的单源：id 唯一、每个派发变体恰有一条、条数 = 变体数。
    #[test]
    fn catalogue_covers_every_dispatch_exactly_once() {
        let all = ActionCatalogue::all();
        assert_eq!(all.len(), 7, "目录条数 = 既有 7 个动作");
        assert_eq!(
            all.len(),
            ALL_DISPATCHES.len(),
            "目录条数 MUST == 派发可达动作数"
        );

        // id 唯一
        let mut ids: Vec<&str> = all.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "动作 id MUST 唯一");

        // 每个派发变体恰有一条目录项（无遗漏、无重复）
        for d in ALL_DISPATCHES {
            let n = all.iter().filter(|s| s.dispatch == *d).count();
            assert_eq!(n, 1, "派发变体 {:?} MUST 恰有一条目录项", d);
        }
    }

    /// execute_action 的 action_type 枚举 MUST 由目录生成，且 == 目录里暴露项集合。
    #[test]
    fn llm_enum_equals_exposed_catalogue_subset() {
        let def = ExecuteActionTool::definition();
        let enum_vals: Vec<String> = def.parameters["properties"]["action_type"]["enum"]
            .as_array()
            .expect("enum 数组")
            .iter()
            .map(|v| v.as_str().expect("字符串枚举项").to_string())
            .collect();

        let mut exposed: Vec<String> = ActionCatalogue::llm_exposed_ids()
            .into_iter()
            .map(String::from)
            .collect();
        exposed.sort();
        let mut got = enum_vals.clone();
        got.sort();
        assert_eq!(got, exposed, "LLM 枚举 MUST == 目录里 expose_to_llm 项");

        // 显式行为钉住：三个非暴露动作 MUST NOT 出现在枚举里
        for id in [
            "ngac_org_policy_apply",
            "ngac_strategy_apply",
            "system_config_apply",
        ] {
            assert!(
                !enum_vals.iter().any(|v| v == id),
                "{id} MUST NOT 经 execute_action 枚举暴露给 LLM（只经提案 actions[] 可达）"
            );
        }
        assert_eq!(enum_vals.len(), 4, "既有暴露面恒为 4 项（行为不变）");
    }

    /// 确认级 MUST 由目录派生且无通配依赖：每条与目录一致，未知 id fail-safe。
    #[test]
    fn confirmation_level_derived_from_catalogue_no_wildcard() {
        // 既有映射保持不变
        assert!(matches!(
            ActionCatalogue::confirmation_level("send_notification"),
            ConfirmationLevel::Preview
        ));
        assert!(matches!(
            ActionCatalogue::confirmation_level("generate_document"),
            ConfirmationLevel::None
        ));
        for id in [
            "batch_approve",
            "status_transition",
            "ngac_org_policy_apply",
            "ngac_strategy_apply",
            "system_config_apply",
        ] {
            assert!(
                matches!(
                    ActionCatalogue::confirmation_level(id),
                    ConfirmationLevel::Explicit
                ),
                "{id} 确认级 MUST 为 Explicit"
            );
        }
        // 未知 id：fail-safe = Explicit（不裸执行）
        assert!(matches!(
            ActionCatalogue::confirmation_level("no_such_action"),
            ConfirmationLevel::Explicit
        ));
        // 目录内每条都能被 get 命中（id 唯一、查表正确）且派发标识自洽
        for spec in ActionCatalogue::all() {
            let found = ActionCatalogue::get(spec.id).expect("目录内 id MUST 可查");
            assert_eq!(found.dispatch, spec.dispatch, "{} 派发标识自洽", spec.id);
        }
    }
}
