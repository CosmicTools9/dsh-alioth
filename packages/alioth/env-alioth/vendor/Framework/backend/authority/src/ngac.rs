//! NgacGuard — Optional NGAC defense-in-depth seam.
//!
//! Framework identity handlers call `NgacGuard` before every CRUD operation.
//! The default implementation (`NoopNgacGuard`) performs no checks,
//! maintaining backward compatibility for namespaces without NGAC configured.
//!
//! Namespaces that opt into NGAC enable the `ngac-rls` Cargo feature,
//! which activates `RlsNgacGuard` — calling `require_resource_access` on
//! single-resource operations and extracting `visible_resource_ids` for
//! list filtering (per NGAC_SPEC.md §6-7).

use actix_web::HttpRequest;
use async_trait::async_trait;
use sqlx::PgPool;

use common::error::AliothError;

/// Seam for NGAC defense-in-depth checks.
///
/// # Default
/// `NoopNgacGuard` — all checks pass, `visible_ids` returns `None` (admin full visibility).
///
/// # With NGAC
/// Enable `ngac-rls` feature to use `RlsNgacGuard`, which:
/// - `check_access` → `common::permissions::require_resource_access`
/// - `visible_ids` → read from `RequestContext.visible_resource_ids`
#[async_trait]
pub trait NgacGuard: Send + Sync + Default {
    /// Verify the current user has `action` on `resource:{id}`.
    ///
    /// Called before create/get/update/delete. Returns `Ok(())` if permitted,
    /// `Err(AliothError::Forbidden(...))` if denied.
    async fn check_access(
        &self,
        pool: &PgPool,
        user_id: i64,
        resource: &str,
        id: i64,
        action: &str,
    ) -> Result<(), AliothError>;

    /// Return the list of resource IDs visible to the current user.
    ///
    /// Synchronous — reads from request extensions, no I/O.
    /// - `None` → admin, no filtering (show all)
    /// - `Some([])` → no visible resources (return empty list)
    /// - `Some(ids)` → apply `WHERE id = ANY($ids)` filter
    fn visible_ids(&self, req: &HttpRequest, resource: &str) -> Option<Vec<i64>>;
}

// ── NoopNgacGuard (default) ──────────────────────────────────────

/// Default no-op guard — all access permitted, no visibility filtering.
#[derive(Debug, Clone, Default)]
pub struct NoopNgacGuard;

#[async_trait]
impl NgacGuard for NoopNgacGuard {
    async fn check_access(
        &self,
        _pool: &PgPool,
        _user_id: i64,
        _resource: &str,
        _id: i64,
        _action: &str,
    ) -> Result<(), AliothError> {
        Ok(())
    }

    fn visible_ids(&self, _req: &HttpRequest, _resource: &str) -> Option<Vec<i64>> {
        None
    }
}

// ── RlsNgacGuard (ngac-rls feature) ──────────────────────────────

/// NGAC-aware guard using `require_resource_access` + `visible_resource_ids`.
///
/// Available when the `ngac-rls` Cargo feature is enabled.
#[cfg(feature = "ngac-rls")]
#[derive(Debug, Clone, Default)]
pub struct RlsNgacGuard;

#[cfg(feature = "ngac-rls")]
#[async_trait]
impl NgacGuard for RlsNgacGuard {
    async fn check_access(
        &self,
        pool: &PgPool,
        user_id: i64,
        resource: &str,
        id: i64,
        action: &str,
    ) -> Result<(), AliothError> {
        common::permissions::require_resource_access(pool, user_id, resource, id, action).await
    }

    fn visible_ids(&self, req: &HttpRequest, resource: &str) -> Option<Vec<i64>> {
        use common::context::RequestContext;
        let ctx = RequestContext::from_request(req);
        ctx.and_then(|c| c.get_visible_resource_ids(resource).cloned())
    }
}
