//! # framework-schedule — 日程管理公共库
//!
//! WorkspaceDock 日程面板的公共模型与业务逻辑。
//! 供 Gateway 后端日程 API 使用。

pub mod models;
pub mod reminder;
pub mod service;

pub use models::*;
pub use reminder::{ScheduleReminderHandler, SCHEDULE_REMINDER_PLAN_CODE};
pub use service::{ScheduleError, ScheduleRepository, ScheduleService};
