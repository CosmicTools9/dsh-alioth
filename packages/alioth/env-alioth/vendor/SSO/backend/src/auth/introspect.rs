//! OAuth 2.0 Token Introspection 端点（RFC 7662）
//!
//! `POST /auth/introspect` 接收 `application/x-www-form-urlencoded` 的 `token` 字段，
//! 解码 JWT 并校验其绑定会话的吊销状态，返回 RFC 7662 标准响应。
//!
//! 端点公开（无需调用方 JWT）—— 真实性由被自省的 token 本身证明。

use actix_web::{web, HttpResponse};
use serde::Serialize;
use sqlx::PgPool;

use super::jwt::decode_token_any;
use super::session::is_session_active;
use super::AuthState;

/// 自省请求体（RFC 7662: `application/x-www-form-urlencoded`）。
#[derive(Debug, serde::Deserialize)]
pub struct IntrospectRequest {
    pub token: String,
}

/// RFC 7662 自省响应。
///
/// 非活跃时仅返回 `{ "active": false }`；活跃时附加上下文字段。
/// 空字符串字段不序列化，避免泄露多余线索。
#[derive(Debug, Serialize)]
struct IntrospectResponse {
    active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    sub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exp: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    iat: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    iss: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aud: Option<String>,
}

/// 返回 `{"active": false}` 的紧凑响应。
fn inactive() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({ "active": false }))
}

/// Token 自省 handler。
///
/// 流程：
/// 1. 解码 JWT（校验签名 + `exp` + `iss`/`aud`）—— 任一失败即视为非活跃。
/// 2. 若 token 绑定会话（`sid` 非空），查询 `is_session_active` 判定吊销状态。
///    无 `sid` 的 token（未持久化会话）不查库，直接视为活跃。
/// 3. 非活跃返回 `{ "active": false }`；活跃返回附加上下文字段。
pub async fn introspect_handler(
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    req: web::Form<IntrospectRequest>,
) -> HttpResponse {
    let claims = match decode_token_any(&req.token, &state.verification_keys()) {
        Ok(c) => c,
        Err(_) => return inactive(),
    };

    // 会话吊销检查：仅当 token 绑定到持久化会话时才查询。
    let active = if claims.sid.is_empty() {
        true
    } else {
        is_session_active(&pool, &claims.sid).await
    };

    if !active {
        return inactive();
    }

    let response = IntrospectResponse {
        active: true,
        sub: Some(claims.sub.clone()),
        sid: if claims.sid.is_empty() {
            None
        } else {
            Some(claims.sid)
        },
        token_type: Some("access_token".to_string()),
        exp: Some(claims.exp),
        iat: Some(claims.iat),
        email: if claims.email.is_empty() {
            None
        } else {
            Some(claims.email)
        },
        iss: if claims.iss.is_empty() {
            None
        } else {
            Some(claims.iss)
        },
        aud: if claims.aud.is_empty() {
            None
        } else {
            Some(claims.aud)
        },
    };

    HttpResponse::Ok().json(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::jwt::{derive_public_key, encode_access_token, Claims};
    use sqlx::PgPool;

    // 测试用 EC P-256 密钥对（PKCS#8 PEM），与 jwt.rs 测试同源算法。
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

    /// 创建测试用户并返回其 id（同时清理钩子由调用方负责）。
    async fn insert_test_user(pool: &PgPool, suffix: &str) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO isahl_auth.auth_users (name, email, password_hash, status) \
             VALUES ($1, $2, 'argon2-placeholder', 'active') RETURNING id",
        )
        .bind(format!("introspect_test_{}", suffix))
        .bind(format!("introspect_test_{}@example.com", suffix))
        .fetch_one(pool)
        .await
        .expect("插入测试用户失败")
    }

    async fn insert_test_session(pool: &PgPool, user_id: i64, sid: &str) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO isahl_auth.sso_sessions (user_id, session_token, status, expires_at) \
             VALUES ($1, $2, 'active', now() + interval '1 hour') RETURNING id",
        )
        .bind(user_id)
        .bind(sid)
        .fetch_one(pool)
        .await
        .expect("插入测试会话失败")
    }

    async fn revoke_session(pool: &PgPool, sid: &str) {
        sqlx::query(
            "UPDATE isahl_auth.sso_sessions SET status = 'revoked' WHERE session_token = $1",
        )
        .bind(sid)
        .execute(pool)
        .await
        .expect("吊销会话失败");
    }

    async fn cleanup(pool: &PgPool, user_id: i64) {
        let _ = sqlx::query("DELETE FROM isahl_auth.sso_sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
            .bind(user_id)
            .execute(pool)
            .await;
    }

    async fn body_to_json(resp: HttpResponse) -> serde_json::Value {
        use actix_web::body::to_bytes;
        let body = resp.into_body();
        let bytes = to_bytes(body).await.expect("read body");
        serde_json::from_slice(&bytes).expect("parse json")
    }

    #[tokio::test]
    async fn introspect_active_session_token_is_active() {
        let pool = test_pool().await;
        let suffix = format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let user_id = insert_test_user(&pool, &suffix).await;
        let sid = format!("sess_active_{}", suffix);
        let _sess_id = insert_test_session(&pool, user_id, &sid).await;

        let state = test_state();
        let mut claims = Claims::new(&user_id.to_string(), "u@example.com", false);
        claims.sid = sid.clone();
        let token = encode_access_token(&claims, &state.jwt_private_key).unwrap();

        let resp = introspect_handler(
            web::Data::new(pool.clone()),
            web::Data::new(state),
            web::Form(IntrospectRequest { token }),
        )
        .await;

        let json = body_to_json(resp).await;
        assert_eq!(json["active"], serde_json::json!(true));
        assert_eq!(json["sub"], serde_json::json!(user_id.to_string()));
        assert_eq!(json["sid"], serde_json::json!(sid));
        assert_eq!(json["token_type"], serde_json::json!("access_token"));

        cleanup(&pool, user_id).await;
    }

    #[tokio::test]
    async fn introspect_revoked_session_is_inactive() {
        let pool = test_pool().await;
        let suffix = format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let user_id = insert_test_user(&pool, &suffix).await;
        let sid = format!("sess_revoked_{}", suffix);
        let _sess_id = insert_test_session(&pool, user_id, &sid).await;
        revoke_session(&pool, &sid).await;

        let state = test_state();
        let mut claims = Claims::new(&user_id.to_string(), "u@example.com", false);
        claims.sid = sid.clone();
        let token = encode_access_token(&claims, &state.jwt_private_key).unwrap();

        let resp = introspect_handler(
            web::Data::new(pool.clone()),
            web::Data::new(state),
            web::Form(IntrospectRequest { token }),
        )
        .await;

        let json = body_to_json(resp).await;
        assert_eq!(json, serde_json::json!({ "active": false }));

        cleanup(&pool, user_id).await;
    }

    #[tokio::test]
    async fn introspect_invalid_token_is_inactive() {
        let pool = test_pool().await;
        let state = test_state();

        let resp = introspect_handler(
            web::Data::new(pool.clone()),
            web::Data::new(state),
            web::Form(IntrospectRequest {
                token: "not-a-real-jwt".to_string(),
            }),
        )
        .await;

        let json = body_to_json(resp).await;
        assert_eq!(json, serde_json::json!({ "active": false }));
    }

    #[tokio::test]
    async fn introspect_token_without_session_is_active() {
        let pool = test_pool().await;
        let state = test_state();
        // 无 sid 绑定：不查库，签名/过期通过后即视为活跃。
        let claims = Claims::new("user-no-sid", "u@example.com", false);
        let token = encode_access_token(&claims, &state.jwt_private_key).unwrap();

        let resp = introspect_handler(
            web::Data::new(pool.clone()),
            web::Data::new(state),
            web::Form(IntrospectRequest { token }),
        )
        .await;

        let json = body_to_json(resp).await;
        assert_eq!(json["active"], serde_json::json!(true));
        assert_eq!(json["sub"], serde_json::json!("user-no-sid"));
        assert_eq!(json["sid"], serde_json::Value::Null);
    }
}
