//! SSO Admin API
//!
//! Admin-only endpoints for managing users, NGAC attributes, identity verifications,
//! and identity providers. All handlers enforce admin NGAC user attribute check.

pub mod api_clients;
pub mod handlers;
pub mod oidc_clients;
pub mod plans;
pub mod subscriptions;

use actix_web::web;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api/admin")
            .wrap(crate::auth::middleware::NgacPep::new())
            // ─── Bootstrap（种子自检：system 用户主体绑定）──────────
            .route(
                "/bootstrap/system-subject",
                web::get().to(handlers::get_system_subject_status),
            )
            .route(
                "/bootstrap/system-subject",
                web::post().to(handlers::bind_system_subject),
            )
            // ─── Users ───────────────────────────────────────────────
            .route("/users", web::get().to(handlers::list_users))
            .route("/users", web::post().to(handlers::create_user))
            .route("/users/{id}", web::get().to(handlers::get_user))
            .route("/users/{id}", web::put().to(handlers::update_user))
            // DELETE on /users/{id} → soft-disable (preserves audit history)
            .route("/users/{id}", web::delete().to(handlers::disable_user))
            .route("/users/{id}/enable", web::post().to(handlers::enable_user))
            .route(
                "/users/{id}/reset-password",
                web::post().to(handlers::admin_reset_password),
            )
            .route(
                "/users/{id}/unlock",
                web::post().to(handlers::admin_unlock_account),
            )
            // ─── User attributes (NGAC roles) ───────────────────────
            .route(
                "/users/{id}/attributes",
                web::get().to(handlers::list_user_attributes),
            )
            .route(
                "/users/{id}/attributes/bind",
                web::post().to(handlers::bind_user_attribute),
            )
            .route(
                "/users/{id}/attributes/{ua_id}",
                web::delete().to(handlers::unbind_user_attribute),
            )
            .route(
                "/user-attributes",
                web::get().to(handlers::list_all_user_attributes),
            )
            .route(
                "/user-attributes",
                web::post().to(handlers::create_user_attribute),
            )
            .route(
                "/user-attributes/{id}",
                web::put().to(handlers::update_user_attribute),
            )
            .route(
                "/user-attributes/{id}",
                web::delete().to(handlers::delete_user_attribute),
            )
            // ─── Object attributes (NGAC resource labels) ────────────
            .route(
                "/object-attributes",
                web::get().to(handlers::list_object_attributes),
            )
            .route(
                "/object-attributes",
                web::post().to(handlers::create_object_attribute),
            )
            .route(
                "/object-attributes/{id}",
                web::put().to(handlers::update_object_attribute),
            )
            .route(
                "/object-attributes/{id}",
                web::delete().to(handlers::delete_object_attribute),
            )
            // ─── Identity verifications ──────────────────────────────
            .route(
                "/identity-verifications",
                web::get().to(handlers::list_identity_verifications),
            )
            .route(
                "/identity-verifications/{id}/approve",
                web::post().to(handlers::approve_identity_verification),
            )
            .route(
                "/identity-verifications/{id}/reject",
                web::post().to(handlers::reject_identity_verification),
            )
            // ─── Identity providers ─────────────────────────────────
            .route("/providers", web::get().to(handlers::list_providers))
            .route("/providers", web::post().to(handlers::create_provider))
            .route("/providers/{id}", web::get().to(handlers::get_provider))
            .route("/providers/{id}", web::put().to(handlers::update_provider))
            .route(
                "/providers/{id}",
                web::delete().to(handlers::delete_provider),
            )
            .route(
                "/providers/{id}/toggle",
                web::post().to(handlers::toggle_provider),
            )
            .route(
                "/providers/{id}/test",
                web::post().to(handlers::test_provider),
            )
            // ─── Sessions ───────────────────────────────────────────
            .route("/sessions", web::get().to(handlers::list_user_sessions))
            .route(
                "/sessions/{token}",
                web::delete().to(handlers::revoke_user_session),
            )
            // ─── OIDC Clients ──────────────────────────────────────
            .route(
                "/oidc/clients",
                web::get().to(oidc_clients::list_oidc_clients),
            )
            .route(
                "/oidc/clients",
                web::post().to(oidc_clients::create_oidc_client),
            )
            .route(
                "/oidc/clients/{id}",
                web::put().to(oidc_clients::update_oidc_client),
            )
            .route(
                "/oidc/clients/{id}",
                web::delete().to(oidc_clients::delete_oidc_client),
            )
            // ─── OpenAPI 统一调用方注册表（api_clients） ─────────────
            .route("/api-clients", web::get().to(api_clients::list_api_clients))
            .route(
                "/api-clients",
                web::post().to(api_clients::create_api_client),
            )
            .route(
                "/api-clients/{id}",
                web::put().to(api_clients::update_api_client),
            )
            .route(
                "/api-clients/{id}",
                web::delete().to(api_clients::delete_api_client),
            )
            .route(
                "/api-clients/{id}/rotate-secret",
                web::post().to(api_clients::rotate_api_client_secret),
            )
            // ─── OpenAPI 套餐 / 订阅 / 账单（P4 商业化） ─────────────
            .route("/api-plans", web::get().to(subscriptions::list_plans))
            .route("/api-plans", web::post().to(plans::create_plan))
            .route("/api-plans/{id}", web::put().to(plans::update_plan))
            .route("/api-plans/{id}", web::delete().to(plans::delete_plan))
            .route(
                "/api-subscriptions",
                web::get().to(subscriptions::list_subscriptions),
            )
            .route(
                "/api-subscriptions",
                web::post().to(subscriptions::create_subscription),
            )
            .route(
                "/api-subscriptions/{id}",
                web::put().to(subscriptions::update_subscription),
            )
            .route(
                "/api-subscriptions/{id}/plan",
                web::put().to(subscriptions::change_plan),
            )
            .route(
                "/api-subscriptions/{id}/status",
                web::post().to(subscriptions::set_status),
            )
            .route(
                "/api-reconcile",
                web::get().to(subscriptions::reconcile_export),
            )
            // ─── NGAC access policies (associations) ────────────────
            .route(
                "/ngac/access-rights",
                web::get().to(handlers::list_access_rights),
            )
            .route(
                "/ngac/policy-classes",
                web::get().to(handlers::list_policy_classes),
            )
            .route(
                "/ngac/associations",
                web::get().to(handlers::list_associations),
            )
            .route(
                "/ngac/associations",
                web::post().to(handlers::create_association),
            )
            .route(
                "/ngac/associations/{id}",
                web::put().to(handlers::update_association),
            )
            .route(
                "/ngac/associations/{id}",
                web::delete().to(handlers::delete_association),
            )
            // ─── NGAC prohibitions（禁止规则，deny） ────────────────
            .route(
                "/ngac/prohibitions",
                web::get().to(handlers::list_prohibitions),
            )
            .route(
                "/ngac/prohibitions",
                web::post().to(handlers::create_prohibition),
            )
            .route(
                "/ngac/prohibitions/{id}",
                web::put().to(handlers::update_prohibition),
            )
            .route(
                "/ngac/prohibitions/{id}",
                web::delete().to(handlers::delete_prohibition),
            )
            // ─── NGAC policy matrix（矩阵投影，只读） ───────────────
            .route("/ngac/matrix", web::get().to(handlers::get_policy_matrix))
            // ─── NGAC access review（主体/资源双中心，只读） ───────────
            .route(
                "/ngac/review/user/{user_id}",
                web::get().to(handlers::get_user_access_review),
            )
            .route(
                "/ngac/review/resource",
                web::get().to(handlers::get_resource_access_review),
            )
            // ─── NGAC 审计轨迹 + 删除影响预览（只读） ───────────────
            .route("/ngac/audit-log", web::get().to(handlers::get_audit_log))
            .route(
                "/ngac/impact-preview",
                web::get().to(handlers::get_impact_preview),
            )
            // ─── OA 页面预览静态文件（add-ngac-oa-preview，dev-only） ──
            .route(
                "/ngac/previews/{filename:.*}",
                web::get().to(handlers::get_ngac_preview_file),
            )
            // ─── NGAC 图快照（refactor-ngac-admin-nl-graph，只读聚合） ──
            .route("/ngac/graph", web::get().to(handlers::get_ngac_graph)),
    )
    // 权限申请管理端点（add-ngac-access-request D3，require_admin）
    .configure(crate::ngac::access_request::configure_admin_routes)
    // 绑定申请管理端点（add-ngac-binding-request D2，require_admin）
    .configure(crate::ngac::binding_request::configure_admin_routes)
    // 组织规范资产端点（ngac-org-phase-d1，D-1 org-policy-assets；require_admin）
    .configure(crate::ngac::org_policy::configure_admin_routes);
}
