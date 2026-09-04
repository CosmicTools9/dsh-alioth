//! # factor-status — 状态机因子
//!
//! Alioth 标准库: 状态管理与流转。基于 `zc_id_status` 的 `flag` 字段
//! (`start`/`doing`/`end`) 实现标准状态机。
//!
//! ## 参考 identity 因子获取完整实现模板
//! - models.rs: 状态实体 DTO + AliothEntity trait
//! - repositories.rs: AliothRepository 实现
//! - services.rs: 状态转换业务逻辑
//! - handlers/: HTTP handler + 路由

pub mod handlers;
pub mod seed;

use actix_web::web;

pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/service/status").configure(handlers::status::register));
}
