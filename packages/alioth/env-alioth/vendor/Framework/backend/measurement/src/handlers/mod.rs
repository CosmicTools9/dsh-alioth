//! 计量因子 Handler 集合（共享内核路由）
//!
//! 统一实现 units/exchange-rates/scalars/conversion-rates 的完整 CRUD：
//! - RLS 读（visible_ids + authorized_columns）与 NGAC 写（require_resource_access + dk_ctx）
//! - units 附带 multiplier 换算组装 + dimension_color 补色（Alioth 契约能力，全局可用）
//! - exchange-rates 附带字符串货币代码解析（left_currency/right_currency 输入）
//! - scalars 附带标量创建领域事件发布
//!
//! ns 壳仅注册 scope 前缀；本模块为全部 ns 的单一实现。

pub mod exchange_rate;
pub mod scalar;
pub mod unit;
pub mod unit_conversion_rate;
