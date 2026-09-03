//! identity-org — 主体/组织/身份共享内核（extract-identity-org-core）
//!
//! isahl 全局主体/组织/身份模型（跨 ns 一致，无 namespace 语义）：
//! - identities：主体资质证照（zc_id_identity/entity_rr_identity/cate-identity/segm-date）
//! - org_tree / subjects：后续域分批上移
//!
//! WZ 业务叶表查询（view_tags/stor-ctn-vehicle 等）保留 WZ 壳。

pub mod handlers;
pub mod models;
pub mod repository;
pub mod service;

pub use handlers::identities::configure_identities;
pub use handlers::identity::register as register_identity;
