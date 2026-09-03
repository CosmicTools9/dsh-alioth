use super::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use regex::Regex;

pub struct FlowDesignAgent {
    config: AgentConfig,
    patterns: Vec<Regex>,
}

impl Default for FlowDesignAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl FlowDesignAgent {
    pub fn new() -> Self {
        let config = AgentConfig {
            code: "flow_design".to_string(),
            name: "流程设计助手".to_string(),
            description: "将业务需求转化为可执行的业务流程设计，包括审批流、状态机、工作流"
                .to_string(),
            capabilities: vec![
                AgentCapability::FlowDesign,
                AgentCapability::DocumentAutomation,
            ],
            system_prompt: r#"你是「流程设计助手」，专精于将业务需求转化为可执行的业务流程。

## 核心能力
1. 理解业务需求，设计审批流程、状态流转、工作流步骤
2. 生成 Mermaid 流程图
3. 识别流程中的角色、条件分支、会签/或签节点
4. 输出 Alioth 工作流配置（constraints.yaml、workflows.yaml、statemachines.yaml 结构）
5. 检查流程闭环性和死路

## 工作流
1. **需求拆解**：识别业务目标、参与角色、关键状态
2. **流程建模**：设计状态机 + 工作流步骤
3. **条件设计**：定义流转条件和审批规则
4. **输出结构化配置**：返回可被 Alioth 运行时解析的流程定义

## 输出格式
- 自然语言流程说明
- Mermaid 流程图代码
- 结构化流程配置 JSON

```json
{
  "flow_name": "流程名称",
  "participants": ["角色1", "角色2"],
  "states": [{"name": "状态名", "type": "start|intermediate|end"}],
  "transitions": [
    {"from": "", "to": "", "condition": "", "action": ""}
  ],
  "workflow_config": {
    "steps": [{"step_id": "", "name": "", "assignee": "", "condition": ""}]
  },
  "mermaid": "graph TD; ..."
}
```

## 约束
- 所有流程必须有明确的起始和结束状态
- 条件分支必须覆盖默认路径
- 审批节点必须说明是 会签(all) 还是 或签(any)
- 状态命名使用物理列名风格（英文 snake_case）
- 不生成实际 DDL，只生成流程定义
"#
            .to_string(),
            model_override: None,
            generation_params: None,
            user_selectable: true,
            sort_order: 30,
            suggested_actions: vec![
                "生成配置".to_string(),
                "导出 Mermaid".to_string(),
                "模拟执行".to_string(),
            ],
            ..Default::default()
        };

        let patterns = vec![
            Regex::new(r"(?i)流程|审批|工作流|workflow|状态机|statemachine|流转|节点|步骤| BPM")
                .unwrap(),
            Regex::new(r"(?i)设计.*流程|定义.*审批|配置.*工作流|画.*流程图|怎么.*走流程").unwrap(),
            Regex::new(r"(?i)会签|或签|并行审批|条件分支|自动流转|超时提醒|催办").unwrap(),
        ];

        Self { config, patterns }
    }
}

#[async_trait]
impl Agent for FlowDesignAgent {
    fn config(&self) -> &AgentConfig {
        &self.config
    }

    fn intent_keywords(&self) -> Vec<String> {
        vec![
            "流程".to_string(),
            "审批".to_string(),
            "工作流".to_string(),
            "workflow".to_string(),
            "状态机".to_string(),
            "statemachine".to_string(),
            "流转".to_string(),
            "节点".to_string(),
            "步骤".to_string(),
            "会签".to_string(),
            "或签".to_string(),
            "条件分支".to_string(),
            "自动流转".to_string(),
            "超时".to_string(),
            "催办".to_string(),
        ]
    }

    fn intent_patterns(&self) -> Vec<Regex> {
        self.patterns.clone()
    }
}
