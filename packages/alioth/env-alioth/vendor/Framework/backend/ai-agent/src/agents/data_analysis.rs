use super::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use regex::Regex;

pub struct DataAnalysisAgent {
    config: AgentConfig,
    patterns: Vec<Regex>,
}

impl Default for DataAnalysisAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl DataAnalysisAgent {
    pub fn new() -> Self {
        let config = AgentConfig {
            code: "data_analysis".to_string(),
            name: "数据分析助手".to_string(),
            description: "理解业务数据，生成分析洞察、趋势预测和可视化建议".to_string(),
            capabilities: vec![
                AgentCapability::DataAnalysis,
                AgentCapability::MultimodalUnderstanding,
            ],
            system_prompt: r#"你是「数据分析助手」，专精于业务数据的理解、分析与洞察生成。

## 核心能力
1. 理解业务数据schema，识别关键指标和维度
2. 生成SQL查询建议（PostgreSQL方言）
3. 提供趋势分析、同比环比、异常检测
4. 推荐合适的图表类型和可视化方案
5. 将分析结果转化为业务语言

## 工作流
1. **需求理解**：明确用户想分析什么指标、什么维度、什么时间范围
2. **数据探查**：根据页面上下文或用户描述，推断可用数据表和字段
3. **分析执行**：生成分析思路（必要时提供SQL）
4. **洞察输出**：用自然语言解释数据含义，标注关键发现

## 输出格式
- 自然语言分析摘要
- 推荐的SQL查询（如适用）
- 可视化建议（图表类型、维度、指标）
- 结构化分析结果JSON

```json
{
  "analysis_type": "趋势分析|对比分析|异常检测|预测",
  "metrics": [{"name": "指标名", "value": "值", "unit": "单位"}],
  "sql_suggestion": "SELECT ...",
  "visualization": {"chart_type": "line|bar|pie|table", "x_axis": "", "y_axis": ""},
  "insights": ["洞察1", "洞察2"],
  "recommendations": ["建议1", "建议2"]
}
```

## 约束
- 不提供可能破坏数据的DML/DDL语句
- 对数据隐私敏感的内容进行脱敏提示
- 不确定的数据关系要明确说明假设
- 时间序列分析需说明时间粒度
"#
            .to_string(),
            model_override: Some("deepseek-v4-pro".to_string()),
            generation_params: Some(llm::GenerationParams {
                temperature: 0.3,
                max_tokens: 8192,
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
            sort_order: 20,
            available_tools: vec![super::ToolDefinition {
                name: "query_sql".to_string(),
                description: "执行SQL查询以获取业务数据".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "SQL查询语句"}
                    },
                    "required": ["query"]
                }),
                execution_target: super::ExecutionTarget::Backend,
            }],
            db_access_level: super::DbAccessLevel::SchemaRestricted,
            allowed_schemas: vec!["isahl".to_string()],
            max_execution_steps: 5,
            suggested_actions: vec![
                "生成SQL".to_string(),
                "推荐图表".to_string(),
                "导出报告".to_string(),
            ],
            ..Default::default()
        };

        let patterns = vec![
            Regex::new(r"(?i)分析|统计|趋势|同比|环比|报表|图表|可视化|dashboard|看板|指标|KPI")
                .unwrap(),
            Regex::new(r"(?i)SQL|查询|query|数据.*多少|销量|库存|金额|利润|增长|下降").unwrap(),
            Regex::new(r"(?i)为什么|原因|归因|异常|波动|对比|排名|占比|分布").unwrap(),
        ];

        Self { config, patterns }
    }
}

#[async_trait]
impl Agent for DataAnalysisAgent {
    fn config(&self) -> &AgentConfig {
        &self.config
    }

    fn intent_keywords(&self) -> Vec<String> {
        vec![
            "分析".to_string(),
            "统计".to_string(),
            "趋势".to_string(),
            "报表".to_string(),
            "图表".to_string(),
            "可视化".to_string(),
            "查询".to_string(),
            "SQL".to_string(),
            "同比".to_string(),
            "环比".to_string(),
            "排名".to_string(),
            "占比".to_string(),
            "异常".to_string(),
            "波动".to_string(),
            "dashboard".to_string(),
            "看板".to_string(),
            "指标".to_string(),
            "KPI".to_string(),
        ]
    }

    fn intent_patterns(&self) -> Vec<Regex> {
        self.patterns.clone()
    }
}
