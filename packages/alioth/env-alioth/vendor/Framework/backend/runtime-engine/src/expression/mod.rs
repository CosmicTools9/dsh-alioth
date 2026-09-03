//! 表达式解析与求值模块
//!
//! **注意**: 数据类型已迁移至 `runtime-contract` crate。
//! 本模块保留求值器实现和解析器重新导出。

pub mod evaluator;
pub mod parser;
pub mod rhai;

// 重新导出契约类型（保持向后兼容）
pub use runtime_contract::expression::*;

// 重新导出求值器
pub use evaluator::ExpressionEvaluator;
// Rhai 公式引擎（复杂公式通道）
pub use rhai::RhaiExpressionEngine;
