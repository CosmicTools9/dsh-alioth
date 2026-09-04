//! 聚合所有实体 handler 的注册函数（BACKEND_FRAMEWORK §7.1.2）

pub mod stock_count;
pub mod stock_count_detail;
pub mod stock_count_status;

pub fn register(cfg: &mut actix_web::web::ServiceConfig) {
    stock_count::register(cfg);
    stock_count_detail::register(cfg);
    stock_count_status::register(cfg);
}
