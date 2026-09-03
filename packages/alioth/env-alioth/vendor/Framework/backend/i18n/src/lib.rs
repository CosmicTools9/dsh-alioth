//! AliothStudio backend i18n core
//!
//! Provides locale parsing, dictionary management, and message formatting
//! for Rust backend services.

pub mod error;
pub mod locale;
pub mod manager;

pub use error::I18nError;
pub use locale::{parse_accept_language, resolve_locale, Locale};
pub use manager::{Dictionary, I18nManager};
