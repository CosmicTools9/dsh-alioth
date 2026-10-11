//! 创建者授权派生测试（`ngac_host::creator_derived_permits` +
//! `permissions::require_resource_access` 注入位；`derive-creator-grants-from-lifecycle`）。
//!
//! 口径：`TEST_DATABASE_URL="postgres://<user>@localhost:5432/aliothstudio_test"` + `--ignored`
//! （DB 型；仿 `ngac_ownership_holders_test` 约定，夹具自建自清、前缀 `zz-cg`）。
//!
//! 覆盖：共享 UA 命中（**零物化行**前置）/ 无共享 UA 拒绝 / 创建者 rr 过期拒绝 /
//! prohibition 否决派生授权（deny-overrides 同权）/ 标量谓词与门位一致 + approve 不在集内。

use sqlx::PgPool;

const RT: &str = "zz-cg";

async fn open_test_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    if url.is_none() {
        eprintln!("skipped: DATABASE_URL 未设置");
        return None;
    }
    Some(common::testing::connect_test_db().await)
}

async fn cleanup(pool: &PgPool) {
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user_attribute IN \
                         (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'zz-cg-%')")
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'zz-cg-%'")
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username LIKE 'zz-cg-%'")
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_subj-position\" WHERE notice LIKE 'zz-cg-%'")
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_prohibition WHERE o_name LIKE 'zz-cg-%'")
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE o_name LIKE 'zz-cg-%'")
        .execute(pool)
        .await;
}

async fn make_ua(pool: &PgPool, name: &str) -> i64 {
    let pc: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default'")
            .fetch_one(pool)
            .await
            .expect("default policy class");
    sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class) VALUES ($1, $2) RETURNING id",
    )
    .bind(format!("zz-cg-{name}"))
    .bind(pc)
    .fetch_one(pool)
    .await
    .expect("insert UA")
}

async fn make_user(pool: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id")
        .bind(format!("zz-cg-{name}"))
        .fetch_one(pool)
        .await
        .expect("insert user")
}

async fn bind(pool: &PgPool, user: i64, ua: i64, expires_past: bool) {
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, expires_at) \
         VALUES ($1, $2, CASE WHEN $3 THEN NOW() - INTERVAL '1 hour' ELSE NULL END)",
    )
    .bind(user)
    .bind(ua)
    .bind(expires_past)
    .execute(pool)
    .await
    .expect("insert rr");
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn creator_grant_derived_four_states() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;

    let ua_shared = make_ua(&pool, "ua-shared").await;
    let u_creator = make_user(&pool, "creator").await;
    let u_judge = make_user(&pool, "judge").await;
    let u_other = make_user(&pool, "other").await;

    // 生命周期行（派生载体）：创建者 = u_creator。坐标三元组经 `ontology_binding::resolve`
    // 解析（§6.12 声明即必须；禁硬编码 ZUID，同 ngac_ownership_holders_test 先例）。
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("TX", "FJA", "↓_GG"))
            .await
            .expect("resolve position coords");
    let row_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_subj-position\" \
         (notice, created_by_id, updated_by_id, dk_scene, dk_factor, dk_function) \
         VALUES ($1, $2, $2, $3, $4, $5) RETURNING id",
    )
    .bind("zz-cg-position")
    .bind(u_creator)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(&pool)
    .await
    .expect("insert lifecycle fixture row");

    // 前置断言：零物化（无实例 OA 锚点、无关联行）——本测试证明的全部前提。
    let materialized: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM isahl_auth.ngac_object_attribute \
                  WHERE resource_type = $1 AND fk_resource = $2), \
                (SELECT count(*) FROM isahl_auth.ngac_association a \
                  JOIN isahl_auth.ngac_object_attribute oa \
                    ON oa.id = a.fk_object_attribute \
                   AND oa.resource_type = $1 AND oa.fk_resource = $2)",
    )
    .bind(RT)
    .bind(row_id)
    .fetch_one(&pool)
    .await
    .expect("count materialized");
    assert_eq!(materialized, (0, 0), "前置：零物化锚点/关联");

    // ① 无任何 rr ⇒ 拒绝（fail-closed）
    assert!(
        common::permissions::require_resource_access(&pool, u_judge, RT, row_id, "read")
            .await
            .is_err(),
        "① 无 rr ⇒ 拒绝"
    );

    // ② 共享 UA（创建者与判定者各一条活 rr）⇒ 派生放行（零物化行）
    bind(&pool, u_creator, ua_shared, false).await;
    bind(&pool, u_judge, ua_shared, false).await;
    assert!(
        common::permissions::require_resource_access(&pool, u_judge, RT, row_id, "read")
            .await
            .is_ok(),
        "② 共享 UA ⇒ 派生放行"
    );
    assert!(
        common::permissions::require_resource_access(&pool, u_creator, RT, row_id, "delete")
            .await
            .is_ok(),
        "② 创建者本人（同 UA）⇒ 放行"
    );
    assert!(
        common::permissions::require_resource_access(&pool, u_other, RT, row_id, "read")
            .await
            .is_err(),
        "② 无共享 UA 的第三方 ⇒ 拒绝"
    );

    // ③ 创建者 rr 过期 ⇒ 派生失效
    sqlx::query(
        "UPDATE isahl_auth.ngac_user_rr_attribute SET expires_at = NOW() - INTERVAL '1 hour' \
                 WHERE fk_user = $1 AND fk_user_attribute = $2",
    )
    .bind(u_creator)
    .bind(ua_shared)
    .execute(&pool)
    .await
    .expect("expire creator rr");
    assert!(
        common::permissions::require_resource_access(&pool, u_judge, RT, row_id, "read")
            .await
            .is_err(),
        "③ 创建者 rr 过期 ⇒ 拒绝"
    );
    sqlx::query(
        "UPDATE isahl_auth.ngac_user_rr_attribute SET expires_at = NULL \
                 WHERE fk_user = $1 AND fk_user_attribute = $2",
    )
    .bind(u_creator)
    .bind(ua_shared)
    .execute(&pool)
    .await
    .expect("revive creator rr");

    // ④ prohibition 否决派生授权（deny-overrides 同权）：通配 OA（resource_type='*'）
    //    落在 resource_attrs 闭包内，挂禁止后即使派生命中也拒。
    let read_right: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'read'")
            .fetch_one(&pool)
            .await
            .expect("read right seeded");
    let wildcard_oa: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_object_attribute \
           (o_name, fk_policy_class, resource_type, fk_resource) \
         SELECT 'zz-cg-wildcard', pc.id, '*', 0 \
           FROM isahl_auth.ngac_policy_class pc WHERE pc.o_name = 'default' \
         ON CONFLICT (resource_type, fk_resource) DO UPDATE SET o_name = EXCLUDED.o_name \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("ensure wildcard OA");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_prohibition \
           (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active) \
         VALUES ('zz-cg-prohibit', $1, $2, ARRAY[$3::bigint], TRUE)",
    )
    .bind(ua_shared)
    .bind(wildcard_oa)
    .bind(read_right)
    .execute(&pool)
    .await
    .expect("insert prohibition");

    assert!(
        common::permissions::require_resource_access(&pool, u_judge, RT, row_id, "read")
            .await
            .is_err(),
        "④ prohibition ⇒ deny-overrides 否决派生授权"
    );
    assert!(
        common::permissions::require_resource_access(&pool, u_judge, RT, row_id, "update")
            .await
            .is_ok(),
        "④ prohibition 只禁 read ⇒ update 仍派生放行"
    );

    // ⑤ 标量谓词一致性与 rights 集边界（approve 不在 RIGHTS_CREATOR）
    assert!(
        ngac_host::creator_derived_permits(&pool, u_judge, row_id, "read")
            .await
            .expect("scalar permits"),
        "⑤ 标量谓词命中"
    );
    assert!(
        !ngac_host::creator_derived_permits(&pool, u_judge, row_id, "approve")
            .await
            .expect("scalar permits approve"),
        "⑤ approve 不在创建者集（审核权与创建权分离）"
    );
    assert!(
        !ngac_host::creator_derived_permits(&pool, u_other, row_id, "read")
            .await
            .expect("scalar permits other"),
        "⑤ 无共享 UA ⇒ 标量谓词不命中"
    );

    cleanup(&pool).await;
}
