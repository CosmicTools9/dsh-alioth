//! # framework-inbox — 站内信公共库
//!
//! WorkspaceDock 站内信面板的公共模型与业务逻辑。
//! 供 Gateway 后端站内信 API 使用。

pub mod models;
pub mod service;

pub use models::{InboxActionResponse, SendMessageRequest};
pub use service::InboxService;
