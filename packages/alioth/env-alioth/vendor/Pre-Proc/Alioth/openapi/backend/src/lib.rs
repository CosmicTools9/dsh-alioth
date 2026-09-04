//! # alioth-service-openapi — OpenAPI 数据服务产品
//!
//! 企业应用与第三方系统对接的数据服务（产品）管理：
//! - 对接配置（`isahl.zc_id_prot-openapi_config`）
//! - 数据服务产品（销售/采购/制造，`zc_id_prod-openapi-*`）
//!
//! 定义与配置数据在 isahl 对应表管理（标准 CRUD + NGAC 授权 + 审计），
//! UI 位于 Gateway（跨 APP 通用能力）。

pub mod handlers;
pub mod models;
pub mod repositories;

use actix_web::web;

/// 注册 openapi 因子的全部路由。
///
/// 注意：禁止包 `web::scope("")`——空前缀 scope 会成为路由器通配节点，
/// 遮蔽其后注册的所有兄弟路由（实证：WZ 上 /api/openapi/* 全家族 404）。
/// handlers::register 内部已自带 `scope("/service/openapi")` 前缀。
pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    cfg.configure(handlers::register);
}
