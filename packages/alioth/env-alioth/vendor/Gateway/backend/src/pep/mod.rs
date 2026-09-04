//! PEP (Policy Enforcement Point) 模块 - 简化版
//!
//! 负责拦截请求并执行 NGAC 访问决策

pub mod cache;
pub mod jwks;
pub mod middleware;

pub use cache::{ColumnCache, VersionProbe};
pub use middleware::NgacEnforcer;
