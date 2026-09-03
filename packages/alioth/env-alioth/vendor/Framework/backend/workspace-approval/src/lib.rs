//! # framework-workspace-approval — 工作区审批公共库
//!
//! WorkspaceDock 审批面板的 approve/reject 业务逻辑。
//! 基于 zc_id_stus-approve + zc_id_lifecycle_r_primary-status 的状态切换。

pub mod hook;
pub mod models;
pub mod service;

pub use hook::ApprovalHook;
pub use models::{ApprovalActionResponse, ApprovalActor};
pub use service::ApprovalService;
