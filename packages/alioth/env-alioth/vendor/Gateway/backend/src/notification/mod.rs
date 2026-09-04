//! Gateway 通知中心 — 数据变更订阅与站内信推送
//!
//! 提供用户订阅管理 API 和触发器 after 链路通知能力。

pub mod db_messaging;
pub mod handlers;
pub mod models;
pub mod repository;
pub mod service;

pub use handlers::configure_routes;
pub use service::NotificationService;
