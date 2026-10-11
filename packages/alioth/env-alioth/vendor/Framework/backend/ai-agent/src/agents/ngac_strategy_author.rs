use super::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;

/// NGAC 策略操作代理（wire-nl-assist-into-assistant）。
///
/// 只授予 `ngac_strategy_draft`（提案型，零写入）+ `query_sql`（只读 SELECT，探索策略图）；
/// 真实写径 = 用户确认后的 `POST /api/chat-sessions/{sid}/execute-action`
/// 动作 `ngac_strategy_apply`（admin 门控 + 审计，ADR D-051 提案-确认铁律）。
/// 路由：页面上下文 `suggestedAgent='ngac_strategy_author'`（/admin/permissions）。
pub struct NgacStrategyAuthorAgent {
    config: AgentConfig,
}

impl Default for NgacStrategyAuthorAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl NgacStrategyAuthorAgent {
    pub fn new() -> Self {
        let config = AgentConfig {
            code: "ngac_strategy_author".to_string(),
            name: "NGAC 策略操作".to_string(),
            description: "自然语言 → NGAC 策略操作提案（关联/禁止/UA 指派）——只提案不落库，\
经用户确认后经确认通道执行。"
                .to_string(),
            capabilities: vec![AgentCapability::GeneralConversation],
            system_prompt: r#"你是 NGAC 策略操作助手，负责把管理员的自然语言诉求整理为「策略操作提案」。

## 工作方式（铁律）
1. 只提案、不落库：你必须调用 `ngac_strategy_draft` 工具生成提案；工具返回的是 `needs_confirmation` 提案。MUST NOT 声称已执行、已保存或已生效。
2. 提案在管理员点击确认后经确认通道执行；服务端会做 admin 门控、审计。
3. UA/OA/用户 id MUST 先用 `query_sql` 工具查实（SELECT），MUST NOT 编造 id。
4. 诉求不明确或实体引用不确定时，先追问澄清（不调用工具）。
5. 提案前 SHOULD 用 `ngac_dry_run` 工具预演影响（patches = 你的候选关联/禁止补丁，
   checks = 关键判决点）并在提案 message 中给出 before/after/flipped 摘要；解释存量授权时用
   `ngac_explain_paths` 工具（只读路径证据）；回答「某用户能否对某资源做某动作」用
   `ngac_check_access` 工具（批量判定，与服务端裁决同源）。三者**零写库**、无需确认。

## 操作面（ngac_strategy_draft 的 operations[]）
- create_association {ua_id, oa_id, actions[]}
- delete_association {association_id}
- create_prohibition {ua_id, oa_id, actions[]}
- delete_prohibition {prohibition_id}
- assign_ua {user_id, ua_id}
- remove_ua_assignment {user_id, ua_id}

## 查询模板（query_sql 工具）
- UA 清单：SELECT id, o_name FROM isahl_auth.ngac_user_attribute WHERE deleted_at IS NULL
- OA 清单：SELECT id, resource_type, fk_resource FROM isahl_auth.ngac_object_attribute WHERE deleted_at IS NULL
- 关联清单：SELECT a.id, ua.o_name AS ua, oa.resource_type AS rt, ar.o_name AS action FROM isahl_auth.ngac_association a JOIN isahl_auth.ngac_user_attribute ua ON a.fk_user_attribute=ua.id JOIN isahl_auth.ngac_object_attribute oa ON a.fk_object_attribute=oa.id JOIN isahl_auth.ngac_access_right ar ON ar.id = ANY(a.ak_access_rights) WHERE a.deleted_at IS NULL
- 用户清单：SELECT id, username FROM isahl_auth.auth_users LIMIT 50

## 回复契约（structured.actions）
工具返回提案后，在回复末尾附加一个 ```json 代码块，内容为该提案对象，供前端渲染确认按钮：
```json
{"actions":[{"id":"ngac:strategy-apply","label":"应用 N 项策略操作","kind":"execute","action_type":"ngac_strategy_apply","params":{"operations":[...]}}]}
```
- 无提案时（追问/解释）不要输出 actions 字段。
- 回复简洁、专业、中文；解释操作要点与风险（例如：禁止规则的优先级高于关联）。"#
                .to_string(),
            user_selectable: false,
            sort_order: 41,
            icon: "Shield".to_string(),
            category: "admin".to_string(),
            available_tools: vec![
                crate::tools::executor::QuerySqlTool::definition(),
                crate::tools::executor::NgacStrategyDraftTool::definition(),
                // 只读分析工具（tasks 3.3）：提案前预演影响 / 解释存量授权。
                // 经 `ToolContext.action_handler` 收窄执行（目录条目不暴露给
                // `execute_action` 枚举）⇒ 能力不外溢到其它可执行动作。
                crate::tools::executor::NgacDryRunTool::definition(),
                crate::tools::executor::NgacExplainPathsTool::definition(),
                crate::tools::executor::NgacCheckAccessTool::definition(),
            ],
            db_access_level: super::DbAccessLevel::ReadOnly,
            allowed_schemas: vec![],
            max_execution_steps: 6,
            ..Default::default()
        };
        Self { config }
    }
}

#[async_trait]
impl Agent for NgacStrategyAuthorAgent {
    fn config(&self) -> &AgentConfig {
        &self.config
    }

    fn intent_keywords(&self) -> Vec<String> {
        vec![
            "权限".to_string(),
            "策略".to_string(),
            "关联".to_string(),
            "禁止".to_string(),
            "指派".to_string(),
            "NGAC".to_string(),
            "association".to_string(),
            "prohibition".to_string(),
        ]
    }

    fn intent_patterns(&self) -> Vec<regex::Regex> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use crate::agents::build_default_registry;

    #[test]
    fn registered_with_draft_and_query_tools() {
        let reg = build_default_registry();
        let agent = reg.get("ngac_strategy_author").expect("内置注册缺失");
        let cfg = agent.config();
        // DbAccessLevel::ReadOnly（assert_eq 因 derive 顺序问题用 matches! 替代）
        assert!(matches!(
            cfg.db_access_level,
            crate::agents::DbAccessLevel::ReadOnly
        ));
        let names: Vec<&str> = cfg
            .available_tools
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert!(names.contains(&"ngac_strategy_draft"));
        assert!(names.contains(&"query_sql"));
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
    }
}
