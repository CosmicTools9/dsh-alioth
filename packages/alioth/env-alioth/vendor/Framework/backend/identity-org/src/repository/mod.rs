//! identity-org 数据访问层（split 自 repository.rs 单体，④ 候选：一实体一文件）
//!
//! 域辅助项（enum/const/fn）随所属实体归文件；对外路径经 glob 再导出保持 crate::repository::X。
//! 身份实体 Repository — 标准 CRUD 实现
//!
//! Identity 使用自定义 Repository，其余实体组合 GenericRepository，
//! 仅自定义 create/update 的 INSERT/UPDATE SQL。

pub mod bill_check;
pub mod consignment;
pub mod contract;
pub mod deta_bill_check;
mod drift_guard;
pub mod environment;
pub mod fence;
pub mod freight_product;
pub mod identity;
pub mod inventory_sales;
pub mod invoice;
pub mod invoice_detail;
pub mod license;
pub mod natural_person;
mod ontology_binding;
pub mod payment;
pub mod pricing_agreement;
pub mod seal;
pub mod settlement_bank;
pub mod settlement_cash;
pub mod settlement_channel;
#[cfg(test)]
mod tests;
pub mod trade_order;
pub mod traffic_line;
pub mod transit_route;
pub mod transport_tracking;
pub mod vehicle;

pub use bill_check::*;
pub use consignment::*;
pub use contract::*;
pub use deta_bill_check::*;
pub use environment::*;
pub use fence::*;
pub use freight_product::*;
pub use identity::*;
pub use inventory_sales::*;
pub use invoice::*;
pub use invoice_detail::*;
pub use license::*;
pub use natural_person::*;
pub use payment::*;
pub use pricing_agreement::*;
pub use seal::*;
pub use settlement_bank::*;
pub use settlement_cash::*;
pub use settlement_channel::*;
pub use trade_order::*;
pub use traffic_line::*;
pub use transit_route::*;
pub use transport_tracking::*;
pub use vehicle::*;
