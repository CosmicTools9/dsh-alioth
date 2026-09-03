use super::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use regex::Regex;

pub struct FormFillingAgent {
    config: AgentConfig,
    patterns: Vec<Regex>,
}

impl Default for FormFillingAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl FormFillingAgent {
    pub fn new() -> Self {
        let config = AgentConfig {
            code: "form_filling".to_string(),
            name: "智能填单助手".to_string(),
            description: "理解多模态信息（图片、文档、表格、聊天记录），自动提取关键字段并汇总到业务表单中".to_string(),
            capabilities: vec![
                AgentCapability::MultimodalUnderstanding,
                AgentCapability::FormFilling,
            ],
            system_prompt: r#"你是「智能填单助手」，专精于从非结构化或多模态信息中提取结构化数据并填入业务表单。

## 核心能力
1. 从图片、PDF、Excel、聊天记录中提取关键字段
2. 理解表单schema，将提取的数据映射到对应字段
3. 对缺失或模糊信息，向用户发起澄清提问
4. 支持批量填单（一张图片/文档包含多条记录）

## 工作流
1. **信息提取**：分析用户提供的文本、图片、文档内容，识别业务实体和字段
2. **字段映射**：将提取的信息映射到目标表单的字段结构
3. **缺失处理**：标记置信度低的字段，询问用户确认
4. **输出格式**：返回结构化JSON，包含 `filled_fields` 和 `pending_confirmations`

## 输出格式
回复内容分为两部分：
- 自然语言摘要（向用户说明提取了哪些信息）
- 结构化JSON块（可被前端解析自动填入表单）

```json
{
  "target_form": "表单名称",
  "filled_fields": {
    "字段名": {"value": "值", "confidence": 0.95, "source": "提取来源"}
  },
  "pending_confirmations": [
    {"field": "字段名", "reason": "原因", "suggested_value": "建议值"}
  ]
}
```

## 约束
- 不编造数据；无法识别的字段标记为 null 并要求确认
- 金额、日期等关键字段必须附带提取来源说明
- 批量填单时按记录分组输出
"#.to_string(),
            model_override: None,
            generation_params: None,
            user_selectable: true,
            sort_order: 10,
            requires_input_default: true,
            suggested_actions: vec![
                "确认并提交".to_string(),
                "修改字段".to_string(),
            ],
            ..Default::default()
        };

        let patterns = vec![
            Regex::new(r"(?i)填(写|表|单)|录入|导入|提取|识别|OCR|发票|合同|报销|申请|登记")
                .unwrap(),
            Regex::new(r"(?i)图片.*(信息|内容|文字)|拍照|截图|附件.*(填|录)").unwrap(),
            Regex::new(r"(?i)表单|form|field|字段|自动填充|autofill").unwrap(),
        ];

        Self { config, patterns }
    }
}

#[async_trait]
impl Agent for FormFillingAgent {
    fn config(&self) -> &AgentConfig {
        &self.config
    }

    fn intent_keywords(&self) -> Vec<String> {
        vec![
            "填单".to_string(),
            "填表".to_string(),
            "录入".to_string(),
            "提取".to_string(),
            "识别".to_string(),
            "发票".to_string(),
            "报销".to_string(),
            "申请".to_string(),
            "导入".to_string(),
            "自动填充".to_string(),
            "form".to_string(),
            "field".to_string(),
        ]
    }

    fn intent_patterns(&self) -> Vec<Regex> {
        self.patterns.clone()
    }
}
