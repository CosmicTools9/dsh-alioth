//! # Post-approval hook trait
//!
//! Defines the `ApprovalHook` trait for plugging domain-specific
//! side effects into the approval lifecycle (e.g., user activation,
//! NGAC attribute assignment after identity verification).
//!
//! Implementors are registered at the call site (`execute`'s `hook`
//! parameter) — the trait lives here so domain crates (gateway-sso,
//! product-approval-hook, etc.) can implement it without coupling
//! to `framework-workspace-approval` internals.

use async_trait::async_trait;
use sqlx::PgPool;

/// Post-approval hook for domain-specific side effects.
///
/// Called after a successful status transition in `ApprovalService::execute`.
/// The approval status has already been written to the database before
/// the hook runs, so hook failures do not roll back the approval.
///
/// Implementors MUST log their own errors — the caller treats the hook
/// as best-effort and does not propagate failures.
#[async_trait]
pub trait ApprovalHook: Send + Sync {
    /// Invoked after an approval state change.
    ///
    /// # Arguments
    /// - `pool` — database connection
    /// - `approval_id` — the approval event id
    /// - `status_code` — `"approved"` or `"rejected"` (others possible via direct inserts)
    async fn on_approval(&self, pool: &PgPool, approval_id: i64, status_code: &str);
}
