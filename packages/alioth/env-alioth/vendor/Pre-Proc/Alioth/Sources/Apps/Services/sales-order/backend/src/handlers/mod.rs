//! 聚合所有实体 handler 的注册函数（BACKEND_FRAMEWORK §7.1.2）

pub mod sales_order;

pub fn register(cfg: &mut actix_web::web::ServiceConfig) {
    sales_order::register(cfg);
}
