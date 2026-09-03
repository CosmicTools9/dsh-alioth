//! Alioth 运行时引擎
//!
//! 提供表达式解析、求值、约束验证和业务规则执行能力。
//! 原位于 meta-services::runtime，现下沉至 Framework 作为平台级共享基础设施。
//!
//! **注意**: 所有数据类型已迁移至 `runtime-contract` crate。
//! 本 crate 仅保留求值器、引擎和运行时实现。

pub mod behavior;
pub mod context;
pub mod engine;
pub mod expression;
pub mod extension;
pub mod swrl;

// 重新导出运行时契约类型（保持向后兼容）
pub use runtime_contract::behavior::*;
pub use runtime_contract::expression::*;
pub use runtime_contract::extension::*;
pub use runtime_contract::model_registry::*;
pub use runtime_contract::swrl::*;

// 重新导出上下文类型
pub use context::{RuleContext, RuleEvaluation, RuleEvaluationResult, RuleOperation};

// 重新导出引擎类型
pub use engine::constraint::{
    ConstraintConfig, ConstraintEngine, ConstraintResult, ConstraintValidationResult,
};
pub use engine::expression::{ExpressionEngine, ExpressionError};
pub use engine::rule::{BusinessRuleConfig, RuleEngine, RuleExecution, RuleExecutionResult};

// 重新导出扩展运行时类型
pub use extension::{AppExtensionRegistry, ExtensionLoader, ExtensionRuntimeError};

// 重新导出求值器和解析器
pub use expression::evaluator::ExpressionEvaluator;
pub use expression::parser::parse_constraint_expression;
pub use expression::rhai::RhaiExpressionEngine;
pub use swrl::evaluator::{SwrlEvaluation, SwrlEvaluationResult, SwrlRuleEngine};
pub use swrl::parser::parse_swrl_rule;
