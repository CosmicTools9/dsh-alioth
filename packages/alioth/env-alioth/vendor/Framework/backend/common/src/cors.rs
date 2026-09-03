//! CORS configuration helper
//!
//! Reads `ALLOWED_ORIGINS` environment variable to configure CORS.
//! Format: comma-separated origins, or `*` for any origin (dev only, incompatible with credentials).
//! - `ALLOWED_ORIGINS=*` → permissive mode (only matches localhost origins with credentials)
//! - `ALLOWED_ORIGINS=http://a.com,http://b.com` → allow listed origins only
//!
//! # Usage
//! ```rust
//! std::env::set_var("ALLOWED_ORIGINS", "http://localhost:3000");
//! use common::build_cors;
//! let cors = build_cors().expect("ALLOWED_ORIGINS must be set");
//! ```
//!

use actix_cors::Cors;
use actix_web::http::header;

fn is_localhost_origin(origin: &str) -> bool {
    origin.starts_with("http://localhost:")
        || origin.starts_with("https://localhost:")
        || origin.starts_with("http://127.0.0.1:")
        || origin.starts_with("https://127.0.0.1:")
        || origin.starts_with("http://0.0.0.0:")
        || origin.starts_with("https://0.0.0.0:")
}

/// Build CORS configuration from environment.
pub fn build_cors() -> Result<Cors, String> {
    let allowed_origins = std::env::var("ALLOWED_ORIGINS")
        .map_err(|e| format!("ALLOWED_ORIGINS must be set: {}", e))?;

    let mut cors = Cors::default()
        .allowed_methods(vec!["GET", "POST", "PUT", "DELETE", "PATCH", "OPTIONS"])
        .allowed_headers(vec![
            header::AUTHORIZATION,
            header::ACCEPT,
            header::CONTENT_TYPE,
        ])
        .supports_credentials()
        .max_age(3600);

    if allowed_origins.trim() == "*" || allowed_origins.trim().is_empty() {
        // allow_any_origin() is incompatible with supports_credentials() per CORS spec.
        // Use a permissive origin function for localhost dev instead.
        cors = cors.allowed_origin_fn(|origin, _req_head| {
            is_localhost_origin(origin.to_str().unwrap_or(""))
        });
    } else {
        for origin in allowed_origins.split(',') {
            let origin = origin.trim();
            if !origin.is_empty() {
                cors = cors.allowed_origin(origin);
            }
        }
    }

    Ok(cors)
}
