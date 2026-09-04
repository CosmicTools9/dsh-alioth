//! Audit module
//!
//! Provides audit logging functionality for NGAC events.
//! Stores audit records and provides query capabilities.

pub mod handlers;
pub mod queries;

pub use handlers::DbAuditEvent;
pub use handlers::{AuditEventRecord, AuditEventType};
pub use queries::{
    get_events_by_decision, get_events_by_object, get_events_by_user, get_failed_access_events,
    search_events, EventQuery,
};
