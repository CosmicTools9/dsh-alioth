//! # alioth-service-openapi — OpenAPI 数据服务产品（组合薄壳）
//!
//! 实现单源 = Framework `openapi` 内核（模型数据链路基础设施；见
//! `openspec/specs/framework-scope-boundary` R1 ② 类 / R2「ns 后端 MUST 为组合薄壳」）。
//! 企业应用与第三方系统对接的数据服务（产品）管理：对接配置（`isahl.zc_id_prot-openapi_config`）
//! 与数据服务产品（`zc_id_prod-openapi-*`）；UI 位于 Gateway（跨 APP 通用能力）。
//! 单源判定：change extract-ns-service-kernels-demand-openapi。
//!
//! 注意：禁止包 `web::scope("")`——空前缀 scope 会成为路由器通配节点，
//! 遮蔽其后注册的所有兄弟路由（实证：WZ 上 /api/openapi/* 全家族 404）。
//! 内核 `handlers::register` 内部已自带 `scope("/service/openapi")` 前缀。

use actix_web::web;

/// 注册 openapi 因子的全部路由（委托内核）。
pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    openapi::register_service_routes(cfg);
}
