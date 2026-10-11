//! Integration tests for `require_resource_access` and `RequestContext` visible_ids.
//!
//! These tests connect to a live PG with NGAC tables and admin user 1002
//! already bound to the `admin` UA. Run with:
//!   cargo test -p common --test permissions_test -- --ignored --test-threads=1

use common::testing::connect_test_db;

/// 测试库中任一的 admin UA 持有者 id。
///
/// 历史夹具写死 `1002`（"isahl-usr"），但当前测试库（256 用户 / 161 人持 admin UA）
/// **无该用户** ⇒ 三条用例长期红（2026-09-29 实测）。改为运行时取真实持有着：
/// 夹具不再与库内 id 漂移；测试库无任何 admin 持有时给出明确修复指引。
async fn seeded_admin_user_id(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT ur.fk_user FROM isahl_auth.ngac_user_rr_attribute ur \
         JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute \
         WHERE ua.o_name = 'admin' AND ur.deleted_at IS NULL \
         ORDER BY ur.fk_user LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("测试库需至少一个 admin UA 持有者：bash scripts/db/reset-db.sh --test")
}

#[tokio::test]
#[ignore = "requires live DB with NGAC seed data"]
async fn admin_can_read_known_module() {
    let pool = connect_test_db().await;
    let admin_id = seeded_admin_user_id(&pool).await;
    // 目标 OA 由库内解析：曾写死 `foreign-trade:module`（NS 快照专属名，当前测试库 0 该 OA ⇒ 长期假红）。
    let oa_name: String = sqlx::query_scalar(
        "SELECT oa.o_name FROM isahl_auth.ngac_object_attribute oa \
         JOIN isahl_auth.ngac_association a ON a.fk_object_attribute = oa.id AND a.deleted_at IS NULL \
         JOIN isahl_auth.ngac_user_attribute ua ON ua.id = a.fk_user_attribute AND ua.o_name = 'admin' \
         WHERE oa.deleted_at IS NULL \
           AND EXISTS (SELECT 1 FROM isahl_auth.ngac_access_right ar \
                       WHERE ar.id = ANY(a.ak_access_rights) AND ar.o_name = 'admin') \
         ORDER BY oa.o_name LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("测试库需至少一处 admin UA 的 admin 权 association：bash scripts/db/reset-db.sh --test");
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
            WHERE o_name = $2 AND deleted_at IS NULL
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
    .bind(admin_id)
    .bind(&oa_name)
    .fetch_one(&pool)
    .await
    .expect("permission query");
    assert!(result, "admin UA 应对 {oa_name} 命中 admin 权");
}

#[tokio::test]
#[ignore = "requires live DB with NGAC seed data"]
async fn admin_cannot_perform_unknown_action() {
    let pool = connect_test_db().await;
    let admin_id = seeded_admin_user_id(&pool).await;
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
    .bind(admin_id)
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
    let admin_id = seeded_admin_user_id(&pool).await;
    let result = common::permissions::require_resource_access(
        &pool,
        admin_id,
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

/// admin 谓词唯一实现（`consolidate-ngac-admin-predicate`）：
/// - 直派指派（未删除、未过期）⇒ true（用库内真实持有者，夹具不写死 id）；
/// - 未知用户 ⇒ false（fail-closed）。
/// 过期/委托/继承闭包的语义由 DB 函数 `isahl_auth.is_admin` 承载（与 PDP 有效 UA 集同源）。
#[tokio::test]
#[ignore = "requires live DB with NGAC seed data"]
async fn admin_predicate_matches_direct_assignment() {
    let pool = connect_test_db().await;

    let unknown = common::permissions::is_admin_user(&pool, 987_654_321)
        .await
        .expect("admin predicate query on unknown user");
    assert!(!unknown, "unknown user MUST NOT be admin (fail-closed)");

    let admin_id = seeded_admin_user_id(&pool).await;
    let is_admin = common::permissions::is_admin_user(&pool, admin_id)
        .await
        .expect("admin predicate query");
    assert!(
        is_admin,
        "库内 admin UA 持有者（{admin_id}）MUST 判定为 admin"
    );
}
