//! Runtime tests for `require_resource_access` against a real PG.
//!
//! These tests connect to aliothstudio_test and exercise the actual helper
//! function. They require that the test fixture is in place — see
//! `docs/specs/NGAC_SPEC.md` for the bootstrap script:
//!   1. reset-db.sh --test --reset
//!   2. Apply 007_ngac_extension_tables.sql
//!   3. Apply 005_seed_ngac_and_policy_class.sql
//!   4. Bootstrap: 1 admin user (id=1002) + admin UA binding + foreign-trade module OA + association
//!
//! Run with:
//!   cargo test -p common --test permissions_runtime_test -- --ignored --test-threads=1

use common::permissions::require_resource_access;
use sqlx::PgPool;

const ADMIN_USER_ID: i64 = 1002;
const FOREIGN_TRADE_OA_FK_RESOURCE: i64 = 1;

async fn connect() -> PgPool {
    {
        let database_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| common::testing::test_database_url());
        sqlx::PgPool::connect(&database_url)
            .await
            .expect("connect_test_db failed")
    }
}

#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn admin_can_admin_foreign_trade_module() {
    let pool = connect().await;
    let result = require_resource_access(
        &pool,
        ADMIN_USER_ID,
        "module",
        FOREIGN_TRADE_OA_FK_RESOURCE,
        "admin",
    )
    .await;
    assert!(
        result.is_ok(),
        "admin should be able to admin foreign-trade module, got: {:?}",
        result
    );
}

#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn admin_cannot_perform_unknown_action() {
    let pool = connect().await;
    let result = require_resource_access(
        &pool,
        ADMIN_USER_ID,
        "nonexistent-resource",
        99999,
        "delete",
    )
    .await;
    assert!(result.is_err(), "nonexistent resource should be denied");
}

#[tokio::test]
#[ignore = "requires live test DB with NGAC seed"]
async fn zero_user_id_is_denied() {
    let pool = connect().await;
    let result =
        require_resource_access(&pool, 0, "module", FOREIGN_TRADE_OA_FK_RESOURCE, "admin").await;
    assert!(result.is_err(), "user_id=0 should not pass any NGAC check");
}
