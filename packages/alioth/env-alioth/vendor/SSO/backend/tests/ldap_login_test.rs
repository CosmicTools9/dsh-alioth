//! LDAP 登录集成测试（fix-sso-id-default-heal：sso-gap-report 剩余项闭环）
//!
//! 负路径常跑（无需真实 LDAP server）：
//! - 未配置 → 503「LDAP 认证未配置」
//! - 配置指向不可达 server → 500「认证服务暂时不可用」（`LdapClient::new` 连接失败
//!   → authenticator 跳过该配置 → `last_error` 兜底）
//! - 配置禁用 → `load_ldap_configs` 的 `enabled = true` 谓词过滤 → 503（同未配置）
//!
//! 正路径（真实 LDAP 全流程：登录 → 用户同步 → 会话 → JWT）环境门控：
//! 设 `SSO_LDAP_TEST_URL`（可选 `SSO_LDAP_TEST_BIND_DN` / `SSO_LDAP_TEST_BIND_PASSWORD`
//! / `SSO_LDAP_TEST_BASE_DN` / `SSO_LDAP_TEST_USERNAME` / `SSO_LDAP_TEST_PASSWORD`）时执行。

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth::jwt::configure_token_validation;
use gateway_sso::auth::ldap::ldap_login;
use serde_json::json;
use sqlx::PgPool;

async fn connect() -> PgPool {
    ::common::testing::connect_test_db().await
}

/// 与测试库实际 schema 对齐的 `ldap_configs` 幂等建表（迁移链同源语义）。
async fn ensure_ldap_configs_table(pool: &PgPool) {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.ldap_configs (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            name VARCHAR(128) NOT NULL,
            url VARCHAR(256) NOT NULL,
            bind_dn VARCHAR(256) NOT NULL DEFAULT '',
            bind_password VARCHAR(256) NOT NULL DEFAULT '',
            base_dn VARCHAR(256) NOT NULL DEFAULT '',
            user_filter VARCHAR(256) NOT NULL DEFAULT '(sAMAccountName={})',
            username_attr VARCHAR(64) NOT NULL DEFAULT 'sAMAccountName',
            email_attr VARCHAR(64) NOT NULL DEFAULT 'mail',
            display_name_attr VARCHAR(64) NOT NULL DEFAULT 'displayName',
            groups_attr VARCHAR(64) NOT NULL DEFAULT 'memberOf',
            use_ldaps BOOLEAN NOT NULL DEFAULT FALSE,
            timeout_secs INT NOT NULL DEFAULT 30,
            enabled BOOLEAN NOT NULL DEFAULT TRUE,
            sync_groups BOOLEAN NOT NULL DEFAULT FALSE,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            group_mapping JSONB NOT NULL DEFAULT '{}'
        )"#,
    )
    .execute(pool)
    .await
    .expect("ensure ldap_configs");
}

async fn seed_config(pool: &PgPool, url: &str, enabled: bool) {
    sqlx::query(
        "INSERT INTO isahl_auth.ldap_configs (name, url, bind_dn, bind_password, base_dn, \
         timeout_secs, enabled) \
         VALUES ('it-ldap', $1, 'cn=admin,dc=test,dc=local', 'secret', 'dc=test,dc=local', \
         1, $2)",
    )
    .bind(url)
    .bind(enabled)
    .execute(pool)
    .await
    .expect("seed ldap config");
}

async fn purge_configs(pool: &PgPool, name: &str) {
    sqlx::query("DELETE FROM isahl_auth.ldap_configs WHERE name = $1")
        .bind(name)
        .execute(pool)
        .await
        .ok();
}

macro_rules! init_app {
    ($pool:expr, $ast:expr) => {{
        test::init_service(
            App::new()
                .app_data(web::Data::new($pool.clone()))
                .app_data(web::Data::new($ast.clone()))
                .service(web::resource("/auth/login/ldap").route(web::post().to(ldap_login))),
        )
        .await
    }};
}

#[tokio::test]
async fn ldap_login_unconfigured_returns_503() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("setup");
    ensure_ldap_configs_table(&pool).await;
    purge_configs(&pool, "it-ldap").await;

    let app = init_app!(pool, common::test_auth_state());
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login/ldap")
            .set_json(json!({"username": "someone", "password": "whatever"}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 503, "无配置应 503");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["error"], "LDAP 认证未配置");
}

#[tokio::test]
async fn ldap_login_unreachable_server_returns_500() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("setup");
    ensure_ldap_configs_table(&pool).await;
    purge_configs(&pool, "it-ldap").await;
    // 127.0.0.1:1 无监听 → 连接拒绝（timeout 1s 兜底，不会挂起 30s）
    seed_config(&pool, "ldap://127.0.0.1:1", true).await;

    let app = init_app!(pool, common::test_auth_state());
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login/ldap")
            .set_json(json!({"username": "someone", "password": "whatever"}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 500, "不可达应 500（连接失败兜底）");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["error"], "认证服务暂时不可用");

    purge_configs(&pool, "it-ldap").await;
}

#[tokio::test]
async fn ldap_login_disabled_config_behaves_unconfigured() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("setup");
    ensure_ldap_configs_table(&pool).await;
    purge_configs(&pool, "it-ldap").await;
    seed_config(&pool, "ldap://127.0.0.1:1", false).await;

    let app = init_app!(pool, common::test_auth_state());
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login/ldap")
            .set_json(json!({"username": "someone", "password": "whatever"}))
            .to_request(),
    )
    .await;
    // enabled=true 谓词过滤 → 配置集为空 → 与未配置同语义 503
    assert_eq!(resp.status(), 503, "禁用配置应与未配置同语义");

    purge_configs(&pool, "it-ldap").await;
}

/// 正路径（环境门控）：真实 LDAP 全流程——登录 → 用户同步 → 会话 → JWT。
#[tokio::test]
async fn ldap_login_real_server_full_flow() {
    let url = match std::env::var("SSO_LDAP_TEST_URL") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            println!(
                "skip ldap_login_real_server_full_flow: 未设置 SSO_LDAP_TEST_URL（需真实 LDAP server）"
            );
            return;
        }
    };
    let bind_dn = std::env::var("SSO_LDAP_TEST_BIND_DN").unwrap_or_default();
    let bind_password = std::env::var("SSO_LDAP_TEST_BIND_PASSWORD").unwrap_or_default();
    let base_dn =
        std::env::var("SSO_LDAP_TEST_BASE_DN").unwrap_or_else(|_| "dc=test,dc=local".into());
    let username = std::env::var("SSO_LDAP_TEST_USERNAME").expect("SSO_LDAP_TEST_USERNAME");
    let password = std::env::var("SSO_LDAP_TEST_PASSWORD").expect("SSO_LDAP_TEST_PASSWORD");

    let pool = connect().await;
    common::setup_schema(&pool).await.expect("setup");
    ensure_ldap_configs_table(&pool).await;
    purge_configs(&pool, "it-ldap").await;
    sqlx::query(
        "INSERT INTO isahl_auth.ldap_configs (name, url, bind_dn, bind_password, base_dn, \
         timeout_secs, enabled) VALUES ('it-ldap', $1, $2, $3, $4, 5, TRUE)",
    )
    .bind(&url)
    .bind(&bind_dn)
    .bind(&bind_password)
    .bind(&base_dn)
    .execute(&pool)
    .await
    .expect("seed real ldap config");

    let ast = common::test_auth_state();
    configure_token_validation(
        "http://localhost:9002".into(),
        "http://localhost:9002".into(),
    );
    let app = init_app!(pool, ast);
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/login/ldap")
            .set_json(json!({"username": username, "password": password}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "真实 LDAP 登录应 200");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(!body["access_token"].as_str().unwrap_or_default().is_empty());
    assert_eq!(body["user"]["is_ldap_user"], true);

    // 清理：配置 + 同步落库的 LDAP 用户（username/email 按 env 输入兜底清除）
    purge_configs(&pool, "it-ldap").await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username = $1 OR email = $2")
        .bind(&username)
        .bind(format!("{username}@ldap.test"))
        .execute(&pool)
        .await;
}
