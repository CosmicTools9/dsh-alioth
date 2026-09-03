//! Alioth 运行时行为接口契约
//!
//! 仅包含 trait 定义和纯数据类型，无实现、无数据库依赖、无复杂业务逻辑。
//! 由 `runtime-engine` 实现这些契约，`alioth-gen` 仅依赖本 crate 生成代码。

pub mod behavior;
pub mod expression;
pub mod extension;
pub mod model_registry;
pub mod swrl;

// Re-export behavior types
pub use behavior::*;

// Re-export expression types
pub use expression::*;

// Re-export extension types
pub use extension::*;

// Re-export model registry types
pub use model_registry::*;

// Re-export SWRL types
pub use swrl::*;
