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
pub mod extension_surface;

// 重新导出运行时契约类型（保持向后兼容）
pub use runtime_contract::behavior::*;
pub use runtime_contract::extension::*;
pub use runtime_contract::model_registry::*;

// 重新导出上下文类型
pub use context::{RuleContext, RuleEvaluation, RuleEvaluationResult, RuleOperation};

// 重新导出引擎类型
pub use engine::constraint::{
    ConstraintConfig, ConstraintEngine, ConstraintResult, ConstraintValidationResult,
};
pub use engine::expression::{ExpressionEngine, ExpressionError};
pub use engine::rule::{
    rule_dependency_edges, BusinessRuleConfig, ConflictPolicy, RuleEngine, RuleExecution,
    RuleExecutionResult, SaturationConfig,
};

// 重新导出扩展运行时类型
pub use extension::{
    collect_expression_advisories, collect_expression_violations, solve_warnings,
    AppExtensionRegistry, ExtensionLoader, ExtensionRuntimeError,
};
// 扩展执行结果（定义在 runtime-contract；此处转发，使 crud 等消费方无需直接依赖契约 crate）
pub use extension_surface::{
    ConstraintDecl, DeclarationInventory, ExtensionSurface, RuleDecl, StateMachineDecl,
    TransitionDecl, WorkflowDecl,
};
pub use runtime_contract::extension::ExtensionResult;

// 重新导出表达式引擎（平台唯一实现）与静态分析
pub use expression::{
    and_atoms, collect_variables, is_truthy, plan_face_expressions, plan_face_violations, CmpOp,
    ComparisonAtom, RhaiExpressionEngine,
};
