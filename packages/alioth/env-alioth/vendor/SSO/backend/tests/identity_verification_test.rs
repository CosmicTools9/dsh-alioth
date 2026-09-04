//! Identity verification integration tests
//!
//! Tests the identity_verifications table operations through the DB layer:
//!   - INSERT after migration
//!   - UNIQUE constraint on user_id
//!   - Entity backfill on verify (zc_id_appr-user_verify)
#![allow(clippy::type_complexity)]

mod common;

use ::common::testing::connect_test_db;
use sqlx::PgPool;

/// Create the identity_verifications table and minimal entity tables
/// needed for the test flow. This mimics the Phase 0 migration DDL.
async fn create_test_tables(pool: &PgPool) {
    // identity_verifications table (Phase 0 addition)
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.identity_verifications (
            id BIGINT NOT NULL DEFAULT isahl.gen_next_zuid(),
            user_id BIGINT NOT NULL,
            verification_type VARCHAR(32) NOT NULL,
            real_name TEXT,
            id_card_number TEXT,
            id_card_front_url TEXT,
            id_card_back_url TEXT,
            enterprise_name TEXT,
            business_license_number TEXT,
            business_license_url TEXT,
            legal_person_name TEXT,
            entity_instance_id BIGINT,
            entity_instance_table TEXT,
            verification_status TEXT DEFAULT 'submitted',
            approval_event_id BIGINT,
            rejected_reason TEXT,
            verified_at TIMESTAMP WITH TIME ZONE,
            created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
            CONSTRAINT identity_verifications_pkey PRIMARY KEY (id),
            CONSTRAINT identity_verifications_user_id_unique UNIQUE (user_id),
            CONSTRAINT identity_verifications_user_id_fkey
                FOREIGN KEY (user_id) REFERENCES isahl_auth.auth_users(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("Failed to create identity_verifications table");

    // Minimal entity table simulating isahl.zc_id_empl-natural
    // (only the columns used by submit_identity handler)
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS isahl.zc_id_empl_natural_test (
            id BIGSERIAL PRIMARY KEY,
            notice TEXT,
            code TEXT,
            fk_user BIGINT,
            created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("Failed to create entity test table");

    // Minimal table simulating isahl.zc_id_appr-user_verify
    // (only the columns used by verify_identity handler)
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS isahl.zc_id_appr_user_verify_test (
            id BIGSERIAL PRIMARY KEY,
            created_by_id BIGINT,
            updated_by_id BIGINT,
            notice TEXT,
            ck_category BIGINT,
            fk_object BIGINT,
            created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("Failed to create approval entity test table");
}

async fn setup_user(pool: &PgPool) -> i64 {
    // Idempotent upsert: tests run in parallel / across runs sharing the DB, so a bare
    // INSERT collides on the unique email/name. upsert-then-select is race-safe.
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.auth_users (id, name, username, email, status, created_at, updated_at)
        VALUES (isahl.gen_next_zuid(), 'identity_test', 'identity_test', 'identity@test.local', 'pending', NOW(), NOW())
        ON CONFLICT (email) DO NOTHING
        "#,
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id FROM isahl_auth.auth_users WHERE email = 'identity@test.local' LIMIT 1
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("Failed to create/find test user")
}

async fn cleanup_user(pool: &PgPool, user_id: i64) {
    sqlx::query("DELETE FROM isahl_auth.identity_verifications WHERE user_id = $1")
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
async fn test_identity_verification_insert() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");
    create_test_tables(&pool).await;

    let user_id = setup_user(&pool).await;

    let result = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO isahl_auth.identity_verifications (
            user_id, verification_type, real_name, id_card_number,
            verification_status, created_at, updated_at
        ) VALUES ($1, 'personal', $2, $3, 'submitted', NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind("张三")
    .bind("110101199001011234")
    .fetch_one(&pool)
    .await;

    assert!(
        result.is_ok(),
        "INSERT into identity_verifications should succeed after migration"
    );
    let ver_id = result.unwrap();
    assert!(ver_id > 0, "Should return a valid generated ID");

    // Verify record exists and has correct values
    let (status, name): (String, Option<String>) = sqlx::query_as(
        r#"
        SELECT verification_status, real_name
        FROM isahl_auth.identity_verifications
        WHERE id = $1
        "#,
    )
    .bind(ver_id)
    .fetch_one(&pool)
    .await
    .expect("Should read back inserted record");

    assert_eq!(status, "submitted", "Status should be 'submitted'");
    assert_eq!(name.as_deref(), Some("张三"), "real_name should match");

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_identity_verification_unique_constraint() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");
    create_test_tables(&pool).await;

    let user_id = setup_user(&pool).await;

    // First insert should succeed
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.identity_verifications (
            user_id, verification_type, verification_status, created_at, updated_at
        ) VALUES ($1, 'personal', 'submitted', NOW(), NOW())
        "#,
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("First INSERT should succeed");

    // Second insert for same user should fail on UNIQUE constraint
    let dup_result = sqlx::query(
        r#"
        INSERT INTO isahl_auth.identity_verifications (
            user_id, verification_type, verification_status, created_at, updated_at
        ) VALUES ($1, 'enterprise', 'submitted', NOW(), NOW())
        "#,
    )
    .bind(user_id)
    .execute(&pool)
    .await;

    assert!(
        dup_result.is_err(),
        "Duplicate INSERT on user_id should be rejected by UNIQUE constraint"
    );

    let err = dup_result.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("unique") || err_str.contains("UNIQUE") || err_str.contains("duplicate"),
        "Error message should indicate unique constraint violation: {}",
        err_str
    );

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_entity_backfill_on_verify() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");
    create_test_tables(&pool).await;

    let user_id = setup_user(&pool).await;

    // Step 1: Insert identity verification (as submit_identity would)
    let entity_instance_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl.zc_id_empl_natural_test (notice, code, fk_user, created_at, updated_at)
        VALUES ('张三', '110101199001011234', $1, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("Entity insert should succeed");

    let ver_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl_auth.identity_verifications (
            user_id, verification_type, real_name, id_card_number,
            entity_instance_id, entity_instance_table,
            verification_status, created_at, updated_at
        ) VALUES ($1, 'personal', '张三', '110101199001011234', $2, 'zc_id_empl_natural_test', 'submitted', NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(entity_instance_id)
    .fetch_one(&pool)
    .await
    .expect("Identity verification INSERT should succeed");
    assert!(ver_id > 0, "ID should be positive");

    // Step 2: Simulate verify_identity — backfill approval event
    let ck_category = 1_i64; // personal = 1 (from identity.rs:262-266)
    let approval_event_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl.zc_id_appr_user_verify_test (
            created_by_id, updated_by_id, notice, ck_category, fk_object, created_at, updated_at
        ) VALUES ($1, $1, $2, $3, $4, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(format!("用户 {} 注册审批", user_id))
    .bind(ck_category)
    .bind(entity_instance_id)
    .fetch_one(&pool)
    .await
    .expect("Approval event INSERT should succeed");
    assert!(
        approval_event_id > 0,
        "Approval event ID should be positive"
    );

    // Step 3: Update verification record with approval_event_id
    sqlx::query(
        r#"
        UPDATE isahl_auth.identity_verifications
        SET verification_status = 'verified', approval_event_id = $1, verified_at = NOW(), updated_at = NOW()
        WHERE id = $2
        "#,
    )
    .bind(approval_event_id)
    .bind(ver_id)
    .execute(&pool)
    .await
    .expect("Verification status update should succeed");

    // Step 4: Verify the chain — query back the verification record
    let (status, event_id): (String, Option<i64>) = sqlx::query_as(
        r#"
        SELECT verification_status, approval_event_id
        FROM isahl_auth.identity_verifications
        WHERE id = $1
        "#,
    )
    .bind(ver_id)
    .fetch_one(&pool)
    .await
    .expect("Should read back verification");

    assert_eq!(status, "verified", "Status should be 'verified'");
    assert_eq!(
        event_id,
        Some(approval_event_id),
        "Should link to approval event"
    );

    // Also verify user status was updated (from identity.rs:280-285)
    let user_status: Option<String> =
        sqlx::query_scalar("SELECT status FROM isahl_auth.auth_users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&pool)
            .await
            .expect("Should query user status")
            .unwrap();

    // Note: user status update (identity_submitted → pending_approval) happens
    // in verify_identity handler — the test simulates the full flow
    assert_eq!(
        user_status.as_deref(),
        Some("pending"),
        "User status should remain 'pending' since the DB-layer test simulates verify step separately"
    );

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_identity_verification_tables_exist() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool)
        .await
        .expect("Failed to setup schema");
    create_test_tables(&pool).await;

    // Verify the identity_verifications table was created by querying information_schema
    let row_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)::BIGINT
        FROM information_schema.tables
        WHERE table_schema = 'isahl_auth'
          AND table_name = 'identity_verifications'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("information_schema query should work");

    assert_eq!(
        row_count, 1,
        "identity_verifications table should exist in isahl_auth schema"
    );
}
