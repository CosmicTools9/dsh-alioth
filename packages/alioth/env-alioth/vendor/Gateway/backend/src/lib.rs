#![allow(unexpected_cfgs)]

pub mod api;
pub mod apps;
pub mod config;
pub mod db;
pub mod epp;
pub mod errors;
pub mod i18n;
pub mod models;
pub mod monitoring;
pub mod namespace_schema;
pub mod ngac;
pub mod notification;
pub mod openapi;
pub mod pep;
pub mod schedule;
/// 跨 namespace 通用种子自愈组件（add-gateway-seed-self-heal）
pub mod seed;
pub mod service_registry;
pub mod system_config_repo;
pub mod trigger_crud;

pub use config::Config;
pub use db::Database;
pub use encoding::zuid_init::{init_zuid_function, validate_zuid_config, ZuidConfig};
pub use errors::{AppError, Result};
pub use monitoring::Metrics;
