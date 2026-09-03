use super::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;

pub struct GeneralAssistantAgent {
    config: AgentConfig,
}

impl Default for GeneralAssistantAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl GeneralAssistantAgent {
    pub fn new() -> Self {
        let config = AgentConfig {
            code: "general".to_string(),
            name: "通用助手".to_string(),
            description: "通用企业助手，回答业务知识、系统使用、一般性问题".to_string(),
            capabilities: vec![AgentCapability::GeneralConversation],
            system_prompt: r#"你是 AliothStudio 企业助手，帮助用户解答业务问题、系统使用指导和一般性咨询。

## 核心能力
1. 回答 AliothStudio 平台的使用方法和功能说明
2. 解释业务概念、数据模型、模块关系
3. 提供操作步骤指引和最佳实践建议
4. 当问题超出能力范围时，明确告知用户并建议转接专用 Agent

## 约束
- 回答简洁、专业，使用中文
- 不确定的信息明确说明，不编造
- 涉及具体操作时，给出可执行的步骤
- 如果用户的问题明显属于某个专用 Agent 的能力范围（如填单、数据分析、流程设计），主动建议用户切换到对应 Agent
"#.to_string(),
            model_override: None,
            generation_params: None,
            user_selectable: false, // 通用助手不可手动选择，作为回退
            sort_order: 99,
            ..Default::default()
        };

        Self { config }
    }
}

#[async_trait]
impl Agent for GeneralAssistantAgent {
    fn config(&self) -> &AgentConfig {
        &self.config
    }

    fn intent_keywords(&self) -> Vec<String> {
        vec![]
    }

    fn intent_patterns(&self) -> Vec<regex::Regex> {
        vec![]
    }
}
