//! Password reset integration tests
//!
//! Tests the password reset token lifecycle through the DB layer:
//!   - request_reset → token generated for existing email
//!   - confirm_reset → password changed with valid token
//!   - confirm_reset → rejected with expired token
//!
//! Since password reset handlers are not yet implemented as HTTP endpoints,
//! these tests verify the SQL operations that will back them.

mod common;

use ::common::testing::connect_test_db;
use sqlx::PgPool;

/// Create the password_reset_tokens table (Phase 0 addition)
/// and ensure the auth_users table has password_hash.
async fn create_test_tables(pool: &PgPool) {
    // 禁 `DROP TABLE`：本文件多个 `#[tokio::test]` 并行执行且共享测试库——先 drop 会让
    // 并行用例把对方正在用的表删掉（本文件历史抖动的根因）。表结构漂移由建表语句的
    // 幂等形态兜底，真漂移则由后续断言的失败显式暴露，而不是靠删表掩盖。
    // 幂等建表：本文件多个 `#[tokio::test]` 并行执行，裸 `CREATE TABLE` 会让后者报
    // 「已存在」；而**并发** `CREATE TABLE IF NOT EXISTS` 在 PostgreSQL 上仍可能抛
    // `pg_type_typname_nsp_index` 唯一冲突（23505）——两种竞态都容忍：忽略建表结果，
    // 随后以 `to_regclass` 断言表确实可用（真正的判据）。
    let _ = sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.password_reset_tokens (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL,
            user_id BIGINT NOT NULL,
            token_hash VARCHAR NOT NULL,
            expires_at TIMESTAMP WITH TIME ZONE NOT NULL,
            used_at TIMESTAMP WITH TIME ZONE,
            created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
            updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
            CONSTRAINT password_reset_tokens_pkey PRIMARY KEY (id),
            CONSTRAINT password_reset_tokens_user_id_fkey
                FOREIGN KEY (user_id) REFERENCES isahl_auth.auth_users(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await;

    let table_ready: bool =
        sqlx::query_scalar("SELECT to_regclass('isahl_auth.password_reset_tokens') IS NOT NULL")
            .fetch_one(pool)
            .await
            .unwrap_or(false);
    assert!(
        table_ready,
        "password_reset_tokens 表 MUST 可用（并发建表竞态不应阻断测试）"
    );

    // Index for looking up by token
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_password_reset_tokens_token
        ON isahl_auth.password_reset_tokens(token_hash)
        "#,
    )
    .execute(pool)
    .await
    .ok();
}

/// Generate a test user with a known password hash
async fn setup_user(pool: &PgPool, email: &str) -> i64 {
    // Idempotent upsert: tests run in parallel / across runs sharing the DB, so a bare
    // INSERT collides on the unique email/name. Deriving name+username from the email
    // keeps them unique per call; upsert-then-select is race-safe.
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.auth_users (id, name, username, email, password_hash, status, created_at, updated_at)
        VALUES (isahl.gen_next_zuid(), $1, $1, $1, 'old_hash', 'active', NOW(), NOW())
        ON CONFLICT (name) DO NOTHING
        "#,
    )
    .bind(email)
    .execute(pool)
    .await
    .ok();
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id FROM isahl_auth.auth_users WHERE email = $1 LIMIT 1
        "#,
    )
    .bind(email)
    .fetch_one(pool)
    .await
    .expect("Failed to create/find test user")
}

async fn cleanup_by_email(pool: &PgPool, email: &str) {
    sqlx::query(
        r#"
        DELETE FROM isahl_auth.password_reset_tokens
        WHERE user_id IN (SELECT id FROM isahl_auth.auth_users WHERE email = $1)
        "#,
    )
    .bind(email)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE email = $1")
        .bind(email)
        .execute(pool)
        .await
        .ok();
}

fn generate_token() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    let bytes: [u8; 32] = rng.random();
    hex::encode(bytes)
}

#[tokio::test]
async fn test_request_reset_generates_token() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");
    create_test_tables(&pool).await;

    let email = "reset-gen@test.local";
    let user_id = setup_user(&pool, email).await;

    // Simulate request_reset: find user by email, generate token, insert
    let found_user: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1")
            .bind(email)
            .fetch_one(&pool)
            .await
            .expect("User should exist by email");

    assert_eq!(found_user, user_id, "Should find the correct user by email");

    let token = generate_token();
    let exp = chrono::Utc::now() + chrono::Duration::hours(1);

    let token_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl_auth.password_reset_tokens (user_id, token_hash, expires_at, created_at, updated_at)
        VALUES ($1, $2, $3, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(&token)
    .bind(exp)
    .fetch_one(&pool)
    .await
    .expect("Password reset token INSERT should succeed");

    assert!(token_id > 0, "Should generate a valid token ID");

    // Verify token is retrievable
    let (stored_user, stored_token, stored_used): (i64, String, bool) = sqlx::query_as(
        r#"
        SELECT user_id, token_hash, (used_at IS NOT NULL)
        FROM isahl_auth.password_reset_tokens
        WHERE id = $1
        "#,
    )
    .bind(token_id)
    .fetch_one(&pool)
    .await
    .expect("Should read back reset token");

    assert_eq!(
        stored_user, user_id,
        "Token should reference the correct user"
    );
    assert_eq!(stored_token, token, "Token value should match");
    assert!(!stored_used, "Token should not be marked as used");

    cleanup_by_email(&pool, email).await;
}

#[tokio::test]
async fn test_confirm_reset_changes_password() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");
    create_test_tables(&pool).await;

    let email = "reset-confirm@test.local";
    let user_id = setup_user(&pool, email).await;

    // Step 1: Create a reset token
    let token = generate_token();
    let exp = chrono::Utc::now() + chrono::Duration::hours(1);

    let token_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl_auth.password_reset_tokens (user_id, token_hash, expires_at, created_at, updated_at)
        VALUES ($1, $2, $3, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(&token)
    .bind(exp)
    .fetch_one(&pool)
    .await
    .expect("Token INSERT should succeed");

    // Step 2: Look up token (simulating confirm_reset lookup)
    let token_record: Option<(i64, i64, bool, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        r#"
        SELECT id, user_id, (used_at IS NOT NULL), expires_at
        FROM isahl_auth.password_reset_tokens
        WHERE token_hash = $1
          AND used_at IS NULL
          AND expires_at > NOW()
        "#,
    )
    .bind(&token)
    .fetch_optional(&pool)
    .await
    .expect("Token lookup should succeed");

    assert!(
        token_record.is_some(),
        "Valid token should be found for confirm_reset"
    );
    let (found_id, found_user, _, _) = token_record.unwrap();
    assert_eq!(found_id, token_id, "Should find the correct token record");
    assert_eq!(
        found_user, user_id,
        "Token should reference the correct user"
    );

    // Step 3: Update password (simulating confirm_reset)
    let new_hash = "$argon2id$v=19$m=19456,t=2,p=1$newhashvalue$newhashvaluelongstring";
    sqlx::query(
        "UPDATE isahl_auth.auth_users SET password_hash = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind(new_hash)
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("Password update should succeed");

    // Step 4: Mark token as used
    sqlx::query(
        "UPDATE isahl_auth.password_reset_tokens SET used_at = NOW(), updated_at = NOW() WHERE id = $1",
    )
    .bind(token_id)
    .execute(&pool)
    .await
    .expect("Token used flag update should succeed");

    // Step 5: Verify password hash changed
    let stored_hash: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM isahl_auth.auth_users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&pool)
            .await
            .expect("Should query password hash")
            .unwrap();

    assert_eq!(
        stored_hash.as_deref(),
        Some(new_hash),
        "Password hash should be updated to new value"
    );

    // Step 6: Verify token is now marked used and can't be reused
    let reused: Option<(i64,)> = sqlx::query_as(
        r#"
        SELECT id
        FROM isahl_auth.password_reset_tokens
        WHERE token_hash = $1
          AND used_at IS NULL
          AND expires_at > NOW()
        "#,
    )
    .bind(&token)
    .fetch_optional(&pool)
    .await
    .expect("Reuse check query should succeed");

    assert!(
        reused.is_none(),
        "Used token should not be found by confirm_reset lookup"
    );

    cleanup_by_email(&pool, email).await;
}

#[tokio::test]
async fn test_expired_token_is_rejected() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");
    create_test_tables(&pool).await;

    let email = "reset-expired@test.local";
    let user_id = setup_user(&pool, email).await;

    // Create a token that expired 1 hour ago
    let token = generate_token();
    let expired_at = chrono::Utc::now() - chrono::Duration::hours(1);

    sqlx::query(
        r#"
        INSERT INTO isahl_auth.password_reset_tokens (user_id, token_hash, expires_at, created_at, updated_at)
        VALUES ($1, $2, $3, NOW(), NOW())
        "#,
    )
    .bind(user_id)
    .bind(&token)
    .bind(expired_at)
    .execute(&pool)
    .await
    .expect("Expired token INSERT should succeed");

    // Attempt to look up the expired token (simulating confirm_reset with expired)
    let token_record: Option<(i64, i64)> = sqlx::query_as(
        r#"
        SELECT id, user_id
        FROM isahl_auth.password_reset_tokens
        WHERE token_hash = $1
          AND used_at IS NULL
          AND expires_at > NOW()
        "#,
    )
    .bind(&token)
    .fetch_optional(&pool)
    .await
    .expect("Expired token lookup should succeed");

    assert!(
        token_record.is_none(),
        "Expired token should not be found by confirm_reset lookup"
    );

    // Verify the original password hash is unchanged
    let original_hash: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM isahl_auth.auth_users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&pool)
            .await
            .expect("Should query password hash")
            .unwrap();

    assert_eq!(
        original_hash.as_deref(),
        Some("old_hash"),
        "Password should remain unchanged when using expired token"
    );

    cleanup_by_email(&pool, email).await;
}

#[tokio::test]
async fn test_request_reset_nonexistent_email_returns_empty() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    let nonexistent_email = "nonexistent@test.local";

    // Simulate request_reset: try to find user by email
    let found: Option<i64> =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE email = $1")
            .bind(nonexistent_email)
            .fetch_optional(&pool)
            .await
            .expect("Lookup should succeed");

    assert!(
        found.is_none(),
        "Should not find user for nonexistent email"
    );
}
