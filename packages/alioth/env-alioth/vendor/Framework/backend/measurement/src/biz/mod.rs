//! biz 面——计量业务形状共享内核（consolidate-duplicated-services A′）
//!
//! 从 WZ/Alioth measurement 生产实现提取的唯一实现来源：
//! - models：业务形状 DTO（MeasurementUnit/ScalarPrice/ExchangeRate/UnitConversionRate + 请求）
//! - repositories：叶表路由 INSERT（unit/rate 量纲）、scalar/exchange 定制 INSERT，
//!   id 依赖列默认 gen_next_uid(table_code)（MUST NOT gen_next_zuid）

pub mod models;
pub mod repositories;
