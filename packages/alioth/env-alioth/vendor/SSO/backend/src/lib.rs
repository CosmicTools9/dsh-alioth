pub mod admin;
pub mod audit;
pub mod auth;
pub mod config;
pub mod db;
pub mod epp;
pub mod error;
pub mod http_client;
pub mod log_email;
pub mod ngac;
pub mod scim;
pub mod websocket;

pub use auth::AuthState;
pub use config::Config;
pub use db::Database;

use actix_web::web;

/// GET /auth/mode — returns auth mode indicator
/// Frontends use this to detect standalone vs SSO auth mode.
async fn auth_mode() -> actix_web::HttpResponse {
    actix_web::HttpResponse::Ok().json(serde_json::json!({ "auth_mode": "sso" }))
}

/// Register SSO public routes (auth/* — no JWT required).
///
/// Caller MUST provide the following app_data BEFORE calling this:
/// - `web::Data<PgPool>`
/// - `web::Data<Config>`
/// - `web::Data<AuthState>`
/// - `web::Data<websocket::AppState>`
/// - `web::Data<Box<dyn common::EmailService>>`
/// - `web::Data<Box<dyn common::SmsService>>`
/// - `web::Data<auth::oauth_callback::OAuthAuthState>`
pub fn configure_public_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/auth/mode", actix_web::web::get().to(auth_mode))
        .route(
            "/.well-known/openid-configuration",
            web::get().to(auth::oidc_provider::oidc_discovery),
        )
        .service(
            web::scope("/oidc")
                .route(
                    "/authorize",
                    web::get().to(auth::oidc_provider::oidc_authorize),
                )
                .route("/token", web::post().to(auth::oidc_provider::oidc_token)),
        )
        .service(
            web::scope("/auth")
                .configure(auth::login::configure)
                .configure(auth::identity::configure_routes)
                .configure(auth::zchat::configure)
                .configure(auth::oauth::configure)
                .configure(auth::slo::configure)
                .configure(auth::reset_password::configure_routes)
                .configure(auth::password_change::configure_routes)
                .configure(auth::profile::configure_routes)
                .configure(auth::mfa_management::configure_routes)
                .configure(auth::notification_preferences::configure_routes)
                .configure(auth::social::configure)
                .configure(auth::portal::configure_routes)
                .route(
                    "/introspect",
                    web::post().to(auth::introspect::introspect_handler),
                )
                .route("/token", web::post().to(auth::token::token_handler))
                .route(
                    "/authenticate",
                    web::post().to(auth::api_key::authenticate_handler),
                )
                .configure(auth::webauthn::configure),
        )
        .service(web::scope("/scim/v2").configure(scim::configure));
}

/// Register SSO public routes without any scope wrapper.
/// Caller can wrap in any scope (e.g. `web::scope("/api/auth")`) without double-nesting.
pub fn configure_public_routes_without_scope(cfg: &mut web::ServiceConfig) {
    cfg.route("/mode", actix_web::web::get().to(auth_mode))
        .route(
            "/.well-known/openid-configuration",
            web::get().to(auth::oidc_provider::oidc_discovery),
        )
        .service(
            web::scope("/oidc")
                .route(
                    "/authorize",
                    web::get().to(auth::oidc_provider::oidc_authorize),
                )
                .route("/token", web::post().to(auth::oidc_provider::oidc_token)),
        )
        .configure(auth::login::configure)
        .configure(auth::identity::configure_routes)
        .configure(auth::zchat::configure)
        .configure(auth::oauth::configure)
        .configure(auth::slo::configure)
        .configure(auth::reset_password::configure_routes)
        .configure(auth::password_change::configure_routes)
        .configure(auth::profile::configure_routes)
        .configure(auth::mfa_management::configure_routes)
        .configure(auth::notification_preferences::configure_routes)
        .configure(auth::social::configure)
        .configure(auth::portal::configure_routes);
}

/// Register SSO protected routes (behind NgacEnforcer or RequireAuth).
///
/// These include NGAC PDP/PIP, audit log, admin API, and WebSocket audit stream.
/// All routes under `/api/ngac`, `/api/audit`, `/api/admin`, `/ws/audit`.
///
/// 安全边界（SECURITY_SPEC §3.4 豁免最小化）：
/// - `/api/ngac/*` 仅保留 PDP 决策端点（decide/check/batch/list/columns）与审计接入，
///   noauth 仅供 PEP 自调用；**PIP 不再挂载于此**（旧 `/api/ngac/pip/*` 已移除）。
/// - PIP 管理面端点挂载于 `/api/admin/ngac/pip/*`：`RequireAuth`（JWT）+ `NgacPep`
///   （sso_admin OA 决策）双重保护，与既有 `/api/admin` 路由接线一致。
pub fn configure_protected_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api/ngac")
            .configure(ngac::pdp::configure_routes)
            .route("/audit", web::post().to(audit::handlers::ingest_event)),
    )
    .service(
        web::scope("/api/admin/ngac/pip")
            .wrap(crate::auth::middleware::NgacPep::new())
            .configure(ngac::pip::configure_routes),
    )
    .service(
        web::scope("/api/audit")
            .wrap(crate::auth::middleware::NgacPep::new())
            .route("/events", web::get().to(audit::handlers::list_events))
            .route("/events/{id}", web::get().to(audit::handlers::get_event))
            .route("/stats", web::get().to(audit::handlers::get_stats))
            .route(
                "/events/cleanup",
                web::delete().to(audit::handlers::cleanup_events),
            ),
    )
    .configure(admin::configure)
    .service(web::scope("/ws").route(
        "/audit",
        web::get().to(websocket::handler::ws_audit_handler),
    ));
}

/// Full register (public + protected) — for standalone SSO or simple embedding.
///
/// 包含 `/api/auth` 前缀别名，使独立部署的 SSO 与内嵌模式提供完全对等的鉴权
/// 路由面（`/auth/*` 与 `/api/auth/*` 同时可用）。
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    configure_public_routes(cfg);
    cfg.service(web::scope("/api/auth").configure(configure_public_routes_without_scope));
    configure_protected_routes(cfg);
}

/// Build a standalone SSO HTTP server (backward compatible).
pub async fn build_server(config: Config) -> std::io::Result<actix_web::dev::Server> {
    use actix_web::{middleware::Logger, App, HttpResponse, HttpServer};

    let database = Database::new(&config)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let pool = database.pool().clone();

    let jwt_private_key = config.sso_jwt_private_key.as_bytes().to_vec();
    let jwt_public_key = auth::jwt::derive_public_key(&jwt_private_key)
        .map_err(|e| std::io::Error::other(format!("Failed to derive JWT public key: {e}")))?;
    // 轮换窗口历史公钥（可选）：kid 从 PEM 派生（不信任存储值），供 JWKS 双钥过渡。
    let jwt_public_keys_prev = match &config.sso_jwt_public_key_prev {
        Some(pem) => {
            let pem_bytes = pem.as_bytes().to_vec();
            let kid = auth::jwt::public_key_kid(&pem_bytes).map_err(|e| {
                std::io::Error::other(format!("Failed to derive prev JWT public key kid: {e}"))
            })?;
            vec![(kid, pem_bytes)]
        }
        None => vec![],
    };
    // 注入内部令牌的 iss/aud 绑定（单第一方场景：audience == issuer）。
    // 必须在签发/验证任何令牌之前完成，decode_token 会强制校验这两个声明。
    auth::jwt::configure_token_validation(config.oidc_issuer.clone(), config.oidc_issuer.clone());
    // 生产环境必须开启 Cookie Secure，否则认证 Cookie 可在明文 HTTP 下被嗅探。
    if !std::env::var("COOKIE_SECURE")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        log::error!(
            "SSO SECURITY: COOKIE_SECURE is not 'true' — auth cookies will be sent without the \
             Secure flag (plaintext HTTP sniffing risk). Set COOKIE_SECURE=true when serving \
             behind HTTPS in production."
        );
    }
    let auth_state = AuthState {
        jwt_private_key,
        jwt_public_key,
        jwt_public_keys_prev,
        encryption_key: config.encryption_key.as_bytes().to_vec(),
        jwt_access_expiry_secs: config.jwt_access_expiry,
        jwt_refresh_expiry_secs: config.jwt_refresh_expiry,
        identity_verify_mode: config.identity_verify_mode.clone(),
        identity_external_verify_url: config.identity_external_verify_url.clone(),
        ngac_preview_dir: config.ngac_preview_dir.clone(),
    };

    let server_addr = config.server_addr.clone();
    let ws_app_state = web::Data::new(websocket::AppState::new());

    log::info!("SSO Service listening on {}", server_addr);

    let server = HttpServer::new(move || {
        let cors = common::build_cors().expect("Failed to build CORS config");
        App::new()
            .wrap(cors)
            .wrap(Logger::default())
            .wrap(auth::middleware::RequireAuth::new())
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .app_data(ws_app_state.clone())
            .app_data(web::Data::new({
                // 邮件服务按 SSO_EMAIL_MODE 选择（config.rs fail-closed：
                // 未设置/非法值落回 smtp，生产不允许隐式降级 log）。
                let svc: Box<dyn common::EmailService> = match config.email_mode.as_str() {
                    "log" => Box::new(log_email::LogEmailService),
                    _ => Box::new(common::SmtpEmailService::new(pool.clone())),
                };
                svc
            }))
            .app_data(web::Data::new(
                Box::new(common::CloudSmsService::new(pool.clone())) as Box<dyn common::SmsService>,
            ))
            .app_data(web::Data::new(auth::oauth_callback::OAuthAuthState {
                jwt_private_key: auth_state.jwt_private_key.clone(),
                jwt_public_key: auth_state.jwt_public_key.clone(),
                jwt_access_expiry_secs: auth_state.jwt_access_expiry_secs,
                jwt_refresh_expiry_secs: auth_state.jwt_refresh_expiry_secs,
            }))
            .route(
                "/health",
                web::get()
                    .to(|| async { HttpResponse::Ok().json(serde_json::json!({"status": "ok"})) }),
            )
            .route("/.well-known/jwks.json", web::get().to(auth::jwt::jwks))
            .configure(configure_routes)
    })
    .backlog(1024)
    .bind(&server_addr)
    .map_err(|e| common::server::bind_error(&server_addr, e))?
    .run();

    Ok(server)
}
