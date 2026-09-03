use super::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use regex::Regex;

pub struct SimulationAgent {
    config: AgentConfig,
    patterns: Vec<Regex>,
}

impl Default for SimulationAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulationAgent {
    pub fn new() -> Self {
        let config = AgentConfig {
            code: "simulation".to_string(),
            name: "仿真验证助手".to_string(),
            description: "基于业务规则和假设条件进行仿真推演，验证决策可行性".to_string(),
            capabilities: vec![AgentCapability::Simulation, AgentCapability::DataAnalysis],
            system_prompt: r#"你是「仿真验证助手」，专精于基于业务规则和假设条件进行推演验证。

## 核心能力
1. 理解业务约束和规则（如库存上下限、审批额度、交付周期）
2. 根据用户假设进行多情景推演（what-if 分析）
3. 计算关键路径、瓶颈识别、资源冲突
4. 蒙特卡洛风格概率推演（在数据不足时给出置信区间）
5. 验证流程设计的可行性

## 工作流
1. **规则提取**：从页面上下文或用户描述中提取业务约束
2. **假设建模**：将用户的 what-if 问题转化为参数化模型
3. **情景推演**：运行多种情景，计算结果分布
4. **风险评估**：标注高风险路径和边界条件
5. **建议输出**：给出最优策略和 fallback 方案

## 输出格式
- 自然语言推演报告
- 情景对比表格
- 结构化仿真结果 JSON

```json
{
  "simulation_type": "what-if|瓶颈分析|资源冲突|流程验证",
  "scenarios": [
    {
      "name": "情景名称",
      "parameters": {"参数名": "值"},
      "outcomes": [{"metric": "指标", "value": "值", "unit": "单位"}],
      "risk_level": "low|medium|high",
      "probability": 0.75
    }
  ],
  "bottlenecks": [{"stage": "环节", "constraint": "约束", "impact": "影响"}],
  "recommendation": "最优策略",
  "fallback": "备选方案"
}
```

## 约束
- 明确标注推演基于的假设条件
- 数据不足时给出置信区间，不捏造精确数字
- 涉及财务/合规的推演需附加风险提示
- 复杂模型分步骤呈现，避免一次性输出过多计算
"#
            .to_string(),
            model_override: None,
            generation_params: Some(llm::GenerationParams {
                temperature: 0.4,
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
            sort_order: 40,
            suggested_actions: vec![
                "调整参数再试".to_string(),
                "导出报告".to_string(),
                "推荐方案".to_string(),
            ],
            ..Default::default()
        };

        let patterns = vec![
            Regex::new(r"(?i)仿真|模拟|验证|推演|what.if|如果|假设|情景|场景|预测|预估").unwrap(),
            Regex::new(r"(?i)瓶颈|冲突|资源|负荷|产能|交期|风险|评估|压力测试").unwrap(),
            Regex::new(r"(?i)试试|看一下|会怎样|是否合理|可行吗|能不能|是否够").unwrap(),
        ];

        Self { config, patterns }
    }
}

#[async_trait]
impl Agent for SimulationAgent {
    fn config(&self) -> &AgentConfig {
        &self.config
    }

    fn intent_keywords(&self) -> Vec<String> {
        vec![
            "仿真".to_string(),
            "模拟".to_string(),
            "验证".to_string(),
            "推演".to_string(),
            "what-if".to_string(),
            "假设".to_string(),
            "情景".to_string(),
            "场景".to_string(),
            "预测".to_string(),
            "瓶颈".to_string(),
            "冲突".to_string(),
            "风险".to_string(),
            "评估".to_string(),
            "压力测试".to_string(),
            "可行性".to_string(),
        ]
    }

    fn intent_patterns(&self) -> Vec<Regex> {
        self.patterns.clone()
    }
}
