//! OAuth 2.0 Token 端点（RFC 6749）
//!
//! 当前支持 `grant_type=client_credentials`（服务到服务、非交互式获取 access_token）。
//! 复用 `isahl_auth.api_clients` 表校验 `client_id` + `client_secret`，并按客户端
//! 被授予的 scope 子集签发 service token（`sub = client:<client_id>`，
//! `svc_user_id = api_clients.fk_service_user`，无用户级 claims）。
//! （旧 `oidc_clients` 表保留给 L3 OIDC 授权码流程；服务令牌统一走 api_clients。）

use actix_web::{web, HttpResponse};
use sqlx::PgPool;

use super::client_secret::verify_client_secret_async;
use super::jwt::{encode_access_token, Claims};
use super::AuthState;

/// Token 端点请求体（`application/x-www-form-urlencoded`）。
#[derive(Debug, serde::Deserialize)]
pub struct TokenRequest {
    pub grant_type: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct TokenErrorResponse {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_description: Option<String>,
}

fn unauthorized() -> HttpResponse {
    HttpResponse::Unauthorized().json(TokenErrorResponse {
        error: "invalid_client".to_string(),
        error_description: Some("Invalid client credentials".to_string()),
    })
}

fn bad_request(error: &str, description: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(TokenErrorResponse {
        error: error.to_string(),
        error_description: Some(description.to_string()),
    })
}

/// `POST /auth/token` —— 处理 `client_credentials` grant。
///
/// 流程：
/// 1. 校验 `grant_type == client_credentials`，否则 400 `unsupported_grant_type`。
/// 2. 查询 `oidc_clients`：未知/禁用/已删除 client → 401；空 secret（public client）→ 401
///    （client_credentials 必须使用 secret）。
/// 3. `verify_client_secret_async` 校验 secret，失败 → 401。
/// 4. 按授予 scope 子集约束请求 scope：
///    - 未请求 scope → 授予全部 scope；
///    - client 未配置 scope（旧客户端兼容）→ 放行请求的全部 scope；
///    - 否则仅保留授予子集，若含未授予 scope → 400 `invalid_scope`。
/// 5. 签发 access_token（`sub = client:<client_id>`，无刷新令牌）。
pub async fn token_handler(
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    req: web::Form<TokenRequest>,
) -> HttpResponse {
    let grant_type = req.grant_type.as_deref().unwrap_or("");
    if grant_type != "client_credentials" {
        return bad_request(
            "unsupported_grant_type",
            &format!("grant_type '{}' is not supported", grant_type),
        );
    }

    let client_id = req.client_id.as_deref().unwrap_or("");
    let client_secret = req.client_secret.as_deref().unwrap_or("");

    // 1. 查询 client（含 secret 散列、授予 scope 与服务用户）
    let row: Option<(String, Vec<String>, i64)> = sqlx::query_as(
        "SELECT secret_hash, scopes, fk_service_user \
         FROM isahl_auth.api_clients \
         WHERE client_id = $1 AND enabled AND deleted_at IS NULL",
    )
    .bind(client_id)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);

    let (secret_hash, granted, svc_user_id) = match row {
        Some(r) => r,
        None => return unauthorized(),
    };

    // client_credentials 必须携带 secret（public client 不允许该 grant）
    if secret_hash.is_empty() {
        return unauthorized();
    }
    if verify_client_secret_async(client_secret.to_string(), secret_hash)
        .await
        .is_err()
    {
        return unauthorized();
    }

    // 2. scope 子集约束
    let requested: Vec<String> = req
        .scope
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    let effective: Vec<String> = if requested.is_empty() {
        granted.clone()
    } else if granted.is_empty() {
        // 旧客户端未配置 scope：兼容放行请求的全部 scope
        requested.clone()
    } else {
        let eff: Vec<String> = requested
            .iter()
            .filter(|s| granted.contains(s))
            .cloned()
            .collect();
        if eff.len() != requested.len() {
            return bad_request(
                "invalid_scope",
                "requested scope contains values not granted to this client",
            );
        }
        eff
    };

    // 3. 签发 service token（无用户 claims，无刷新令牌；svc_user_id 供 PEP 走 NGAC）
    let mut claims = Claims::new(&format!("client:{}", client_id), "", false);
    claims.scope = effective.join(" ");
    claims.svc_user_id = svc_user_id;
    let access_token = match encode_access_token(&claims, &state.jwt_private_key) {
        Ok(t) => t,
        Err(e) => {
            log::error!("token: failed to encode access_token: {}", e);
            return HttpResponse::InternalServerError().json(TokenErrorResponse {
                error: "server_error".to_string(),
                error_description: Some("failed to issue token".to_string()),
            });
        }
    };

    let scope_str = effective.join(" ");
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
    use crate::auth::client_secret::hash_client_secret_async;
    use crate::auth::jwt::derive_public_key;
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

    async fn insert_client(
        pool: &PgPool,
        suffix: &str,
        secret: &str,
        scopes: &[String],
    ) -> (String, String) {
        let client_id = format!("cc_test_{}", suffix);
        let secret_hash = hash_client_secret_async(secret.to_string())
            .await
            .expect("hash secret");
        // 创建服务用户（api_clients.fk_service_user 必填）
        let svc_user_id = crate::auth::service_user::ensure_service_user(pool, &client_id, "test")
            .await
            .expect("ensure service user");
        sqlx::query(
            "INSERT INTO isahl_auth.api_clients \
             (client_id, client_type, client_name, secret_hash, scopes, fk_service_user, enabled) \
             VALUES ($1, 'oauth2', 'test', $2, $3::TEXT[], $4, TRUE) \
             ON CONFLICT (client_id) DO UPDATE SET \
               secret_hash = $2, scopes = $3::TEXT[], fk_service_user = $4, deleted_at = NULL, enabled = TRUE",
        )
        .bind(&client_id)
        .bind(&secret_hash)
        .bind(scopes)
        .bind(svc_user_id)
        .execute(pool)
        .await
        .expect("insert client");
        (client_id, secret.to_string())
    }

    async fn cleanup_client(pool: &PgPool, client_id: &str) {
        // 清理 api_clients 与关联服务用户（定向，避免全表 DELETE 破坏 seed）
        let svc_user: Option<i64> = sqlx::query_scalar(
            "SELECT fk_service_user FROM isahl_auth.api_clients WHERE client_id = $1",
        )
        .bind(client_id)
        .fetch_optional(pool)
        .await
        .unwrap_or(None);
        if let Some(uid) = svc_user {
            let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
                .bind(uid)
                .execute(pool)
                .await;
        }
        let _ = sqlx::query("DELETE FROM isahl_auth.api_clients WHERE client_id = $1")
            .bind(client_id)
            .execute(pool)
            .await;
    }

    async fn call(pool: PgPool, state: AuthState, body: TokenRequest) -> HttpResponse {
        token_handler(web::Data::new(pool), web::Data::new(state), web::Form(body)).await
    }

    async fn body_to_json(resp: HttpResponse) -> serde_json::Value {
        use actix_web::body::to_bytes;
        let b = resp.into_body();
        let bytes = to_bytes(b).await.expect("read body");
        serde_json::from_slice(&bytes).expect("parse json")
    }

    #[tokio::test]
    async fn valid_credentials_issue_service_token() {
        let pool = test_pool().await;
        let suffix = format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let scopes = vec!["read".to_string(), "write".to_string()];
        let (client_id, secret) = insert_client(&pool, &suffix, "topsecret", &scopes).await;

        let resp = call(
            pool.clone(),
            test_state(),
            TokenRequest {
                grant_type: Some("client_credentials".to_string()),
                client_id: Some(client_id.clone()),
                client_secret: Some(secret),
                scope: Some("read write".to_string()),
            },
        )
        .await;
        let json = body_to_json(resp).await;

        assert_eq!(json["token_type"], serde_json::json!("Bearer"));
        let token = json["access_token"].as_str().expect("access_token");
        assert!(!token.is_empty());

        // 解码验证 sub 与 scope
        let state = test_state();
        let claims = crate::auth::jwt::decode_token(token, &state.jwt_public_key).unwrap();
        assert_eq!(claims.sub, format!("client:{}", client_id));
        assert_eq!(claims.scope, "read write");
        assert!(claims.svc_user_id > 0, "服务令牌必须携带 svc_user_id");

        cleanup_client(&pool, &client_id).await;
    }

    #[tokio::test]
    async fn invalid_secret_is_rejected() {
        let pool = test_pool().await;
        let suffix = format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let scopes = vec!["read".to_string()];
        let (client_id, _) = insert_client(&pool, &suffix, "topsecret", &scopes).await;

        let resp = call(
            pool.clone(),
            test_state(),
            TokenRequest {
                grant_type: Some("client_credentials".to_string()),
                client_id: Some(client_id.clone()),
                client_secret: Some("wrong".to_string()),
                scope: None,
            },
        )
        .await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::UNAUTHORIZED);

        cleanup_client(&pool, &client_id).await;
    }

    #[tokio::test]
    async fn unknown_client_is_rejected() {
        let pool = test_pool().await;
        let resp = call(
            pool.clone(),
            test_state(),
            TokenRequest {
                grant_type: Some("client_credentials".to_string()),
                client_id: Some("does_not_exist_xyz".to_string()),
                client_secret: Some("whatever".to_string()),
                scope: None,
            },
        )
        .await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn unsupported_grant_type_is_rejected() {
        let pool = test_pool().await;
        let resp = call(
            pool.clone(),
            test_state(),
            TokenRequest {
                grant_type: Some("password".to_string()),
                client_id: Some("x".to_string()),
                client_secret: Some("y".to_string()),
                scope: None,
            },
        )
        .await;
        let json = body_to_json(resp).await;
        assert_eq!(json["error"], serde_json::json!("unsupported_grant_type"));
    }

    #[tokio::test]
    async fn ungranted_scope_is_rejected() {
        let pool = test_pool().await;
        let suffix = format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let scopes = vec!["read".to_string()];
        let (client_id, secret) = insert_client(&pool, &suffix, "topsecret", &scopes).await;

        let resp = call(
            pool.clone(),
            test_state(),
            TokenRequest {
                grant_type: Some("client_credentials".to_string()),
                client_id: Some(client_id.clone()),
                client_secret: Some(secret),
                scope: Some("read admin".to_string()),
            },
        )
        .await;
        let json = body_to_json(resp).await;
        assert_eq!(json["error"], serde_json::json!("invalid_scope"));

        cleanup_client(&pool, &client_id).await;
    }
}
