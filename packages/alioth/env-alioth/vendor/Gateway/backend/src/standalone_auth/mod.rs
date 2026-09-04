//! Gateway standalone auth module — ES256 JWT auth without SSO dependency
//!
//! Provides passwordless username-based login, JWT issuance/verification,
//! and a self-contained user store in `isahl_auth.standalone_users`.
//!
//! # Route convention (matching `gateway_sso` API surface)
//! - `configure_routes(cfg)` — registers endpoints under `/auth` scope
//! - `configure_routes_without_scope(cfg)` — registers bare paths without `/auth`
//!   prefix, for use under `web::scope("/api/auth")` from main.rs
//!
//! # JWT
//! - Algorithm: ES256 (ECDSA P-256)
//! - Key: `GATEWAY_JWT_PRIVATE_KEY` env var or embedded dev key
//! - Claims: `{ sub, email, exp, iat, username, namespace, iss="gateway-standalone", sid="" }`

use std::sync::OnceLock;

use actix_web::{web, HttpRequest, HttpResponse};
use jsonwebtoken::{DecodingKey, EncodingKey, Validation};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

// ── JWT Claims ─────────────────────────────────────────────────────────────────

/// Standalone JWT claims — mirrors SSO AppClaims shape for PEP compatibility
///
/// - `iss`: `"gateway-standalone"` (distinguishable from SSO's issuer)
/// - `aud`: `"gateway-standalone"`（PEP 显式强制 iss/aud 绑定，standalone 令牌
///   必须携带与签发 issuer 一致的 aud，否则 PEP 401——NGAC PEP iss/aud 强制校验）
/// - `sid`: always `""` (no server-side session to revoke)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandaloneClaims {
    pub sub: String,
    pub email: String,
    // RFC 7519 §4.1.4/4.1.6：exp/iat 必须是 NumericDate（JSON number），
    // 禁止字符串化——PEP 端 Claims 以 usize 解析，字符串化导致全 401
    // （serde_zuid 迁移误伤回归，2026-08-13 实测修复）
    pub exp: i64,
    pub iat: i64,
    pub username: String,
    pub namespace: String,
    #[serde(default)]
    pub iss: Option<String>,
    #[serde(default)]
    pub aud: Option<String>,
    #[serde(default)]
    pub sid: String,
}

// ── Issuer ─────────────────────────────────────────────────────────────────────

/// Standalone JWT 的 iss/aud 值 —— standalone_auth 签发与 Gateway PEP 校验的单一事实源。
/// main.rs 在 no-sso 分支将其作为 PEP 的 token binding（对应 SSO 模式读 SSO_JWT_ISSUER），
/// 保证 PEP iss/aud 校验与签发值恒一致（防历史死锁：PEP 强制默认 issuer → 全 401）。
pub const STANDALONE_ISSUER: &str = "gateway-standalone";

// ── Auth Config ────────────────────────────────────────────────────────────────

/// AuthConfig holds the ES256 key pair for JWT signing and verification
pub struct AuthConfig {
    /// Verification key (public) — for JWT decode in auth handlers
    pub decoding_key: DecodingKey,
    /// Signing key (private)
    pub encoding_key: EncodingKey,
    /// Public key PEM bytes — for PEP middleware (DecodingKey::as_bytes() returns DER, not PEM)
    pub public_key_pem: Vec<u8>,
}

static AUTH_CONFIG: OnceLock<AuthConfig> = OnceLock::new();
pub fn init_auth_config() {
    let (decoding_key, encoding_key, public_key_pem) = load_keys();
    let _ = AUTH_CONFIG.set(AuthConfig {
        decoding_key,
        encoding_key,
        public_key_pem,
    });
    log::warn!("Auth mode: Standalone (no SSO)");
    log::warn!(
        "Standalone mode is for self-hosted/development only — \
         DO NOT expose to public internet"
    );
}

/// Get the global auth config reference (panics if not initialized)
pub fn auth_config() -> &'static AuthConfig {
    AUTH_CONFIG
        .get()
        .expect("AuthConfig not initialized — call init_auth_config() in main")
}

fn load_keys() -> (DecodingKey, EncodingKey, Vec<u8>) {
    if let Ok(pem) = std::env::var("GATEWAY_JWT_PRIVATE_KEY") {
        if !pem.is_empty() && !pem.starts_with("enc:") {
            let encoding_key = EncodingKey::from_ec_pem(pem.as_bytes())
                .expect("GATEWAY_JWT_PRIVATE_KEY is not a valid EC P-256 private key PEM");
            let decoding_key = DecodingKey::from_ec_pem(pem.as_bytes())
                .expect("GATEWAY_JWT_PRIVATE_KEY cannot derive public key");
            log::info!("Loaded standalone ES256 key from GATEWAY_JWT_PRIVATE_KEY");
            // Private key PEM can also serve as the public key for verification
            return (decoding_key, encoding_key, pem.into_bytes());
        }
    }

    log::warn!("GATEWAY_JWT_PRIVATE_KEY not configured — using embedded development key");
    log::warn!("Set GATEWAY_JWT_PRIVATE_KEY for production use.");

    let encoding_key = EncodingKey::from_ec_pem(DEV_PRIVATE_KEY.as_bytes())
        .expect("Embedded DEV_PRIVATE_KEY is invalid");
    let decoding_key = DecodingKey::from_ec_pem(DEV_PUBLIC_KEY.as_bytes())
        .expect("Embedded DEV_PUBLIC_KEY is invalid — derive from DEV_PRIVATE_KEY");
    (
        decoding_key,
        encoding_key,
        DEV_PUBLIC_KEY.as_bytes().to_vec(),
    )
}

/// Embedded development ES256 private key (PKCS#8 PEM) — local dev only.
///
/// Generated via:
/// ```sh
/// openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:prime256v1
/// ```
const DEV_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgvTkNZwK8WqNH/aEn
rUkSD5+lYAesakhvTFcWpKteHbOhRANCAASmyJF5MqiJ0MkA77TZJkGAdqiqhv26
IVcpjkHR5sxTZhZ5eH/SSSV/ddphVgahp0cRM9H4HSgzNMIkDNv5dJuN
-----END PRIVATE KEY-----";
/// Derived EC P-256 public key (PKCS#8 PEM) for local dev — matches DEV_PRIVATE_KEY.
/// Generated via: p256::SecretKey::from_pkcs8_pem(DEV_PRIVATE_KEY).public_key().to_public_key_pem()
const DEV_PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEpsiReTKoidDJAO+02SZBgHaoqob9
uiFXKY5B0ebMU2YWeXh/0kklf3XaYVYGoadHETPR+B0oMzTCJAzb+XSbjQ==
-----END PUBLIC KEY-----";

// ── Namespace Derivation ───────────────────────────────────────────────────────

/// Derive a namespace from a raw username: `NS-<Pascal(sanitize(username))>`
///
/// Algorithm:
/// 1. Keep only `[a-zA-Z0-9-]` characters
/// 2. Uppercase the first character (PascalCase)
/// 3. Prefix with `NS-`
///
/// # Errors
/// Returns an error if the sanitized result is empty.
///
/// # Examples
/// - `"alice"` → `"NS-Alice"`
/// - `"bob.smith"` → `"NS-Bobsmith"`
/// - `"alice!"` → `"NS-Alice"`
fn derive_namespace(raw: &str) -> Result<String, &'static str> {
    let sanitized: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if sanitized.is_empty() {
        return Err("Username must contain at least one alphanumeric character");
    }
    let pascal = match sanitized.chars().next() {
        Some(c) => c.to_uppercase().to_string() + &sanitized[1..],
        None => return Err("Invalid username"),
    };
    Ok(format!("NS-{}", pascal))
}

// ── Handlers ───────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
}

/// POST /auth/login — passwordless login
///
/// 1. Normalizes username (trim, lowercase)
/// 2. Queries `isahl_auth.standalone_users` by `username_norm`
/// 3. If not found: derives namespace → `NS-<Pascal(sanitized)>`, inserts new user
/// 4. Issues ES256 JWT (30 min expiry, `iss="gateway-standalone"`, `sid=""`)
///
/// Returns `201` (new user) or `200` (existing user).
/// Returns `409` on namespace conflict (unique constraint violation).
async fn login(pool: web::Data<PgPool>, body: web::Json<LoginRequest>) -> HttpResponse {
    let raw = body.username.trim();
    if raw.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "bad_request", "message": "Username is required"
        }));
    }

    let username_norm = raw.to_lowercase();
    let existing: Option<(i64, String, String)> = sqlx::query_as(
        "SELECT id, username, namespace FROM isahl_auth.standalone_users WHERE username_norm = $1",
    )
    .bind(&username_norm)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);

    let (user_id, username, namespace, is_new) = match existing {
        Some((id, uname, ns)) => (id, uname, ns, false),
        None => {
            let ns = match derive_namespace(raw) {
                Ok(n) => n,
                Err(e) => {
                    return HttpResponse::BadRequest().json(serde_json::json!({
                        "error": "bad_request", "message": e
                    }))
                }
            };
            match sqlx::query_as::<_, (i64,)>(
                "INSERT INTO isahl_auth.standalone_users (username, username_norm, namespace) \
                 VALUES ($1, $2, $3) RETURNING id",
            )
            .bind(raw)
            .bind(&username_norm)
            .bind(&ns)
            .fetch_optional(pool.get_ref())
            .await
            {
                Ok(Some((id,))) => (id, raw.to_string(), ns, true),
                Ok(None) | Err(_) => {
                    return HttpResponse::Conflict().json(serde_json::json!({
                        "error": "namespace_conflict",
                        "message": format!("Namespace '{}' is already taken", ns)
                    }))
                }
            }
        }
    };

    let now = chrono::Utc::now();
    let exp = (now.timestamp() + 1800) as i64;
    let claims = StandaloneClaims {
        sub: user_id.to_string(),
        email: format!("{}@standalone.local", username),
        exp,
        iat: now.timestamp() as i64,
        username: username.clone(),
        namespace: namespace.clone(),
        iss: Some(STANDALONE_ISSUER.to_string()),
        aud: Some(STANDALONE_ISSUER.to_string()),
        sid: String::new(),
    };

    let token = match jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256),
        &claims,
        &auth_config().encoding_key,
    ) {
        Ok(t) => t,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "token_sign_failed", "message": format!("JWT signing: {}", e)
            }))
        }
    };
    let status = if is_new { 201 } else { 200 };
    HttpResponse::build(actix_web::http::StatusCode::from_u16(status).unwrap())
        .json(serde_json::json!({
            "token": token,
            "user": { "id": user_id, "username": username, "namespace": namespace, "is_new": is_new }
        }))
}
/// POST /auth/logout — no-op
///
/// Standalone mode has no server-side sessions to invalidate.
async fn logout() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({ "message": "Logged out" }))
}

/// POST /auth/refresh — re-issue JWT with fresh expiry
///
/// Verifies the current Bearer token, then issues a new 30-minute token
/// with the same claims but a fresh `exp` and `iat`.
async fn refresh(req: HttpRequest) -> HttpResponse {
    let claims = match extract_and_verify_token(&req) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let now = chrono::Utc::now();
    let exp_ts = (now.timestamp() + 1800) as i64;
    let new_claims = StandaloneClaims {
        sub: claims.sub,
        email: claims.email,
        exp: exp_ts,
        iat: now.timestamp() as i64,
        username: claims.username,
        namespace: claims.namespace,
        iss: Some(STANDALONE_ISSUER.to_string()),
        aud: Some(STANDALONE_ISSUER.to_string()),
        sid: String::new(),
    };

    let token = match jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256),
        &new_claims,
        &auth_config().encoding_key,
    ) {
        Ok(t) => t,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "token_sign_failed",
                "message": format!("Failed to sign JWT: {}", e)
            }));
        }
    };

    HttpResponse::Ok().json(serde_json::json!({ "token": token }))
}

/// GET /auth/me — user info + accessible modules + employee profile
///
/// Returns user data from JWT claims plus employee details from
/// auth_users → zc_id_subj-employee → zc_id_prot-profile_config JOIN.
async fn me(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    preproc: web::Data<std::sync::Mutex<crate::preproc::discovery::PreprocDiscovery>>,
) -> HttpResponse {
    let claims = match extract_and_verify_token(&req) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let user_id: i64 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::BadRequest().json(serde_json::json!({"error": "invalid user id"}))
        }
    };

    // Query auth_users → zc_id_subj-employee → zc_id_prot-profile_config
    let profile: Option<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        r#"
        SELECT
            e.notice AS department,
            e.code AS position,
            e.p_number AS phone,
            u.email AS email,
            u.user_type AS role
        FROM isahl_auth.auth_users u
        LEFT JOIN "isahl.zc_id_subj-employee" e ON e.id = u.entity_id AND e.deleted_at IS NULL
        WHERE u.id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);

    let (department, position, phone, email, role) =
        profile.unwrap_or((None, None, None, None, None));

    // Gather accessible app codes from preproc discovery
    let accessible_modules = {
        let mut discovery = preproc.lock().unwrap();
        discovery.ensure_scanned().ok();
        discovery
            .get_apps()
            .values()
            .map(|app| app.code.clone())
            .collect::<Vec<_>>()
    };
    // ── 主体认知字段（fix-subject-cognition-residual-gaps D4，与 SSO me 同形同语义）──
    // subject：entity_table/entity_id → zc_id_subjects（父表单查询覆盖全部可绑定实体
    // 继承后代）；未绑定/悬空/失败 → null + warn，不 500
    let subject: serde_json::Value = match sqlx::query_as::<_, (Option<String>, Option<i64>)>(
        "SELECT entity_table, entity_id FROM isahl_auth.auth_users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await
    {
        Ok(Some((Some(table), Some(entity_id)))) if !table.is_empty() => {
            match sqlx::query_as::<_, (Option<String>, Option<String>)>(
                "SELECT code, notice FROM isahl.zc_id_subjects WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(entity_id)
            .fetch_optional(pool.get_ref())
            .await
            {
                Ok(Some((code, notice))) => serde_json::json!({
                    "id": entity_id.to_string(),
                    "code": code,
                    "name": notice,
                    "entity_table": table,
                }),
                Ok(None) => {
                    log::warn!(
                        "standalone me: dangling subject entity_id={} table={}",
                        entity_id,
                        table
                    );
                    serde_json::Value::Null
                }
                Err(e) => {
                    log::warn!("standalone me: subject resolve failed: {}", e);
                    serde_json::Value::Null
                }
            }
        }
        Ok(_) => serde_json::Value::Null,
        Err(e) => {
            log::warn!("standalone me: entity binding query failed: {}", e);
            serde_json::Value::Null
        }
    };

    // positions + perspectives：fk_user → empl-natural/empl-agent → post_rr_employee → 岗位；
    // 岗位 → relation-post_view_r_tags → tags-post_view（按岗位聚合）。失败降级空数组。
    let position_rows: Vec<(i64, String, Option<String>)> = sqlx::query_as(
        r#"SELECT p.id, p.code, p.notice
           FROM isahl."zc_id_subj-post_rr_employee" spre
           JOIN isahl."zc_id_subj-position" p ON p.id = spre.ref_left AND p.deleted_at IS NULL
           WHERE spre.deleted_at IS NULL AND spre.ref_right IN (
               SELECT id FROM isahl."zc_id_empl-natural" WHERE fk_user = $1 AND deleted_at IS NULL
               UNION ALL
               SELECT id FROM isahl."zc_id_empl-agent" WHERE fk_user = $1 AND deleted_at IS NULL
           )
           ORDER BY p.o_number, p.id"#,
    )
    .bind(user_id)
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_else(|e| {
        log::warn!("standalone me: positions query failed: {}", e);
        Vec::new()
    });

    let positions: Vec<serde_json::Value> = position_rows
        .iter()
        .map(|(id, code, notice)| {
            serde_json::json!({ "id": id.to_string(), "code": code, "name": notice })
        })
        .collect();

    let mut perspectives: Vec<serde_json::Value> = Vec::new();
    for (pos_id, pos_code, pos_notice) in &position_rows {
        let tags: Vec<(String, Option<String>)> = sqlx::query_as(
            r#"SELECT vt.code, vt.notice
               FROM isahl."zc_id_relation-post_view_r_tags" rt
               JOIN isahl."zc_id_tags-post_view" vt ON vt.id = rt.ref_right AND vt.deleted_at IS NULL
               WHERE rt.ref_left = $1 AND rt.deleted_at IS NULL
               ORDER BY vt.o_number, vt.id"#,
        )
        .bind(pos_id)
        .fetch_all(pool.get_ref())
        .await
        .unwrap_or_else(|e| {
            log::warn!(
                "standalone me: view tags query failed for position {}: {}",
                pos_id,
                e
            );
            Vec::new()
        });
        perspectives.push(serde_json::json!({
            "position_id": pos_id.to_string(),
            "position_code": pos_code,
            "position_name": pos_notice,
            "view_tags": tags.iter().map(|(c, n)| serde_json::json!({ "code": c, "name": n })).collect::<Vec<_>>(),
        }));
    }

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": {
            "id": user_id.to_string(),
            "name": claims.username,
            "email": email.unwrap_or(claims.email),
            "phone": phone,
            "department": department,
            "position": position,
            "role": role.unwrap_or_else(|| String::from("user")),
            "avatar_url": null,
            // standalone 模式无 NGAC 关联：权限矩阵恒空，前端 fail-open（与 PEP fail-open 语义一致）
            "permissions": {},
            // 主体认知（与 SSO me 同形 snake_case 键；前端 normalizeMeUser 直接消费）
            "subject": subject,
            "positions": positions,
            "perspectives": perspectives,
        },
        "namespace": claims.namespace,
        "accessibleModules": accessible_modules,
    }))
}

/// GET /auth/mode — returns auth mode indicator
///
/// Frontends use this to detect standalone vs SSO auth mode.
async fn mode() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({ "auth_mode": "standalone" }))
}

// ── Token Verification ─────────────────────────────────────────────────────────

/// Extract Bearer token from `Authorization` header and verify it with ES256.
///
/// Returns decoded `StandaloneClaims` on success, or an error `HttpResponse`
/// (401 Unauthorized) on failure.
fn extract_and_verify_token(req: &HttpRequest) -> Result<StandaloneClaims, HttpResponse> {
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let token = match auth_header.strip_prefix("Bearer ") {
        Some(t) => t,
        None => {
            return Err(HttpResponse::Unauthorized().json(serde_json::json!({
                "error": "unauthorized",
                "message": "Missing or invalid Authorization header"
            })));
        }
    };

    let mut validation = Validation::new(jsonwebtoken::Algorithm::ES256);
    validation.set_required_spec_claims(&["sub", "exp", "iat"]);
    validation.validate_exp = true;
    // Accept any issuer (our `iss: "gateway-standalone"` or unset)

    match jsonwebtoken::decode::<StandaloneClaims>(token, &auth_config().decoding_key, &validation)
    {
        Ok(token_data) => Ok(token_data.claims),
        Err(e) => Err(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "unauthorized",
            "message": format!("Invalid token: {}", e)
        }))),
    }
}

// ── Route Configuration ────────────────────────────────────────────────────────

/// Register standalone auth routes under `/auth` scope
///
/// Produces: `/auth/login`, `/auth/logout`, `/auth/refresh`, `/auth/me`, `/auth/mode`
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/auth")
            .route("/login", web::post().to(login))
            .route("/logout", web::post().to(logout))
            .route("/refresh", web::post().to(refresh))
            .route("/me", web::get().to(me))
            .route("/mode", web::get().to(mode)),
    );
}

/// Register standalone auth routes without `/auth` prefix
///
/// For use under `web::scope("/api/auth")` from main.rs:
///
/// ```ignore
/// .service(
///     web::scope("/api/auth")
///         .configure(standalone_auth::configure_routes_without_scope)
/// )
/// ```
///
/// Produces: `/login`, `/logout`, `/refresh`, `/me`, `/mode`
/// (within parent scope `/api/auth`)
pub fn configure_routes_without_scope(cfg: &mut web::ServiceConfig) {
    cfg.route("/login", web::post().to(login))
        .route("/logout", web::post().to(logout))
        .route("/refresh", web::post().to(refresh))
        .route("/me", web::get().to(me))
        .route("/mode", web::get().to(mode));
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_namespace_basic() {
        assert_eq!(derive_namespace("alice").unwrap(), "NS-Alice");
    }

    #[test]
    fn test_derive_namespace_with_special_chars() {
        assert_eq!(derive_namespace("bob.smith!").unwrap(), "NS-Bobsmith");
    }

    #[test]
    fn test_derive_namespace_with_dash() {
        assert_eq!(derive_namespace("alice-smith").unwrap(), "NS-Alice-smith");
    }

    #[test]
    fn test_derive_namespace_uppercase_first() {
        assert_eq!(derive_namespace("ALICE").unwrap(), "NS-ALICE");
    }

    #[test]
    fn test_derive_namespace_empty_after_sanitize() {
        assert!(derive_namespace("!!!").is_err());
    }

    #[test]
    fn test_derive_namespace_empty_input() {
        assert!(derive_namespace("").is_err());
    }

    #[test]
    fn test_derive_namespace_chinese_chars() {
        // Chinese chars are not ASCII alphanumeric, so they get stripped
        assert!(derive_namespace("你好").is_err());
    }

    #[test]
    fn test_derive_namespace_mixed() {
        assert_eq!(derive_namespace("user-123").unwrap(), "NS-User-123");
    }

    #[test]
    fn test_namespace_matches_pattern() {
        let ns = derive_namespace("alice").unwrap();
        // Must match: start with uppercase letter, followed by [a-zA-Z0-9-]*
        assert!(ns.starts_with("NS-"));
        let rest = &ns[3..];
        assert!(rest.starts_with(|c: char| c.is_ascii_uppercase()));
        assert!(rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }

    #[test]
    fn test_standalone_claims_serde() {
        let claims = StandaloneClaims {
            sub: "42".to_string(),
            email: "alice@standalone.local".to_string(),
            exp: 1_700_000_000,
            iat: 1_699_900_000,
            username: "alice".to_string(),
            namespace: "NS-Alice".to_string(),
            iss: Some(STANDALONE_ISSUER.to_string()),
            aud: Some(STANDALONE_ISSUER.to_string()),
            sid: String::new(),
        };
        let json = serde_json::to_value(&claims).unwrap();
        assert_eq!(json["sub"], "42");
        assert_eq!(json["iss"], STANDALONE_ISSUER);
        assert_eq!(json["aud"], STANDALONE_ISSUER);
        assert_eq!(json["sid"], "");
        assert_eq!(json["username"], "alice");
    }

    /// standalone 认证链路集成测试：login 签发 token（iss=STANDALONE_ISSUER）→
    /// PEP（token binding 对齐 STANDALONE_ISSUER）→ 受保护业务端点 200。
    #[actix_web::test]
    async fn standalone_login_token_passes_pep() {
        use actix_web::{test, web, App, HttpResponse};
        use alioth_gateway::pep::NgacEnforcer;
        use std::sync::Mutex;

        std::env::set_var("NGAC_FAIL_OPEN", "true");
        // login 处理器与 auth_config() 均依赖全局 AUTH_CONFIG（OnceLock）——
        // 测试进程内无 main()，必须先显式初始化（幂等，与 main.rs init_state 同语义）
        init_auth_config();
        let pool = common::testing::connect_test_db().await;

        for stmt in [
            "CREATE TABLE IF NOT EXISTS isahl_auth.standalone_users (
                id             bigint  PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
                username       text    NOT NULL,
                username_norm  text    NOT NULL,
                namespace      text    NOT NULL,
                created_at     timestamptz NOT NULL DEFAULT now()
             )",
            "CREATE UNIQUE INDEX IF NOT EXISTS uq_standalone_users_username_norm
                ON isahl_auth.standalone_users (username_norm)",
            "CREATE UNIQUE INDEX IF NOT EXISTS uq_standalone_users_namespace
                ON isahl_auth.standalone_users (namespace)",
        ] {
            let _ = sqlx::query(stmt).execute(&pool).await;
        }

        let username = format!(
            "chain_test_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..16]
        );
        let _ = sqlx::query("DELETE FROM isahl_auth.standalone_users WHERE username_norm = $1")
            .bind(username.to_lowercase())
            .execute(&pool)
            .await;

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool.clone()))
                .app_data(web::Data::new(Mutex::new(
                    crate::preproc::discovery::PreprocDiscovery::new("", None),
                )))
                // 与 main.rs 同构：PEP 仅包 /api scope，/auth/login 在 scope 外（免认证）
                .configure(configure_routes)
                .service(
                    web::scope("/api")
                        .wrap(
                            NgacEnforcer::new(
                                pool.clone(),
                                auth_config().public_key_pem.clone(),
                                Vec::new(),
                                String::new(),
                            )
                            .with_token_binding(
                                STANDALONE_ISSUER.to_string(),
                                STANDALONE_ISSUER.to_string(),
                            ),
                        )
                        .route(
                            "/standalone-protected",
                            web::get().to(|| async {
                                HttpResponse::Ok().json(serde_json::json!({"ok": true}))
                            }),
                        ),
                ),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/auth/login")
            .set_json(serde_json::json!({ "username": username }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(
            resp.status().is_success(),
            "login 应 200/201，实际 {}",
            resp.status()
        );
        let body: serde_json::Value = test::read_body_json(resp).await;
        let token = body["token"].as_str().expect("login 响应应含 token");

        let req = test::TestRequest::get()
            .uri("/api/standalone-protected")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status().as_u16(),
            200,
            "standalone token 应通过对齐 issuer 的 PEP（历史死锁回归：默认绑定曾致全 401）"
        );

        let _ = sqlx::query("DELETE FROM isahl_auth.standalone_users WHERE username_norm = $1")
            .bind(username.to_lowercase())
            .execute(&pool)
            .await;
    }
}
