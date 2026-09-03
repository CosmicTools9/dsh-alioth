//! 规则执行上下文
//!
//! 提供表达式/约束/规则求值时的上下文信息。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 规则执行上下文
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleContext {
    /// 正在处理的实体名称
    pub entity_name: String,
    /// 当前字段值
    pub field_values: HashMap<String, serde_json::Value>,
    /// 当前状态（如有状态机）
    #[serde(default)]
    pub current_state: Option<String>,
    /// 触发操作
    pub operation: RuleOperation,
    /// 用户上下文
    #[serde(default)]
    pub fk_user: Option<String>,
    /// 时间戳
    pub timestamp: String,
}

/// 触发规则评估的操作类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RuleOperation {
    Create,
    Update,
    Delete,
    Transition,
    Query,
}

impl std::fmt::Display for RuleOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuleOperation::Create => write!(f, "create"),
            RuleOperation::Update => write!(f, "update"),
            RuleOperation::Delete => write!(f, "delete"),
            RuleOperation::Transition => write!(f, "transition"),
            RuleOperation::Query => write!(f, "query"),
        }
    }
}

/// 规则评估结果
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuleEvaluationResult {
    pub evaluations: Vec<RuleEvaluation>,
    pub all_passed: bool,
    pub failures: Vec<RuleEvaluation>,
}

impl RuleEvaluationResult {
    pub fn new() -> Self {
        Self {
            evaluations: Vec::new(),
            all_passed: true,
            failures: Vec::new(),
        }
    }

    pub fn add_evaluation(&mut self, eval: RuleEvaluation) {
        if !eval.passed {
            self.all_passed = false;
            if eval.blocking {
                self.failures.push(eval.clone());
            }
        }
        self.evaluations.push(eval);
    }

    pub fn has_blocking_failures(&self) -> bool {
        self.failures.iter().any(|f| f.blocking)
    }

    pub fn error_messages(&self) -> Vec<String> {
        self.failures
            .iter()
            .filter_map(|f| f.error.clone())
            .collect()
    }
}

/// 单条规则评估记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleEvaluation {
    pub rule_name: String,
    pub passed: bool,
    pub error: Option<String>,
    pub error_code: Option<String>,
    pub blocking: bool,
    pub evaluated_at: String,
}
