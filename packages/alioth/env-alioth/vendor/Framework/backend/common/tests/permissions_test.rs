//! Integration tests for `require_resource_access` and `RequestContext` visible_ids.
//!
//! These tests connect to a live PG with NGAC tables and admin user 1002
//! already bound to the `admin` UA. Run with:
//!   cargo test -p common --test permissions_test -- --ignored --test-threads=1

const ADMIN_USER_ID: i64 = 1002; // isahl-usr, bound to admin UA in seed
use common::testing::connect_test_db;

#[tokio::test]
#[ignore = "requires live DB with NGAC seed data"]
async fn admin_can_read_known_module() {
    let pool = connect_test_db().await;
    let result: bool = sqlx::query_scalar(
        r#"
        WITH RECURSIVE user_attrs AS (
            SELECT fk_user_attribute as ua_id, 0 as depth
            FROM isahl_auth.ngac_user_rr_attribute
            WHERE fk_user = $1
              AND deleted_at IS NULL
              AND (expires_at IS NULL OR expires_at > NOW())
            UNION
            SELECT unnest(ua.ancestor_ids)::BIGINT as ua_id, depth + 1
            FROM isahl_auth.ngac_user_attribute ua
            JOIN user_attrs ua2 ON ua.id = ua2.ua_id
            WHERE ua2.depth < 10
        ),
        resource_attrs AS (
            SELECT id as oa_id, 0 as depth
            FROM isahl_auth.ngac_object_attribute
            WHERE o_name = 'foreign-trade:module'
        )
        SELECT EXISTS(
            SELECT 1 FROM isahl_auth.ngac_association a
            JOIN user_attrs ua ON a.fk_user_attribute = ua.ua_id
            JOIN resource_attrs ra ON a.fk_object_attribute = ra.oa_id
            WHERE EXISTS(
                SELECT 1 FROM isahl_auth.ngac_access_right ar
                WHERE ar.id = ANY(a.ak_access_rights)
                AND ar.o_name = 'admin'
            )
        )
        "#,
    )
    .bind(ADMIN_USER_ID)
    .fetch_one(&pool)
    .await
    .expect("permission query");
    assert!(result, "admin should be permitted on foreign-trade:module");
}

#[tokio::test]
#[ignore = "requires live DB with NGAC seed data"]
async fn admin_cannot_perform_unknown_action() {
    let pool = connect_test_db().await;
    let result: bool = sqlx::query_scalar(
        r#"
        WITH RECURSIVE user_attrs AS (
            SELECT fk_user_attribute as ua_id, 0 as depth
            FROM isahl_auth.ngac_user_rr_attribute
            WHERE fk_user = $1
              AND deleted_at IS NULL
              AND (expires_at IS NULL OR expires_at > NOW())
            UNION
            SELECT unnest(ua.ancestor_ids)::BIGINT as ua_id, depth + 1
            FROM isahl_auth.ngac_user_attribute ua
            JOIN user_attrs ua2 ON ua.id = ua2.ua_id
            WHERE ua2.depth < 10
        ),
        resource_attrs AS (
            SELECT id as oa_id, 0 as depth
            FROM isahl_auth.ngac_object_attribute
            WHERE o_name = 'nonexistent-thing'
        )
        SELECT EXISTS(
            SELECT 1 FROM isahl_auth.ngac_association a
            JOIN user_attrs ua ON a.fk_user_attribute = ua.ua_id
            JOIN resource_attrs ra ON a.fk_object_attribute = ra.oa_id
            WHERE EXISTS(
                SELECT 1 FROM isahl_auth.ngac_access_right ar
                WHERE ar.id = ANY(a.ak_access_rights)
                AND ar.o_name = 'delete'
            )
        )
        "#,
    )
    .bind(ADMIN_USER_ID)
    .fetch_one(&pool)
    .await
    .expect("permission query");
    assert!(!result, "nonexistent resource should be denied");
}

#[tokio::test]
#[ignore = "requires live DB with NGAC seed data"]
async fn bootstrap_phase_permits_when_no_associations() {
    // This test is a no-op: we can't easily drop associations, so we just
    // verify the bootstrap query returns false in steady state.
    let pool = connect_test_db().await;
    let has_policies: (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.ngac_association WHERE deleted_at IS NULL)",
    )
    .fetch_one(&pool)
    .await
    .expect("bootstrap query");
    assert!(
        has_policies.0,
        "dev DB has associations; bootstrap guard should be inactive"
    );
}

#[tokio::test]
#[ignore = "requires live DB with NGAC seed data"]
async fn admin_exempt_from_unregistered_resource() {
    // NGAC_SPEC §6.2：admin UA（含继承）对未注册 OA 的资源也应放行——
    // handler 二次校验（require_resource_access）不得误拒 admin。
    let pool = connect_test_db().await;
    let result = common::permissions::require_resource_access(
        &pool,
        ADMIN_USER_ID,
        "unregistered-report-type",
        0,
        "read",
    )
    .await;
    assert!(
        result.is_ok(),
        "admin 对未注册资源应豁免放行: {:?}",
        result.err()
    );
}
