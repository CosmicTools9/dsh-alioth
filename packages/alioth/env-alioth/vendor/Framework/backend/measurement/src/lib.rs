//! measurement — 计量业务共享内核（biz 面）
//!
//! A′ 案（consolidate-duplicated-services）收敛后的唯一实现：
//! - biz/models：业务形状 DTO（MeasurementUnit/ScalarPrice/ExchangeRate/UnitConversionRate + 请求）
//! - biz/repositories：叶表路由 INSERT + 定制 INSERT，id 依赖列默认 gen_next_uid(table_code)
//!
//! WZ/Alioth measurement 壳依赖本 crate 的 biz 面（biz_models/biz_repositories）；
//! 物理面（Rate/Scale/Unit 全字段模型 + convert/cache/date_time）已按零消费者原则移除
//! （remove-measurement-physical-surface）。

pub mod biz;

pub use biz::models as biz_models;
pub use biz::repositories as biz_repositories;

pub mod handlers;
pub mod service;

use actix_web::web;

/// 注册计量因子全部 handler（不含 scope——由调用方自持 scope 路径）
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.configure(handlers::unit::register)
        .configure(handlers::exchange_rate::register)
        .configure(handlers::scalar::register)
        .configure(handlers::unit_conversion_rate::register);
}
