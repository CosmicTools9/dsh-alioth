//! 聚合所有实体 handler 的注册函数（BACKEND_FRAMEWORK §7.1.2）

pub mod account;
pub mod ledger_entry;
pub mod subject_account;

pub fn register(cfg: &mut actix_web::web::ServiceConfig) {
    ledger_entry::register(cfg);
    account::register(cfg);
    subject_account::register(cfg);
}
