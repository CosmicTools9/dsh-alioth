pub mod handlers;
pub mod models;
pub mod repositories;
pub mod seed;

use actix_web::web;

pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/service/language").configure(handlers::language::register));
}
