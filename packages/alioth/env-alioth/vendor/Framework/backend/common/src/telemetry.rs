//! Framework::telemetry — 业务模块的日志与遥测统一入口
//!
//! # 用法
//!
//! 业务模块统一通过本模块引用日志宏，**不直接依赖 `tracing` crate**。
//!
//! ```rust,ignore
//! use common::telemetry::{info, warn, error, trace_span};
//!
//! trace_span!("order_create", {
//!     info!("creating order");
//!     // ... 业务逻辑
//! });
//! ```

// 重新导出 log crate 宏——业务代码统一通过此入口
pub use log::{debug, error, info, log, log_enabled, trace, warn};

/// 语义化 span 宏：标记一段代码块的开始和结束。
///
/// 内部委托给 `log::debug!`，span 边界清晰可见。
#[macro_export]
macro_rules! trace_span {
    ($name:expr, { $($body:tt)* }) => {{
        log::debug!(target: "telemetry::span", "span_start: {}", $name);
        let _result = { $($body)* };
        log::debug!(target: "telemetry::span", "span_end: {}", $name);
        _result
    }};
}

// 让 trace_span! 也可通过 `common::telemetry::trace_span` 访问
pub use crate::trace_span;
