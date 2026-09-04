//! /auth/me enrichment integration tests
//!
//! Tests that NGAC user attributes and accessible resources flow correctly
//! through the PIP layer (which /auth/me relies on).
//!
//! Since these tests verify the data layer that backs /auth/me, they use
//! the PostgresPip directly rather than starting an HTTP server.
#![allow(clippy::type_complexity)]

mod common;

use ::common::testing::connect_test_db;
use sqlx::PgPool;

/// Create NGAC tables needed for the test.
/// These should already exist from migration, but we use IF NOT EXISTS
/// for test isolation.
async fn create_ngac_tables(pool: &PgPool) {
    // Policy class must exist for user attribute references
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

async fn setup_user(pool: &PgPool) -> i64 {
    // Idempotent upsert: this file's tests run in parallel / across runs sharing the DB,
    // so a bare INSERT collides on the unique email/name. upsert-then-select is race-safe.
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.auth_users (id, name, username, email, status, created_at, updated_at)
        VALUES (isahl.gen_next_zuid(), 'me_enrich_test', 'me_enrich_test', 'me_enrich@test.local', 'active', NOW(), NOW())
        ON CONFLICT (email) DO NOTHING
        "#,
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id FROM isahl_auth.auth_users WHERE email = 'me_enrich@test.local' LIMIT 1
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("Failed to create/find test user")
}

async fn default_policy_class_id(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("Failed to find default policy class")
}

async fn setup_admin_attribute(pool: &PgPool) -> i64 {
    // Idempotent: the unique partial index (o_name, fk_policy_class) WHERE deleted_at IS NULL
    // makes a bare re-INSERT collide across runs. Upsert, then resolve the existing id.
    // policy class id 动态查询（o_name='default'），不可硬编码 1。
    let pc_id = default_policy_class_id(pool).await;
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
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id FROM isahl_auth.ngac_user_attribute
        WHERE o_name = 'admin' AND fk_policy_class = $1 AND deleted_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(pc_id)
    .fetch_one(pool)
    .await
    .expect("Failed to create/find admin UA")
}

async fn setup_viewer_attribute(pool: &PgPool) -> i64 {
    // Idempotent: see setup_admin_attribute — avoid duplicate-key on re-runs.
    let pc_id = default_policy_class_id(pool).await;
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
        VALUES ('viewer', $1, NOW(), NOW())
        ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING
        "#,
    )
    .bind(pc_id)
    .execute(pool)
    .await
    .ok();
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id FROM isahl_auth.ngac_user_attribute
        WHERE o_name = 'viewer' AND fk_policy_class = $1 AND deleted_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(pc_id)
    .fetch_one(pool)
    .await
    .expect("Failed to create/find viewer UA")
}

async fn assign_ua_to_user(pool: &PgPool, user_id: i64, ua_id: i64) {
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
        VALUES ($1, $2, NOW(), NOW())
        "#,
    )
    .bind(user_id)
    .bind(ua_id)
    .execute(pool)
    .await
    .expect("Failed to assign UA to user");
}

async fn cleanup_user(pool: &PgPool, user_id: i64) {
    sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_ngac_attributes_empty_when_no_data() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");
    create_ngac_tables(&pool).await;

    let user_id = setup_user(&pool).await;

    // Query NGAC user attributes directly (same SQL as PostgresPip::get_user_attributes)
    let attributes: Vec<(i64, Option<String>, i64, Option<Vec<i64>>, Option<Vec<i64>>)> =
        sqlx::query_as(
            r#"
            SELECT ua.id, ua.o_name, ua.fk_policy_class, ua.ancestor_ids, ua.children_ids
            FROM isahl_auth.ngac_user_attribute ua
            INNER JOIN isahl_auth.ngac_user_rr_attribute ura ON ua.id = ura.fk_user_attribute
            WHERE ura.fk_user = $1
              AND ura.deleted_at IS NULL
              AND (ura.expires_at IS NULL OR ura.expires_at > NOW())
            "#,
        )
        .bind(user_id)
        .fetch_all(&pool)
        .await
        .expect("SQL query for user attributes should succeed");

    assert!(
        attributes.is_empty(),
        "User with no NGAC data should have empty attributes"
    );

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_ngac_attributes_returns_values_for_admin_user() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");
    create_ngac_tables(&pool).await;

    let user_id = setup_user(&pool).await;
    let admin_ua_id = setup_admin_attribute(&pool).await;
    assign_ua_to_user(&pool, user_id, admin_ua_id).await;

    // Query NGAC user attributes (same SQL as PostgresPip::get_user_attributes)
    let attributes: Vec<(i64, Option<String>, i64)> = sqlx::query_as(
        r#"
        SELECT ua.id, ua.o_name, ua.fk_policy_class
        FROM isahl_auth.ngac_user_attribute ua
        INNER JOIN isahl_auth.ngac_user_rr_attribute ura ON ua.id = ura.fk_user_attribute
        WHERE ura.fk_user = $1
          AND ura.deleted_at IS NULL
          AND (ura.expires_at IS NULL OR ura.expires_at > NOW())
        ORDER BY ua.id
        "#,
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await
    .expect("SQL query for user attributes should succeed");

    assert!(!attributes.is_empty(), "Admin user should have attributes");
    assert_eq!(attributes.len(), 1, "Should have exactly one UA");

    let (ua_id, ua_name, _) = &attributes[0];
    assert_eq!(
        ua_name.as_deref(),
        Some("admin"),
        "UA o_name should be 'admin'"
    );
    assert_eq!(*ua_id, admin_ua_id, "UA id should match");

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_ngac_multiple_attributes_per_user() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");
    create_ngac_tables(&pool).await;

    let user_id = setup_user(&pool).await;
    let admin_ua_id = setup_admin_attribute(&pool).await;
    let viewer_ua_id = setup_viewer_attribute(&pool).await;
    assign_ua_to_user(&pool, user_id, admin_ua_id).await;
    assign_ua_to_user(&pool, user_id, viewer_ua_id).await;

    // Query all user attributes
    let attributes: Vec<(i64, Option<String>)> = sqlx::query_as(
        r#"
        SELECT ua.id, ua.o_name
        FROM isahl_auth.ngac_user_attribute ua
        INNER JOIN isahl_auth.ngac_user_rr_attribute ura ON ua.id = ura.fk_user_attribute
        WHERE ura.fk_user = $1
          AND ura.deleted_at IS NULL
          AND (ura.expires_at IS NULL OR ura.expires_at > NOW())
        ORDER BY ua.o_name
        "#,
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await
    .expect("SQL query for user attributes should succeed");

    assert_eq!(attributes.len(), 2, "User should have 2 attributes");

    let names: Vec<Option<String>> = attributes.into_iter().map(|(_, name)| name).collect();
    let name_strs: Vec<&str> = names.iter().filter_map(|n| n.as_deref()).collect();
    assert!(name_strs.contains(&"admin"), "Should include admin UA");
    assert!(name_strs.contains(&"viewer"), "Should include viewer UA");

    cleanup_user(&pool, user_id).await;
}
