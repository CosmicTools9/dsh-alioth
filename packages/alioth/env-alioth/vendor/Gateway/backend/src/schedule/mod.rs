//! Schedule HTTP API — Gateway thin adapter
//!
//! 路由前缀: /api/schedule
//! 业务逻辑在 framework-schedule crate 中。

pub mod handlers;

pub use handlers::configure_routes;
