//! API 密钥认证端点
//!
//! `POST /auth/authenticate`：服务使用 `Authorization: Bearer ak_xxx` 换取短时效
//! access_token（JWT，`sub = client:<client_id>`，`svc_user_id` 为服务用户）。
//! 该端点为公共端点（调用方尚无 JWT，故绕过 JWT 中间件），复用
//! `isahl_auth.api_clients`（`client_type='apikey'`）与 argon2id hash 校验。

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::Utc;
use sqlx::PgPool;

use super::jwt::{encode_access_token, Claims};
use super::password::verify_password_async;
use super::AuthState;

/// `Authorization` 头的 Bearer 前缀（含尾随空格）。
const BEARER_PREFIX: &str = "Bearer ";
/// 密钥前缀固定长度，对应 `api_clients.client_id` 的前 8 字符（"ak_" + 4 字符）。
const KEY_PREFIX_LEN: usize = 8;

#[derive(Debug, serde::Serialize)]
struct TokenErrorResponse {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_description: Option<String>,
}

/// 统一的认证失败响应——不区分「前缀不存在 / hash 不匹配 / 已吊销 / 已过期」，
/// 避免泄露密钥是否存在。
fn unauthorized() -> HttpResponse {
    HttpResponse::Unauthorized().json(TokenErrorResponse {
        error: "invalid_api_key".to_string(),
        error_description: Some("Invalid or inactive API key".to_string()),
    })
}

/// 从 `Authorization: Bearer ak_xxx` 提取密钥明文。
///
/// 返回 `None` 表示缺少/格式错误的 Authorization 头（非 401 语义，由调用方决定返回）。
fn extract_api_key(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get(actix_web::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|auth| auth.strip_prefix(BEARER_PREFIX))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `POST /auth/authenticate` —— 用 API 密钥兑换 access_token。
///
/// 流程：
/// 1. 从 `Authorization: Bearer ak_xxx` 提取密钥；缺失 → 401。
/// 2. 以密钥前 8 字符（`ak_xxxx`）作为 `key_prefix` 查询 `api_clients`
///    （`client_type='apikey'` 且 `deleted_at IS NULL`）。未命中 → 401。
/// 3. `verify_password_async` 校验 argon2id hash；失败 → 401。
/// 4. 检查 `enabled` 与 `expires_at`（过期即拒绝）；否则 → 401。
/// 5. 更新 `last_used_at`，签发 15 分钟 JWT（`sub = client:<client_name>`，
///    scope 为密钥被授予 scope 的空格拼接）。
pub async fn authenticate_handler(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let key = match extract_api_key(&req) {
        Some(k) => k,
        None => {
            return HttpResponse::Unauthorized().json(TokenErrorResponse {
                error: "missing_api_key".to_string(),
                error_description: Some("Authorization: Bearer ak_<key> required".to_string()),
            });
        }
    };

    // 前缀必须至少包含 `ak_` + 4 字符，才能构成有效的 8 字符 key_prefix。
    if key.len() < KEY_PREFIX_LEN {
        return unauthorized();
    }
    let key_prefix: String = key.chars().take(KEY_PREFIX_LEN).collect();

    // 1. 查询（仅取未删除的记录；按 client_id 前 8 字符前缀缩小候选，走
    //    idx_api_clients_prefix 索引。API Key 全文存于 api_clients.client_id，
    //    散列存于 secret_hash —— 前缀缩小候选 + argon2id 校验，不泄露完整密钥）
    type ApiKeyLookupRow = (
        i64,
        String,
        String,
        Vec<String>,
        bool,
        Option<chrono::DateTime<Utc>>,
        i64,
    );
    let row: Option<ApiKeyLookupRow> = sqlx::query_as(
        "SELECT id, client_id, secret_hash, scopes, enabled, expires_at, fk_service_user \
             FROM isahl_auth.api_clients \
             WHERE client_type = 'apikey' AND left(client_id, 8) = $1 AND deleted_at IS NULL",
    )
    .bind(&key_prefix)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);

    let (id, client_id, key_hash, scopes, enabled, expires_at, svc_user_id) = match row {
        Some(r) => r,
        None => return unauthorized(),
    };

    // 2. hash 校验（对完整密钥）
    if verify_password_async(key.clone(), key_hash).await.is_err() {
        return unauthorized();
    }

    // 3. 激活状态与过期检查
    if !enabled {
        return unauthorized();
    }
    if let Some(exp) = expires_at {
        if exp < Utc::now() {
            return unauthorized();
        }
    }

    // 4. 更新 last_used_at（best-effort，失败不影响签发）
    let _ = sqlx::query("UPDATE isahl_auth.api_clients SET last_used_at = NOW() WHERE id = $1")
        .bind(id)
        .execute(pool.get_ref())
        .await;

    // 5. 签发 access_token（sub=client:<client_id>，svc_user_id 供 PEP 走 NGAC）
    let mut claims = Claims::with_expiry_seconds(
        &format!("client:{}", client_id),
        "",
        false,
        state.jwt_access_expiry_secs,
    );
    claims.scope = scopes.join(" ");
    claims.svc_user_id = svc_user_id;

    let access_token = match encode_access_token(&claims, &state.jwt_private_key) {
        Ok(t) => t,
        Err(e) => {
            log::error!("api_key authenticate: failed to encode token: {}", e);
            return HttpResponse::InternalServerError().json(TokenErrorResponse {
                error: "server_error".to_string(),
                error_description: Some("failed to issue token".to_string()),
            });
        }
    };

    let scope_str = claims.scope.clone();
    HttpResponse::Ok().json(serde_json::json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": state.jwt_access_expiry_secs,
        "scope": scope_str,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::jwt::derive_public_key;
    use base64::Engine;
    use sqlx::PgPool;

    const TEST_PRIVATE_KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----
\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgD/UpJ7dxbI+3BhJs
\
dDIxSFS+tdT9wSzVVS8z+Au6MRahRANCAATEcFhYPhVkFdIGNAiBwxQpu0cYRXc0
\
roJB3RHF1LfIsaCxcnVep0snC4+8StUixIjfLAZ8Mc8+uqa43ndeNEFm
\
-----END PRIVATE KEY-----";

    async fn test_pool() -> PgPool {
        let url = std::env::var("DATABASE_URL")
            .or_else(|_| std::env::var("SSO_TEST_DATABASE_URL"))
            .unwrap_or_else(|_| {
                let user = std::env::var("USER").unwrap_or_else(|_| "william.d.zk".to_string());
                format!("postgres://{}@localhost:5432/aliothstudio_test", user)
            });
        PgPool::connect(&url)
            .await
            .expect("无法连接测试库，请先运行 `bash scripts/db/reset-db.sh --test`")
    }

    fn test_state() -> AuthState {
        let private = TEST_PRIVATE_KEY.to_vec();
        let public = derive_public_key(&private).expect("derive public key");
        AuthState {
            jwt_private_key: private,
            jwt_public_key: public,
            jwt_public_keys_prev: vec![],
            encryption_key: vec![],
            ngac_preview_dir: None,
            jwt_access_expiry_secs: 900,
            jwt_refresh_expiry_secs: 604800,
            identity_verify_mode: "local".to_string(),
            identity_external_verify_url: None,
        }
    }

    /// 直接插入一条 API 密钥（绕过 admin 鉴权），返回 (明文, 撤销用的 id 不暴露)。
    async fn insert_key(
        pool: &PgPool,
        client_name: &str,
        plaintext: &str,
        scopes: &[String],
    ) -> i64 {
        use crate::auth::password::hash_password_async;
        use crate::auth::service_user::ensure_service_user;
        let key_hash = hash_password_async(plaintext.to_string())
            .await
            .expect("hash key");
        // api_clients：client_id = 完整密钥明文（前缀索引 left(client_id,8) 缩小候选）
        let svc_user = ensure_service_user(pool, plaintext, client_name)
            .await
            .expect("ensure service user");
        let id: (i64,) = sqlx::query_as(
            "INSERT INTO isahl_auth.api_clients \
             (client_id, client_type, client_name, secret_hash, scopes, fk_service_user, enabled) \
             VALUES ($1, 'apikey', $2, $3, $4::TEXT[], $5, TRUE) RETURNING id",
        )
        .bind(plaintext)
        .bind(client_name)
        .bind(&key_hash)
        .bind(scopes)
        .bind(svc_user)
        .fetch_one(pool)
        .await
        .expect("insert api client");
        id.0
    }

    async fn cleanup_key(pool: &PgPool, id: i64) {
        // 清理 api_clients 与关联服务用户（定向，避免全表 DELETE 破坏 seed）
        let svc_user: Option<i64> =
            sqlx::query_scalar("SELECT fk_service_user FROM isahl_auth.api_clients WHERE id = $1")
                .bind(id)
                .fetch_optional(pool)
                .await
                .unwrap_or(None);
        if let Some(uid) = svc_user {
            let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
                .bind(uid)
                .execute(pool)
                .await;
        }
        let _ = sqlx::query("DELETE FROM isahl_auth.api_clients WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await;
    }

    async fn call(pool: PgPool, state: AuthState, api_key: &str) -> HttpResponse {
        let req = actix_web::test::TestRequest::default()
            .insert_header((
                actix_web::http::header::AUTHORIZATION,
                format!("Bearer {}", api_key),
            ))
            .to_http_parts()
            .0;
        authenticate_handler(req, web::Data::new(pool), web::Data::new(state)).await
    }

    async fn body_to_json(resp: HttpResponse) -> serde_json::Value {
        use actix_web::body::to_bytes;
        let bytes = to_bytes(resp.into_body()).await.expect("read body");
        serde_json::from_slice(&bytes).expect("parse json")
    }

    #[tokio::test]
    async fn valid_key_exchanges_for_access_token() {
        let pool = test_pool().await;
        let plaintext = format!(
            "ak_{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7u8; 32])
        );
        let scopes = vec!["read".to_string(), "write".to_string()];
        let id = insert_key(&pool, "svc-test", &plaintext, &scopes).await;

        let resp = call(pool.clone(), test_state(), &plaintext).await;
        let json = body_to_json(resp).await;

        assert_eq!(json["token_type"], serde_json::json!("Bearer"));
        let token = json["access_token"].as_str().expect("access_token");
        assert!(!token.is_empty());

        let state = test_state();
        let claims = crate::auth::jwt::decode_token(token, &state.jwt_public_key).unwrap();
        // sub = client:<client_id>；API Key 场景 client_id = 完整密钥明文
        assert_eq!(claims.sub, format!("client:{}", plaintext));
        assert_eq!(claims.scope, "read write");
        assert!(claims.svc_user_id > 0, "API Key 令牌必须携带 svc_user_id");

        cleanup_key(&pool, id).await;
    }

    #[tokio::test]
    async fn revoked_key_is_rejected() {
        let pool = test_pool().await;
        let plaintext = format!(
            "ak_{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([9u8; 32])
        );
        let id = insert_key(&pool, "svc-revoked", &plaintext, &[]).await;

        // revoke (soft delete + disable)
        sqlx::query(
            "UPDATE isahl_auth.api_clients SET deleted_at = NOW(), enabled = FALSE WHERE id = $1",
        )
        .bind(id)
        .execute(&pool)
        .await
        .expect("revoke");

        let resp = call(pool.clone(), test_state(), &plaintext).await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::UNAUTHORIZED);

        cleanup_key(&pool, id).await;
    }

    #[tokio::test]
    async fn invalid_prefix_is_rejected() {
        let pool = test_pool().await;
        let resp = call(pool.clone(), test_state(), "ak_nonexistentkey123").await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn wrong_secret_is_rejected() {
        let pool = test_pool().await;
        let good = format!(
            "ak_{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([3u8; 32])
        );
        let id = insert_key(&pool, "svc-wrong", &good, &[]).await;

        // present a different key with the same prefix → hash mismatch
        let bad = format!(
            "ak_{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([4u8; 32])
        );
        let resp = call(pool.clone(), test_state(), &bad).await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::UNAUTHORIZED);

        cleanup_key(&pool, id).await;
    }
}
