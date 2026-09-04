//! # factor-version — 版本控制因子
//!
//! Alioth 标准库：实体版本管理。基于 `isahl.zc_id_version` 实现创建版本、版本链追溯。
//!
//! ## 核心模式
//! - **版本链**: `fk_previous` 链构建版本快照
//!
//! ## 路由
//! - `GET/POST    /api/service/version/versions` — 版本列表/创建
//! - `GET/PUT/DELETE /api/service/version/versions/{id}` — 版本详情/更新/删除

pub mod handlers;

pub mod seed;

use actix_web::web;

/// 注册 version 因子的全部路由
pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/service/version").configure(handlers::version::register));
}
