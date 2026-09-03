use super::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use regex::Regex;

pub struct DocumentAutomationAgent {
    config: AgentConfig,
    patterns: Vec<Regex>,
}

impl Default for DocumentAutomationAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentAutomationAgent {
    pub fn new() -> Self {
        let config = AgentConfig {
            code: "document_automation".to_string(),
            name: "单据自动化助手".to_string(),
            description: "自动理解业务单据内容，执行批量处理、状态推进、异常标记等自动化任务"
                .to_string(),
            capabilities: vec![
                AgentCapability::DocumentAutomation,
                AgentCapability::MultimodalUnderstanding,
            ],
            system_prompt: r#"你是「单据自动化助手」，专精于理解和执行业务单据的自动化处理任务。

## 核心能力
1. 理解单据类型（订单、采购单、入库单、出库单、付款单、报销单等）
2. 自动执行批量审批、状态推进、异常标记
3. 单据间的关联检查（如订单-发货单-收款单的三单匹配）
4. 生成批量操作脚本或执行计划
5. 识别单据中的异常和待处理项

## 工作流
1. **单据识别**：判断单据类型、状态和关键字段
2. **规则匹配**：根据业务规则判断可执行的操作
3. **批量处理**：对多张单据生成统一的处理方案
4. **异常报告**：标记不符合规则的单据及原因
5. **执行建议**：给出确认前预览和回滚方案

## 输出格式
- 自然语言处理摘要
- 结构化执行计划 JSON

```json
{
  "document_type": "单据类型",
  "documents": [
    {
      "id": "单据ID",
      "status": "当前状态",
      "proposed_action": "建议操作",
      "target_status": "目标状态",
      "checks": [{"rule": "规则", "passed": true, "detail": ""}],
      "warnings": ["警告1"]
    }
  ],
  "batch_plan": {
    "total": 10,
    "pass": 8,
    "fail": 2,
    "actions": [{"action": "操作", "count": 8, "document_ids": []}]
  },
  "execution_script": "伪代码或操作描述"
}
```

## 约束
- 所有批量操作必须经过用户确认（不可自动提交）
- 涉及金额、付款、库存变动的操作必须双重校验
- 异常情况必须明确说明原因和修复建议
- 保留完整操作日志（模拟，实际执行由后端完成）
- 不直接修改数据库，只生成执行计划和预览
"#
            .to_string(),
            model_override: None,
            generation_params: Some(llm::GenerationParams {
                temperature: 0.2,
                max_tokens: 4096,
                top_p: 1.0,
                frequency_penalty: 0.0,
                presence_penalty: 0.0,
                reasoning_effort: llm::ReasoningEffort::Medium,
                thinking: None,
                service_tier: None,
                reasoning_split: None,
                response_format: None,
            }),
            user_selectable: true,
            sort_order: 50,
            requires_input_default: true,
            suggested_actions: vec![
                "确认执行".to_string(),
                "逐条审核".to_string(),
                "修改规则".to_string(),
            ],
            ..Default::default()
        };

        let patterns = vec![
            Regex::new(
                r"(?i)单据|订单|采购|入库|出库|发货|收款|付款|报销|审批|批量|自动.*处理|处理.*单",
            )
            .unwrap(),
            Regex::new(r"(?i)三单匹配|对账|核销|过账|结转|关单|完结|归档|批量.*(通过|驳回|审批)")
                .unwrap(),
            Regex::new(r"(?i)待处理|待审批|异常单|滞留|积压|催办|自动.*(推进|流转|完成)").unwrap(),
        ];

        Self { config, patterns }
    }
}

#[async_trait]
impl Agent for DocumentAutomationAgent {
    fn config(&self) -> &AgentConfig {
        &self.config
    }

    fn intent_keywords(&self) -> Vec<String> {
        vec![
            "单据".to_string(),
            "订单".to_string(),
            "采购".to_string(),
            "入库".to_string(),
            "出库".to_string(),
            "发货".to_string(),
            "收款".to_string(),
            "付款".to_string(),
            "报销".to_string(),
            "审批".to_string(),
            "批量".to_string(),
            "自动处理".to_string(),
            "三单匹配".to_string(),
            "对账".to_string(),
            "核销".to_string(),
            "过账".to_string(),
            "关单".to_string(),
            "归档".to_string(),
        ]
    }

    fn intent_patterns(&self) -> Vec<Regex> {
        self.patterns.clone()
    }
}
