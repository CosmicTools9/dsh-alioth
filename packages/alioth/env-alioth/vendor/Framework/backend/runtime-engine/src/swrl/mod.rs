//! SWRL (Semantic Web Rule Language) 规则引擎
//!
//! **注意**: 数据类型已迁移至 `runtime-contract` crate。
//! 本模块保留求值器实现和解析器重新导出。

pub mod evaluator;
pub mod parser;

// 重新导出契约类型（保持向后兼容）
pub use runtime_contract::swrl::*;
