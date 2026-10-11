use super::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;

/// 系统设置作者代理（add-system-config-ai-action 2.4）。
///
/// 只授予 `system_config_draft`（提案型工具，零写入）+ 零 DB 直访；真实写径 =
/// 用户确认后的 `POST /api/chat-sessions/{sid}/execute-action` 动作
/// `system_config_apply`（admin 门控 + 凭证加密 + `system_config.*` 审计等价）。
/// 路由：页面上下文 `suggestedAgent='system_config_admin'`（/admin/infrastructure）。
pub struct SystemConfigAdminAgent {
    config: AgentConfig,
}

impl Default for SystemConfigAdminAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemConfigAdminAgent {
    pub fn new() -> Self {
        let config = AgentConfig {
            code: "system_config_admin".to_string(),
            name: "系统设置助手".to_string(),
            description: "以自然语言草拟系统设置（LLM/邮件等 provider 行）变更提案，经用户确认后落库"
                .to_string(),
            capabilities: vec![AgentCapability::GeneralConversation],
            system_prompt: r#"你是系统设置（system-config）作者助手，负责把管理员的自然语言诉求整理为「系统设置变更提案」。

## 工作方式（铁律）
1. 只提案、不落库：你必须调用 `system_config_draft` 工具生成提案；工具返回的是 `needs_confirmation` 提案。MUST NOT 声称已执行、已保存或已生效。
2. 提案在管理员点击确认后经确认通道执行；服务端会做 admin 门控、凭证加密与 `system_config.*` 审计。
3. 诉求不明确时 MUST 追问澄清：目标行不可辨、分类/provider 取值不确定时由管理员补充。本代理**未被授予任何读取工具**（`db_access_level = None`），MUST NOT 声称已查询配置列表/分类，MUST NOT 编造 category/provider 取值。
4. 连接测试（「测试连接」）不是你的动作面：提示管理员在表单上操作。

## 操作面（system_config_draft 的 operations[]）
- create_config {category（必填，如 llm/email/im/webhook/storage/sms）, provider（必填）, code?, notice?, settings?, credentials?, enabled?, is_default?, domain_?, comments?}
- update_config {id（必填）, 其余可选}
- delete_config {id（必填）}（软删）
- 每项建议附 summary（一句话中文摘要；确认弹窗展示用）。

## 凭证纪律（重要）
- 凭证（api_key/password/secret 等）MUST 只放在 `credentials` 对象（专属键），MUST NOT 混入 `settings`；非对象会被服务端拒绝。
- 对方已在消息里给出的凭证值，MUST NOT 在你的回复正文里重复（工具参数原样即可）；回复只说明「凭证将按 schema 加密保存」。

## 回复契约（structured.actions）
工具返回提案后，在回复末尾附加一个 ```json 代码块，内容为该提案对象（id/kind/action_type/params 原样保留），供前端渲染确认按钮：
```json
{"actions":[{"id":"syscfg:apply","label":"应用 N 项系统设置变更","kind":"execute","action_type":"system_config_apply","params":{"operations":[...]}}]}
```
- 无提案时（追问/解释）不要输出 actions 字段。
- 回复简洁、专业、中文；说明改动点与影响（如 is_default 切换会影响 AI-Chat 的 LLM 装配）。"#
                .to_string(),
            user_selectable: false,
            sort_order: 45,
            icon: "Settings2".to_string(),
            category: "admin".to_string(),
            available_tools: vec![crate::tools::executor::SystemConfigDraftTool::definition()],
            db_access_level: super::DbAccessLevel::None,
            allowed_schemas: vec![],
            max_execution_steps: 4,
            ..Default::default()
        };
        Self { config }
    }
}

#[async_trait]
impl Agent for SystemConfigAdminAgent {
    fn config(&self) -> &AgentConfig {
        &self.config
    }

    fn intent_keywords(&self) -> Vec<String> {
        vec![
            "系统设置".to_string(),
            "邮件配置".to_string(),
            "SMTP".to_string(),
            "API Key".to_string(),
            "模型配置".to_string(),
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
    fn registered_with_draft_tool_only_and_unlisted() {
        let reg = build_default_registry();
        let agent = reg.get("system_config_admin").expect("内置注册缺失");
        let cfg = agent.config();
        assert_eq!(cfg.available_tools.len(), 1);
        assert_eq!(cfg.available_tools[0].name, "system_config_draft");
        assert!(
            !cfg.user_selectable,
            "不可手动选择（页面 suggestedAgent 路由）"
        );
        assert!(matches!(cfg.db_access_level, DbAccessLevel::None));
        assert!(cfg.system_prompt.contains("system_config_draft"));
        assert!(cfg.system_prompt.contains("credentials"));
    }
}
