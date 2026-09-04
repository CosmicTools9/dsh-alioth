//! `POST /auth/token` client_credentials 兑换测试（fix-wz-map-gaps：北斗终端接入通道）
//!
//! 设备凭证走 `isahl_auth.api_clients`（管理面 admin/api_clients.rs 签发/轮换/吊销，
//! 用户面 auth/portal.rs 自助），`svc_user_id` 供 PEP 走 NGAC——终端先兑换
//! service token 再调业务端点（transport-operations /tracking-ingest）。
//! 覆盖三态：有效凭证 200、无效 secret 401、未知 client 401；附 scope 越界 400。
//! 每测试独立 client_id（并行安全，互不竞争 fixture）。

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth::client_secret::hash_client_secret_async;
use gateway_sso::auth::service_user::ensure_service_user;
use gateway_sso::auth::token::token_handler;
use gateway_sso::auth::AuthState;
use sqlx::PgPool;

async fn connect() -> PgPool {
    ::common::testing::connect_test_db().await
}

fn test_auth_state() -> AuthState {
    common::test_auth_state()
}

/// fixture：服务用户 + api_clients 行（argon2id secret）
async fn seed_client(pool: &PgPool, client_id: &str, secret: &str, scopes: &[&str]) {
    let mut tx = pool.begin().await.expect("begin tx");
    let svc_user = ensure_service_user(&mut tx, client_id, "测试北斗终端")
        .await
        .expect("ensure service user");
    let hash = hash_client_secret_async(secret.to_string())
        .await
        .expect("hash secret");
    let scope_arr = format!("{{{}}}", scopes.join(","));
    sqlx::query(
        r#"INSERT INTO isahl_auth.api_clients
           (id, client_id, client_type, client_name, secret_hash, scopes, fk_service_user, enabled)
           VALUES (isahl.gen_next_zuid(), $1, 'apikey', '测试北斗终端', $2, $3::TEXT[], $4, TRUE)"#,
    )
    .bind(client_id)
    .bind(&hash)
    .bind(&scope_arr)
    .bind(svc_user)
    .execute(&mut *tx)
    .await
    .expect("seed api_client");
    tx.commit().await.expect("commit");
}

async fn cleanup(pool: &PgPool, client_id: &str) {
    sqlx::query(r#"DELETE FROM isahl_auth.api_clients WHERE client_id = $1"#)
        .bind(client_id)
        .execute(pool)
        .await
        .expect("cleanup api_client");
    sqlx::query(r#"DELETE FROM isahl_auth.auth_users WHERE username = $1"#)
        .bind(format!("svc:{client_id}"))
        .execute(pool)
        .await
        .expect("cleanup service user");
}
async fn post_token(pool: &PgPool, body: &str) -> (u16, serde_json::Value) {
    let ast = test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(ast))
            .app_data(web::Data::new(pool.clone()))
            .route("/token", web::post().to(token_handler)),
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/token")
        .insert_header((
            actix_web::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        ))
        .set_payload(body.to_string())
        .to_request();
    let resp = test::call_service(&app, req).await;
    let status = resp.status().as_u16();
    let body_text = String::from_utf8(test::read_body(resp).await.to_vec()).unwrap_or_default();
    let value: serde_json::Value = serde_json::from_str(&body_text).unwrap_or_default();
    (status, value)
}

/// 有效凭证：签发 service token（Bearer + access_token 非空）
#[tokio::test]
async fn token_client_credentials_issues_service_token() {
    let cid = "test-device-issue-001";
    let pool = connect().await;
    cleanup(&pool, cid).await;
    seed_client(&pool, cid, "device-secret-abc123", &["transport-tracking"]).await;

    let (status, body) = post_token(
        &pool,
        &format!(
            "grant_type=client_credentials&client_id={cid}&client_secret=device-secret-abc123"
        ),
    )
    .await;
    assert_eq!(status, 200, "有效凭证应 200：{body}");
    assert_eq!(body["token_type"], "Bearer");
    let token = body["access_token"].as_str().unwrap_or_default();
    assert!(!token.is_empty(), "access_token 不应为空");
    assert!(body["expires_in"].as_u64().unwrap_or(0) > 0, "应返回有效期");

    cleanup(&pool, cid).await;
}

/// 无效 secret：401 invalid_client
#[tokio::test]
async fn token_client_credentials_rejects_wrong_secret() {
    let cid = "test-device-wrong-002";
    let pool = connect().await;
    cleanup(&pool, cid).await;
    seed_client(&pool, cid, "device-secret-abc123", &[]).await;

    let (status, body) = post_token(
        &pool,
        &format!("grant_type=client_credentials&client_id={cid}&client_secret=wrong-secret"),
    )
    .await;
    assert_eq!(status, 401, "无效 secret 应 401：{body}");
    assert_eq!(body["error"], "invalid_client");

    cleanup(&pool, cid).await;
}

/// 未知 client（未配置凭证）：401——设备凭证未配置即显式拒绝，无静默放行
#[tokio::test]
async fn token_client_credentials_rejects_unknown_client() {
    let pool = connect().await;
    let (status, body) = post_token(
        &pool,
        "grant_type=client_credentials&client_id=no-such-device-000&client_secret=whatever",
    )
    .await;
    assert_eq!(status, 401, "未知 client 应 401：{body}");
    assert_eq!(body["error"], "invalid_client");
}

/// scope 越界：400 invalid_scope（设备凭证按授予子集收敛）
#[tokio::test]
async fn token_client_credentials_rejects_ungranted_scope() {
    let cid = "test-device-scope-003";
    let pool = connect().await;
    cleanup(&pool, cid).await;
    seed_client(&pool, cid, "device-secret-abc123", &["transport-tracking"]).await;

    let (status, body) = post_token(&pool, &format!(
        "grant_type=client_credentials&client_id={cid}&client_secret=device-secret-abc123&scope=admin"
    ))
    .await;
    assert_eq!(status, 400, "未授予 scope 应 400：{body}");
    assert_eq!(body["error"], "invalid_scope");

    cleanup(&pool, cid).await;
}
