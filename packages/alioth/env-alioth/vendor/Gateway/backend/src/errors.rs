//! Unified application error handling for Gateway backend.
//!
//! Gateway 统一使用 common::AliothError 作为错误类型。

pub use common::{AliothError as AppError, Result};
