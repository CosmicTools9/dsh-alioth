//! SSO（`gateway-sso`）——认证/NGAC/SCIM/admin 服务库。
//!
//! clippy 豁免（crate 级，documented）：`result_large_err` —— 本 crate 的 handler 辅助函数
//! 统一以 `Result<T, HttpResponse>` 返回「错误即响应」形态（NGAC 决策校验 / SCIM 令牌 /
//! admin 鉴权 / WebAuthn / MFA 等 18 处）；`HttpResponse` 天然 >128B，boxing 只为消 lint
//! 会引入无收益分配并迫使 100+ 调用点解引用（本 crate 调用点均显式 match，无 `?` 传播），
//! 语义与可读性双输。Gateway 侧同类 lint 已按 Box + 解引用真修
//! （`Gateway/backend/src/{api/doc_recognition.rs,api/entity_binding.rs,standalone_auth/mod.rs}`）。
#![allow(clippy::result_large_err)]

pub mod admin;
pub mod audit;
pub mod auth;
pub mod config;
pub mod db;
pub mod epp;
pub mod error;
pub mod facade;
pub mod housekeeping;
pub mod http_client;
pub mod log_email;
pub mod ngac;
pub mod scim;
pub mod websocket;

pub use auth::AuthState;
pub use config::Config;
pub use db::Database;

use actix_web::web;

/// GET /auth/mode — returns auth mode indicator + deployment capabilities
/// Frontends use this to detect standalone vs SSO auth mode.
///
/// `email_code_login` = 部署级邮箱验证码登录开关（env `SSO_EMAIL_CODE_LOGIN`），
/// 与 `POST /auth/login/email-code` 的门禁同源；前端以该字段为渲染判据
/// （字段缺失/取数失败 ⇒ 不渲染入口）。standalone 形态的 `/auth/mode`
/// （`Gateway/host/src/standalone_auth`）同形携带该字段且恒 false。
async fn auth_mode(config: web::Data<Config>) -> actix_web::HttpResponse {
    actix_web::HttpResponse::Ok().json(serde_json::json!({
        "auth_mode": "sso",
        "email_code_login": config.email_code_login_enabled,
    }))
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
        .service(web::scope("/auth").configure(configure_auth_face))
        .service(web::scope("/scim/v2").configure(scim::configure));
}

/// 认证面的**面内路由集**（相对路径）——`/auth/*` 与 `/api/auth/*` 两前缀的**唯一来源**。
///
/// 为什么单源：两面此前各写一遍该清单，于是「新增路由只加进一面」的缝隙无人可见——实测两例
/// （webauthn 六路由，change `wire-sso-webauthn-passkey`；`/introspect`、`/token`
/// （zchat `client_credentials`）、`/authenticate`（api_key），change
/// `unify-sso-auth-route-faces`）。两面 MUST 只经本函数取得面内路由，MUST NOT 再各自展开清单。
/// 回归锁定 = `tests/sso_functional_coverage_test.rs::public_route_faces_expose_same_route_set`。
fn configure_auth_face(cfg: &mut web::ServiceConfig) {
    cfg.configure(auth::login::configure)
        .configure(auth::identity::configure_routes)
        .configure(auth::zchat::configure)
        .configure(auth::oauth::configure)
        .configure(auth::slo::configure)
        .configure(auth::reset_password::configure_routes)
        .configure(auth::password_change::configure_routes)
        // /me/* 子 scope 先于 profile 的 scope("/me") 注册——actix scope 前缀匹配不回退，
        // scope("/me") 在前会把 /me/mfa、/me/notification-preferences 全部捕获为 404
        // （2026-09-24 双前缀修复后实证）
        .configure(auth::mfa_management::configure_routes)
        .configure(auth::notification_preferences::configure_routes)
        .configure(auth::profile::configure_routes)
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
        .configure(auth::webauthn::configure);
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
        // 面内路由集单源 = `configure_auth_face`（含 webauthn / introspect / token / authenticate）——
        // 本面此前自持清单，导致三条路由（`/introspect`、`/token`(zchat client_credentials)、
        // `/authenticate`(api_key)）只在另一面注册 ⇒ remote 形态（仅反代本前缀）404。
        .configure(configure_auth_face);
}

/// Register SSO protected routes (behind NgacEnforcer or RequireAuth).
///
/// These include NGAC PDP/PIP, audit log, admin API, and WebSocket audit stream.
/// All routes under `/api/ngac`, `/api/audit`, `/api/admin`, `/ws/audit`.
///
/// 安全边界（SECURITY_SPEC §3.4 豁免最小化，fix-sso-noauth-removal）：
/// - `/api/ngac/*` 保留 PDP 决策端点（decide/check/batch/list/columns/explain 等）
///   与审计接入；scope 内层 wrap 空 matcher RequireAuth —— **无 noauth 白名单**，
///   全部端点须持有效 JWT（PEP 自调用带原请求 token；浏览器管理面带 cookie）。
/// - PIP 管理面端点挂载于 `/api/admin/ngac/pip/*`：`RequireAuth`（JWT）+ `NgacPep`
///   （sso_admin OA 决策）双重保护，与既有 `/api/admin` 路由接线一致。
pub fn configure_protected_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api/ngac")
            // fix-sso-noauth-removal：移除 noauth 白名单——/api/ngac 全部端点
            // 必须持有效 JWT（PEP 自调用带原请求 token、浏览器管理面带 cookie）。
            // 空 matcher RequireAuth = 全保护；独立进程外层另有全局 RequireAuth
            // （双验 ES256 廉价）；内嵌模式（Gateway 直挂本函数）由此补齐裸挂面。
            .wrap(crate::auth::middleware::RequireAuth::with_matcher(
                common::PublicRouteMatcher::new(),
            ))
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
    // EmpAgent admin/agents CRUD（move-chat-admin-agents-to-sso）：embedded 随 ns 进程
    // 内挂载、remote 由独立进程承载 + Gateway 反代 /api/chat-sessions/admin 前缀。
    // NgacPep + handler require_admin（NGAC admin）双层，与 /api/admin 接线一致；
    // scope 前缀先于调用方（host 组合根 /api 内的 /chat-sessions）注册——actix
    // 前缀先注册先得、不回落。
    .service(
        web::scope("/api/chat-sessions/admin/agents")
            .wrap(crate::auth::middleware::NgacPep::new())
            .route("", web::get().to(admin::chat_agents::list_agents))
            .route("", web::post().to(admin::chat_agents::create_agent))
            .route("/{code}", web::patch().to(admin::chat_agents::patch_agent))
            .route(
                "/{code}",
                web::delete().to(admin::chat_agents::delete_agent),
            ),
    )
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

// ── 内嵌 SSO 装配 interface（change extract-embedded-sso-assembly）────────────
//
// 装配知识（jwt 材料派生 · 轮换窗口 prev kid · 令牌校验域 · `AuthState` · `OAuthAuthState` ·
// ws state · email/sms service · 微信注册表）的**唯一实现**。独立形态（`build_server`）与内嵌形态
// （`Gateway/host` 及各 ns 组合根 —— 模板单一源，ADR D-036）共用本处实现；宿主 MUST NOT 重排
// 这些字段（判据正本 = 能力 `sso-embedded-assembly`）。

/// 内嵌形态的装配产物：宿主按需取用，不再自持装配知识。
pub struct EmbeddedSso {
    /// SSO 配置（宿主仍需以其作 `web::Data<Config>` 或读取 issuer 等字段）。
    pub config: Config,
    /// 认证状态（宿主需自行包成 `web::Data`）。
    pub auth_state: AuthState,
    pub ws_state: web::Data<websocket::AppState>,
    pub oauth_state: web::Data<auth::oauth_callback::OAuthAuthState>,
    pub email: web::Data<Box<dyn common::EmailService>>,
    pub sms: web::Data<Box<dyn common::SmsService>>,
    pub wx_miniapp: web::Data<common::WechatMiniappRegistry>,
}

/// 令牌校验域 + jwt 材料 + `AuthState` 的**唯一**装配实现（独立/内嵌两形态共用）。
///
/// 令牌校验域 MUST 在此注入：`TokenValidationConfig` 未注入时以 `http://localhost:9002` 兜底
/// （`auth/jwt.rs`），任何「配置了非默认 issuer」的形态若绕过本函数即会带着默认 iss/aud 签发。
fn build_auth_state(config: &Config) -> std::io::Result<AuthState> {
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
    Ok(AuthState {
        jwt_private_key,
        jwt_public_key,
        jwt_public_keys_prev,
        encryption_key: config.encryption_key.as_bytes().to_vec(),
        jwt_access_expiry_secs: config.jwt_access_expiry,
        jwt_refresh_expiry_secs: config.jwt_refresh_expiry,
        identity_verify_mode: config.identity_verify_mode.clone(),
        identity_external_verify_url: config.identity_external_verify_url.clone(),
        ngac_preview_dir: config.ngac_preview_dir.clone(),
    })
}

/// `OAuthAuthState` 的唯一装配实现（从 `AuthState` 派生 4 字段）。
fn build_oauth_state(auth_state: &AuthState) -> auth::oauth_callback::OAuthAuthState {
    auth::oauth_callback::OAuthAuthState {
        jwt_private_key: auth_state.jwt_private_key.clone(),
        jwt_public_key: auth_state.jwt_public_key.clone(),
        jwt_access_expiry_secs: auth_state.jwt_access_expiry_secs,
        jwt_refresh_expiry_secs: auth_state.jwt_refresh_expiry_secs,
    }
}

/// 邮件服务装配（`SSO_EMAIL_MODE` 分支的唯一实现）。
///
/// config.rs fail-closed：未设置/非法值落回 smtp，生产不允许隐式降级 log。
/// 口令解密口径 = system_config（`enc:` AES-256-GCM）；明文写入原样透传
/// （change fix-email-credential-decryptor）。
fn build_email_service(config: &Config, pool: &sqlx::PgPool) -> Box<dyn common::EmailService> {
    match config.email_mode.as_str() {
        "log" => Box::new(log_email::LogEmailService),
        _ => Box::new(common::SmtpEmailService::with_password_decryptor(
            pool.clone(),
            system_config::crypto::decrypt_prefixed,
        )),
    }
}

/// 短信服务装配（唯一实现）。
fn build_sms_service(pool: &sqlx::PgPool) -> Box<dyn common::SmsService> {
    Box::new(common::CloudSmsService::with_decryptor(
        pool.clone(),
        system_config::crypto::decrypt_prefixed,
    ))
}

/// 装配内嵌 SSO 形态（唯一实现）。
///
/// `wx_drivers` 由宿主按 ns 采集后传入（模板 `Gateway/host/src/main.rs` MUST 保持 ns 无关：
/// ADR D-036；来源 = `Pre-Proc/{ns}/gateway-host/src/lib.rs::wechat_miniapp_drivers()`）。
/// 空集中性集 ⇒ 注册表为空 ⇒ 任意 `fromType` 均 `400 INVALID_FROM_TYPE`（fail-closed）。
pub fn assemble_embedded(
    config: Config,
    pool: &sqlx::PgPool,
    wx_drivers: Vec<Box<dyn common::WechatMiniappLogin>>,
) -> std::io::Result<EmbeddedSso> {
    let auth_state = build_auth_state(&config)?;
    Ok(EmbeddedSso {
        oauth_state: web::Data::new(build_oauth_state(&auth_state)),
        ws_state: web::Data::new(websocket::AppState::new()),
        email: web::Data::new(build_email_service(&config, pool)),
        sms: web::Data::new(build_sms_service(pool)),
        wx_miniapp: web::Data::new(common::WechatMiniappRegistry::with_drivers(wx_drivers)),
        auth_state,
        config,
    })
}

/// 内嵌形态的后台任务（周期性清理）。独立形态在 `build_server` 内自启同一实现。
pub fn spawn_background(pool: sqlx::PgPool) {
    housekeeping::spawn(pool);
}

/// Build a standalone SSO HTTP server (backward compatible).
pub async fn build_server(config: Config) -> std::io::Result<actix_web::dev::Server> {
    use actix_web::{middleware::Logger, App, HttpResponse, HttpServer};

    // CSRF 生产 fail-loud（implement-csrf-protection design.md §9）：识别为生产而
    // `CSRF_ENFORCE` 未开 ⇒ log::error! + 非零退出（返回 Err 由 main 传播），MUST NOT
    // 靠人记。生产判定沿用本文件既有判据 = `COOKIE_SECURE`（唯一既有生产档位信号，
    // 与下方 Cookie 安全告警同口径；单一实现 = `common::csrf::cookie_secure_from_env`）。
    if let Err(e) =
        common::csrf::assert_csrf_enforced_in_prod(common::csrf::cookie_secure_from_env())
    {
        log::error!("SSO SECURITY: {}", e);
        return Err(std::io::Error::other(e));
    }

    let database = Database::new(&config)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let pool = database.pool().clone();

    // auth_users / NGAC 扩展表**启动期收敛**（`ensure_ngac_extension_tables`：扩展表 + 账号枚举类型
    // + 1.6 结构性迁移——`name`/`email` 下线、邮箱回填进 `isahl_auth.auth_user_emails`、`status`/`user_type`
    // 收敛为原生枚举）。此前该函数**仅由登录路径触发且进程内一次性**（`EXTENSION_TABLES_ENSURED`
    // AtomicBool）⇒ 新建库（`namespace-db create/drop` 克隆参考库 schema）在首次「完整 FE 登录」前保持
    // 旧列面，而启动链的 `isahl 用户` 种子与 ns 种子套件按**收敛列面**写入 ⇒ 必然报
    // 「字段 name 不存在 / 枚举输入值无效」并以 WARN 失败（2026-09-30 重建 WZ 实测；且收敛不可由
    // 直连 `/auth/login` 触发——简化分支不调用该 ensure）。启动期执行后 AtomicBool 置位 ⇒ 登录路径
    // 自动跳过（幂等，无重复 DDL）；函数内部各步失败仅 WARN（不阻断启动，与上方 ensure 同口径）。
    ngac::ensure::ensure_ngac_extension_tables(&pool).await;

    // NGAC 判定链 PIP 段函数（`effective_ua_ids` → `is_admin`，唯一实现）启动幂等 ensure：
    // 阻塞式（判据面缺失会让管理面门控/列表查询报错）→ 失败 loud 但不阻断启动
    // （`is_admin_user` 内亦有懒 ensure 兜底）。
    if let Err(e) = common::permissions::ensure_ngac_predicate_fns(&pool).await {
        common::telemetry::warn!("ensure ngac predicate fns failed: {}", e);
    }

    // Trigger smart registry（move-chat-admin-agents-to-sso）：admin/agents 写路径经
    // crud::trigger 依赖进程级 registry。embedded 形态由 Gateway host 的 init_framework
    // 初始化（进程共享）；standalone 进程在此自持初始化。AppContainer::Gateway 口径——
    // SSO 用户表同为 isahl_auth.auth_users（业务容器），且 MUST NOT 读 isahl_meta（容器
    // 边界同 Gateway），继承树从 pg_catalog 装载。失败仅 WARN（启动可用，写路径届时报错可见）。
    if let Err(e) = trigger_registry::init::init_smart_registry_global(
        &pool,
        trigger_registry::AppContainer::Gateway,
    )
    .await
    {
        common::telemetry::warn!("Failed to initialize smart trigger registry: {}", e);
    }
    if let Err(e) = trigger_registry::init::refresh_smart_registry_from_pg_catalog(&pool).await {
        common::telemetry::warn!("Failed to load inheritance graph from pg_catalog: {}", e);
    }

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
    // 装配单源 = `build_auth_state`（含令牌校验域注入 + jwt 材料派生；内嵌形态同源复用）。
    let auth_state = build_auth_state(&config)?;

    let server_addr = config.server_addr.clone();
    let ws_app_state = web::Data::new(websocket::AppState::new());
    // G6：SSO_WORKERS env 条件覆盖 worker 数（闭包 move config 前取出）
    let workers = config.workers;
    // G4：housekeeping 独立持有 pool（HttpServer 闭包 move 原 pool）
    let cleanup_pool = pool.clone();

    log::info!("SSO Service listening on {}", server_addr);

    let http_server = HttpServer::new(move || {
        let cors = common::build_cors().expect("Failed to build CORS config");
        App::new()
            .wrap(cors)
            .wrap(Logger::default())
            .wrap(auth::middleware::RequireAuth::new())
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .app_data(ws_app_state.clone())
            .app_data(web::Data::new(build_email_service(&config, &pool)))
            .app_data(web::Data::new(build_sms_service(&pool)))
            .app_data(web::Data::new(build_oauth_state(&auth_state)))
            .route(
                "/health",
                web::get()
                    .to(|| async { HttpResponse::Ok().json(serde_json::json!({"status": "ok"})) }),
            )
            .route("/.well-known/jwks.json", web::get().to(auth::jwt::jwks))
            .configure(configure_routes)
    })
    .backlog(1024);

    let http_server = match workers {
        Some(n) => http_server.workers(n),
        None => http_server,
    };

    let server = http_server
        .bind(&server_addr)
        .map_err(|e| common::server::bind_error(&server_addr, e))?
        .run();

    // G4 周期后台清理（独立进程形态；Gateway 内嵌形态在 Gateway main.rs 接线）
    spawn_background(cleanup_pool);

    Ok(server)
}
