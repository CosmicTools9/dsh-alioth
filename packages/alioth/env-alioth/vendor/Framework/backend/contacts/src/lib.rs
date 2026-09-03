//! # framework-contacts — 全局通讯录公共库
//!
//! WorkspaceDock 联系人面板的数据查询逻辑。
//! 聚合链: zc_id_entity → zc_id_entity_rr_contacts → zc_id_contacts
//!                     → zc_id_contacts_rr_infos → zc_id_contact_infos
//! 叶表: zc_id_info-email, zc_id_info-telephone

pub mod models;
pub mod service;

pub use models::{
    ContactInfo, ContactInfoValue, ContactsEntity, CreateContactRequest, UpdateContactRequest,
};
pub use service::ContactsService;
