use super::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;

/// 权限规范作者代理（route-ngac-policy-authoring-through-ai-assist 2.4）。
///
/// 只授予 `ngac_policy_draft`（提案型工具，零写入）+ 零 DB 直访；真实写径 =
/// 用户确认后的 `POST /api/chat-sessions/{sid}/execute-action`（ADR D-051：
/// 工具路径 MUST NOT 自证确认、MUST NOT 落库）。
/// 路由：页面上下文 `suggestedAgent='org_policy_author'`（/admin/org-policy）。
pub struct OrgPolicyAuthorAgent {
    config: AgentConfig,
}

impl Default for OrgPolicyAuthorAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl OrgPolicyAuthorAgent {
    pub fn new() -> Self {
        let config = AgentConfig {
            code: "org_policy_author".to_string(),
            name: "权限规范助手".to_string(),
            description: "以自然语言草拟权限规范（类/规则/分级）变更提案，经用户确认后落库"
                .to_string(),
            capabilities: vec![AgentCapability::GeneralConversation],
            system_prompt: r#"你是 NGAC 权限规范作者助手，负责把管理员的自然语言诉求整理为「权限规范变更提案」。

## 工作方式（铁律）
1. 只提案、不落库：你必须调用 `ngac_policy_draft` 工具生成提案；工具返回的是 `needs_confirmation` 提案。MUST NOT 声称已执行、已保存或已生效。
2. 提案在管理员点击确认后经确认通道执行；服务端会做 admin 门控、Rhai 强校验、状态机（编辑门）、审计。
3. 状态机迁移（提交评审 / 激活 / 退役）不在你的范围——提示管理员在管理面操作。
4. 诉求不明确或实体引用不确定时，先追问澄清（不调用工具）。
5. 评估变更影响：用 `ngac_dry_run` 工具预演候选策略补丁的判决前后差异（只读、零写库），
   并在提案 message 中给出影响摘要；解释存量授权来源用 `ngac_explain_paths` 工具（路径证据）；
   回答「某用户能否对某资源做某动作」用 `ngac_check_access` 工具（批量判定，与服务端裁决同源）。

## 操作面（ngac_policy_draft 的 operations[]）
- upsert_class {code, notice, scope{resource_types, actions}, ua_template{name_rule}, label_code, prohibition_template, audit_required, comments}
- patch_class {class_id, 其余同上可选}
- create_rule {policy_class_id, subject_code, resource_type, actions[], condition, obligation, label_code}
- patch_rule {rule_id, 其余可选} / delete_rule {rule_id}
- create_label {code, rank, notice, domain} / patch_label {code, 其余可选} / delete_label {code}
- 每项建议附 summary（一句话中文摘要；确认弹窗展示用）。
- 编辑门：仅 draft/in_review 的规范类可挂接/更新规则；active/retired 内容冻结。

## 表达式字段（condition / obligation / prohibition_template）
MUST 为 Rhai 源码字符串（禁止 JSON 结构化占位）。可用标识符按字段区分（写径强校验，越界即拒绝）：
- `condition` / `prohibition_template`：user_id、resource、action、subject_attributes、object_attributes
  （**不含** decision、permitted——条件在判决终态前求值）；
- `obligation`（事件模式 / 义务算子）：上述五键 + decision、permitted。
自由函数仅限平台内建白名单；不确定就不写表达式（留空）。

## 回复契约（structured.actions）
工具返回提案后，在回复末尾附加一个 ```json 代码块，内容为该提案对象（id/kind/action_type/params 原样保留），供前端渲染确认按钮：
```json
{"actions":[{"id":"ngac:org-policy-apply","label":"应用 N 项权限规范变更","kind":"execute","action_type":"ngac_org_policy_apply","params":{"operations":[...]}}]}
```
- 无提案时（追问/解释）不要输出 actions 字段。
- 回复简洁、专业、中文；解释操作要点与风险（例如：activate 后需组织方案再派生才影响判定）。"#
                .to_string(),
            user_selectable: false,
            sort_order: 40,
            icon: "ShieldCheck".to_string(),
            category: "admin".to_string(),
            available_tools: vec![
                crate::tools::executor::NgacPolicyDraftTool::definition(),
                // 只读分析工具（tasks 3.3）：候选规范变更的补丁级影响预演 + 路径证据。
                // 零写库、无确认级；`expose_to_llm: false` 的目录条目经宿主
                // `ActionHandler` 收窄可达 ⇒ 能力不外溢到其它可执行动作。
                crate::tools::executor::NgacDryRunTool::definition(),
                crate::tools::executor::NgacExplainPathsTool::definition(),
                crate::tools::executor::NgacCheckAccessTool::definition(),
            ],
            db_access_level: super::DbAccessLevel::None,
            allowed_schemas: vec![],
            max_execution_steps: 4,
            ..Default::default()
        };
        Self { config }
    }
}

#[async_trait]
impl Agent for OrgPolicyAuthorAgent {
    fn config(&self) -> &AgentConfig {
        &self.config
    }

    fn intent_keywords(&self) -> Vec<String> {
        vec![
            "权限规范".to_string(),
            "授权规范".to_string(),
            "职责规则".to_string(),
            "分级字典".to_string(),
        ]
    }

    fn intent_patterns(&self) -> Vec<regex::Regex> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use crate::agents::{build_default_registry, DbAccessLevel};

    #[test]
    fn registered_with_draft_and_analysis_tools_and_unlisted() {
        let reg = build_default_registry();
        let agent = reg.get("org_policy_author").expect("内置注册缺失");
        let cfg = agent.config();
        assert_eq!(cfg.available_tools.len(), 4);
        let names: Vec<&str> = cfg
            .available_tools
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert!(names.contains(&"ngac_policy_draft"));
        assert!(
            names.contains(&"ngac_dry_run")
                && names.contains(&"ngac_explain_paths")
                && names.contains(&"ngac_check_access"),
            "只读分析工具（影响预演 / 路径证据 / 访问检查）MUST 可用"
        );
        assert!(
            !names.contains(&"execute_action"),
            "execute_action MUST NOT 授予（能力不外溢到其它可执行动作）"
        );
        assert!(
            !cfg.user_selectable,
            "不可手动选择（页面 suggestedAgent 路由）"
        );
        assert!(matches!(cfg.db_access_level, DbAccessLevel::None));
        assert!(cfg.system_prompt.contains("ngac_policy_draft"));
    }
}
