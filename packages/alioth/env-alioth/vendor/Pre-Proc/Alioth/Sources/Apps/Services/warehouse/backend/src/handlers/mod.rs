//! 聚合所有实体 handler 的注册函数（BACKEND_FRAMEWORK §7.1.2）

pub mod warehouse;
pub mod warehouse_location;

pub fn register(cfg: &mut actix_web::web::ServiceConfig) {
    warehouse::register(cfg);
    warehouse_location::register(cfg);
}
