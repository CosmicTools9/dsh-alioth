//! 聚合所有实体 handler 的注册函数（BACKEND_FRAMEWORK §7.1.2）

pub mod inbound_bom;
pub mod inbound_order;

pub fn register(cfg: &mut actix_web::web::ServiceConfig) {
    inbound_order::register(cfg);
    inbound_bom::register(cfg);
}
