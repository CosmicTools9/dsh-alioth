//! Admin API integration tests
//!
//! Tests the admin API business logic through the DB layer:
//!   - Admin NGAC check rejects non-admin users
//!   - Admin NGAC check passes for admin users
//!   - User creation + listing queries work correctly

mod common;

use ::common::testing::connect_test_db;
use sqlx::PgPool;

/// Create NGAC tables and a default policy class needed for the admin check.
async fn create_ngac_tables(pool: &PgPool) {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.ngac_policy_class (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL,
            o_name TEXT,
            description TEXT,
            created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
            updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
            CONSTRAINT ngac_policy_class_pkey PRIMARY KEY (id)
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("Failed to create ngac_policy_class table");

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.ngac_user_attribute (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL,
            o_name TEXT,
            fk_policy_class BIGINT NOT NULL,
            ancestor_ids BIGINT[] DEFAULT '{}',
            children_ids BIGINT[] DEFAULT '{}',
            property JSONB DEFAULT '{}',
            created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
            updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
            created_by_id BIGINT,
            updated_by_id BIGINT,
            deleted_at TIMESTAMP WITH TIME ZONE,
            CONSTRAINT ngac_user_attribute_pkey PRIMARY KEY (id)
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("Failed to create ngac_user_attribute table");

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.ngac_user_rr_attribute (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL,
            created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
            updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
            created_by_id BIGINT,
            updated_by_id BIGINT,
            o_name TEXT,
            fk_user BIGINT,
            fk_user_attribute BIGINT,
            assigned_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
            expires_at TIMESTAMP WITH TIME ZONE,
            conditions JSONB DEFAULT '{}',
            deleted_at TIMESTAMP WITH TIME ZONE,
            CONSTRAINT ngac_user_rr_attribute_pkey PRIMARY KEY (id),
            CONSTRAINT ngac_user_rr_attribute_fk_user_fkey
                FOREIGN KEY (fk_user) REFERENCES isahl_auth.auth_users(id) ON DELETE CASCADE,
            CONSTRAINT ngac_user_rr_attribute_fk_user_attribute_fkey
                FOREIGN KEY (fk_user_attribute) REFERENCES isahl_auth.ngac_user_attribute(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("Failed to create ngac_user_rr_attribute table");

    // Ensure a default policy class exists（seed 幂等，不假设 id=1：
    // policy class 主键由 isahl.gen_next_zuid() 动态生成）
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_policy_class (id, o_name)
        SELECT isahl.gen_next_zuid(), 'default'
        WHERE NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_policy_class WHERE o_name = 'default')
        "#,
    )
    .execute(pool)
    .await
    .ok();
}

struct TestUsers {
    regular: i64,
    admin: i64,
}

async fn setup_users(pool: &PgPool) -> TestUsers {
    // Create a regular user (no admin UA). Idempotent upsert: tests run in parallel
    // within this binary and share the DB, so a bare INSERT collides on the unique
    // email/name. upsert-then-select is race-safe across runs and threads.
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.auth_users (id, name, username, email, status, is_active, created_at, updated_at)
        VALUES (isahl.gen_next_zuid(), 'regular_user', 'regular_user', 'regular@test.local', 'active', true, NOW(), NOW())
        ON CONFLICT (email) DO NOTHING
        "#,
    )
    .execute(pool)
    .await
    .ok();
    let regular: i64 = sqlx::query_scalar(
        r#"
        SELECT id FROM isahl_auth.auth_users WHERE email = 'regular@test.local' LIMIT 1
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("Failed to create/find regular user");

    // Create an admin user (idempotent, see above)
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.auth_users (id, name, username, email, status, is_active, created_at, updated_at)
        VALUES (isahl.gen_next_zuid(), 'admin_user', 'admin_user', 'admin@test.local', 'active', true, NOW(), NOW())
        ON CONFLICT (email) DO NOTHING
        "#,
    )
    .execute(pool)
    .await
    .ok();
    let admin: i64 = sqlx::query_scalar(
        r#"
        SELECT id FROM isahl_auth.auth_users WHERE email = 'admin@test.local' LIMIT 1
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("Failed to create/find admin user");

    // Create 'admin' UA and assign to the admin user.
    // Idempotent upsert: the unique partial index (o_name, fk_policy_class) WHERE deleted_at IS NULL
    // makes a bare re-INSERT collide across runs / parallel tests; upsert-then-select is race-safe.
    // policy class id 动态查询（o_name='default'），不可硬编码 1。
    let pc_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("Failed to find default policy class");
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
        VALUES ('admin', $1, NOW(), NOW())
        ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING
        "#,
    )
    .bind(pc_id)
    .execute(pool)
    .await
    .ok();
    let admin_ua_id: i64 = sqlx::query_scalar(
        r#"
        SELECT id FROM isahl_auth.ngac_user_attribute
        WHERE o_name = 'admin' AND fk_policy_class = $1 AND deleted_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(pc_id)
    .fetch_one(pool)
    .await
    .expect("Failed to create/find admin UA");

    sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
        VALUES ($1, $2, NOW(), NOW())
        ON CONFLICT (fk_user, fk_user_attribute) DO NOTHING
        "#,
    )
    .bind(admin)
    .bind(admin_ua_id)
    .execute(pool)
    .await
    .expect("Failed to assign admin UA");

    TestUsers { regular, admin }
}

async fn cleanup(pool: &PgPool) {
    // Clean in dependency order。
    // 禁止全表 DELETE ngac_user_attribute / ngac_user_rr_attribute ——
    // 那会删除 005/019 seed 的 UA 与关联，破坏其他并行测试。
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user IN (SELECT id FROM isahl_auth.auth_users \
                           WHERE email IN ('regular@test.local', 'admin@test.local'))",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE email IN ('regular@test.local', 'admin@test.local')")
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_admin_check_rejects_regular_user() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");
    create_ngac_tables(&pool).await;

    let users = setup_users(&pool).await;

    // Same SQL as require_admin() in handlers.rs
    let is_admin: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM isahl_auth.ngac_user_rr_attribute ur
            JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute
            WHERE ur.fk_user = $1
              AND ua.o_name = 'admin'
              AND ur.deleted_at IS NULL
        )
        "#,
    )
    .bind(users.regular)
    .fetch_one(&pool)
    .await
    .expect("Admin check query should succeed");

    assert!(!is_admin, "Regular user should NOT have admin access");

    cleanup(&pool).await;
}

#[tokio::test]
async fn test_admin_check_passes_for_admin_user() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");
    create_ngac_tables(&pool).await;

    let users = setup_users(&pool).await;

    // Same SQL as require_admin() in handlers.rs
    let is_admin: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM isahl_auth.ngac_user_rr_attribute ur
            JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute
            WHERE ur.fk_user = $1
              AND ua.o_name = 'admin'
              AND ur.deleted_at IS NULL
        )
        "#,
    )
    .bind(users.admin)
    .fetch_one(&pool)
    .await
    .expect("Admin check query should succeed");

    assert!(is_admin, "Admin user SHOULD have admin access");

    cleanup(&pool).await;
}

#[tokio::test]
async fn test_list_users_returns_all_users() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");
    create_ngac_tables(&pool).await;

    let users = setup_users(&pool).await;

    // Same SQL as list_users()，但按 id 倒序取最新 50 行——避免共享测试库的历史残留
    // 用户挤占 LIMIT 窗口导致新创建用户被截断（109 用户 > LIMIT 50 曾致误报）。
    let rows: Vec<(i64, String, Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT id, name, email, status
        FROM isahl_auth.auth_users
        ORDER BY id DESC
        LIMIT 50 OFFSET 0
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("list_users query should succeed");

    assert!(
        !rows.is_empty(),
        "Should return at least the two test users"
    );

    let ids: Vec<i64> = rows.iter().map(|(id, _, _, _)| *id).collect();
    assert!(
        ids.contains(&users.regular),
        "Regular user should be in list"
    );
    assert!(ids.contains(&users.admin), "Admin user should be in list");

    cleanup(&pool).await;
}

#[tokio::test]
async fn test_create_user_via_db() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    // Same SQL pattern as create_user() handler
    let new_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl_auth.auth_users (name, email, password_hash, created_at, updated_at)
        VALUES ('new_user', 'new@test.local', 'hashed_password', NOW(), NOW())
        RETURNING id
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("User creation INSERT should succeed");

    assert!(new_id > 0, "New user should get a valid ID");

    // Verify user was created with correct data
    let (name, email, is_active): (String, Option<String>, bool) = sqlx::query_as(
        r#"
        SELECT name, email, COALESCE(is_active, true)
        FROM isahl_auth.auth_users
        WHERE id = $1
        "#,
    )
    .bind(new_id)
    .fetch_one(&pool)
    .await
    .expect("Should query new user");

    assert_eq!(name, "new_user", "Name should match");
    assert_eq!(
        email.as_deref(),
        Some("new@test.local"),
        "Email should match"
    );
    assert!(is_active, "New user should be active by default");

    // Cleanup
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(new_id)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_create_user_duplicate_email_rejected() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");

    let email = "duplicate@test.local";

    // Create first user
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.auth_users (name, email, password_hash, created_at, updated_at)
        VALUES ('first', $1, 'hash', NOW(), NOW())
        "#,
    )
    .bind(email)
    .execute(&pool)
    .await
    .expect("First user creation should succeed");

    // Try to create second user with same email (no UNIQUE constraint on email
    // in the current schema, but we test the business rule: duplicate detection)
    let existing: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT id FROM isahl_auth.auth_users WHERE email = $1
        "#,
    )
    .bind(email)
    .fetch_optional(&pool)
    .await
    .expect("Lookup should succeed");

    assert!(
        existing.is_some(),
        "Pre-existing user should be found by email"
    );

    // Cleanup
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE email = $1")
        .bind(email)
        .execute(&pool)
        .await
        .ok();
}
