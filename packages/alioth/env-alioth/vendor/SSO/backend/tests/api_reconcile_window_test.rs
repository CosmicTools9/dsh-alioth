//! 对账导出窗口下界（change `fix-openapi-metering-gaps` :: reconcile-window-retention-bound）
//!
//! `GET /api/admin/api-reconcile?month=YYYY-MM` 的月份早于 `api_usage` 保留期（90 天）时
//! MUST 返回 400 `WINDOW_OUT_OF_RETENTION`（MUST NOT 返回空数据——对账双方会误读为
//! 「该月零调用」）；窗内月份 MUST 保持 200 与既有列集。

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use sqlx::PgPool;

async fn connect() -> PgPool {
    ::common::testing::connect_test_db().await
}

fn test_auth_state() -> AuthState {
    common::test_auth_state()
}

fn mint_token(ast: &AuthState, user_id: i64, email: &str) -> String {
    configure_token_validation(
        "http://localhost:9002".to_string(),
        "http://localhost:9002".to_string(),
    );
    encode_access_token(
        &Claims::new(&user_id.to_string(), email, false),
        &ast.jwt_private_key,
    )
    .expect("mint token")
}

async fn ensure_ngac(pool: &PgPool) {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_policy_class (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL, o_name TEXT, description TEXT,
            created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
            CONSTRAINT ngac_policy_class_pkey PRIMARY KEY (id))",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_user_attribute (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL, o_name TEXT, fk_policy_class BIGINT NOT NULL,
            ancestor_ids BIGINT[] DEFAULT '{}', children_ids BIGINT[] DEFAULT '{}', property JSONB DEFAULT '{}',
            created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
            created_by_id BIGINT, updated_by_id BIGINT, deleted_at TIMESTAMPTZ,
            CONSTRAINT ngac_user_attribute_pkey PRIMARY KEY (id))",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_user_rr_attribute (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL, created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
            updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, created_by_id BIGINT, updated_by_id BIGINT, o_name TEXT,
            fk_user BIGINT, fk_user_attribute BIGINT, assigned_at TIMESTAMPTZ DEFAULT NOW(), expires_at TIMESTAMPTZ,
            conditions JSONB DEFAULT '{}', deleted_at TIMESTAMPTZ,
            CONSTRAINT ngac_user_rr_attribute_pkey PRIMARY KEY (id))",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_policy_class (id, o_name) \
         SELECT isahl.gen_next_zuid(), 'default' \
         WHERE NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_policy_class WHERE o_name = 'default')",
    )
    .execute(pool)
    .await
    .ok();
}

async fn ensure_admin(pool: &PgPool) -> i64 {
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, username, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), 'admin_recon', 'active', true, NOW(), NOW())
         ON CONFLICT (username) DO NOTHING",
    )
    .execute(pool)
    .await
    .ok();
    let admin: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE username = 'admin_recon' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("admin user");
    sqlx::query(
        "INSERT INTO isahl_auth.auth_user_emails (fk_user, email, is_primary, created_at, updated_at) \
         VALUES ($1, 'admin_recon@test.local', TRUE, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(admin)
    .execute(pool)
    .await
    .ok();
    let pc_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("default policy class");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
         VALUES ('admin', $1, NOW(), NOW()) ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING",
    )
    .bind(pc_id)
    .execute(pool)
    .await
    .ok();
    let ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' AND fk_policy_class = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(pc_id)
    .fetch_one(pool)
    .await
    .expect("admin UA");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at) VALUES ($1, $2, NOW(), NOW())",
    )
    .bind(admin)
    .bind(ua)
    .execute(pool)
    .await
    .ok();
    admin
}

/// 超窗月份 ⇒ 400 `WINDOW_OUT_OF_RETENTION`；当月 ⇒ 200 且列集不变。
#[tokio::test]
async fn reconcile_rejects_month_outside_retention() {
    use chrono::{Datelike, Duration};

    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    ensure_ngac(&pool).await;
    let admin = ensure_admin(&pool).await;
    let ast = test_auth_state();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::admin::configure),
    )
    .await;
    let token = mint_token(&ast, admin, "admin_recon@test.local");
    let hdr = format!("Bearer {}", token);

    // 保留期（90 天）之外：150 天前所在月 ⇒ 必然早于「最早可查月份」
    let now = chrono::Utc::now();
    let old = now - Duration::days(150);
    let old_month = format!("{:04}-{:02}", old.year(), old.month());
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/api/admin/api-reconcile?month={}", old_month))
            .insert_header(("Authorization", hdr.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(
        resp.status().as_u16(),
        400,
        "保留期外月份 MUST 400（不得静默给空数据）"
    );
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["error"], "WINDOW_OUT_OF_RETENTION", "body={body:?}");
    assert_eq!(body["retention_days"], 90);
    assert!(
        body["earliest_month"].as_str().is_some(),
        "响应 MUST 给最早可查月份：{body:?}"
    );

    // 当月 ⇒ 200（列集不变）
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/admin/api-reconcile")
            .insert_header(("Authorization", hdr.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200, "当月 MUST 可查");
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(
        body["items"].is_array(),
        "响应 MUST 保留 items 数组：{body:?}"
    );
    assert!(body["month"].as_str().is_some());
}
