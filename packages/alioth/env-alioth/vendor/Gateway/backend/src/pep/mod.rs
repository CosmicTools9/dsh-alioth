//! PEP (Policy Enforcement Point) 模块 - 简化版
//!
//! 负责拦截请求并执行 NGAC 访问决策

pub mod cache;
pub mod jwks;
pub mod middleware;

// 内部 seam（评审候选 5，2026-10-10）：`middleware.rs` 六大关注点各成模块。
// 仅 `pep` 内部消费（`pub(super)`/re-export），外 interface = `NgacEnforcer` 不变。
mod audit;
mod jwt;
mod obligations;
mod pdp_client;
mod resource_map;
mod session;

pub use cache::{ColumnCache, VersionProbe};
pub use middleware::NgacEnforcer;
