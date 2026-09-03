pub mod crc32;
pub mod handlers;
pub mod models;
pub mod rules;
pub mod serial;
pub mod service;
pub mod zuid;
pub mod zuid_init;

pub use handlers::configure_encoding_routes;
pub use serial::SerialGenerator;
pub use service::EncodingService;
pub use zuid::{PeerType, ZuidError, ZuidGenerator};
pub use zuid_init::{init_zuid_function, validate_zuid_config, ZuidConfig};

use actix_web::web;

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    handlers::configure_encoding_routes(cfg);
}
