//! EPP (Event Processing Point) 模块 - 审计事件记录
//!
//! 实现已下沉至 `common::audit`（供 Framework crate 与 Gateway 共用），
//! 本模块仅 re-export，保持 Gateway 既有调用面兼容。

pub use common::audit::{record_audit_event, AuditError, AuditEvent, Decision};
