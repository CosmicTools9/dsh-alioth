//! 属权派生 UA 持有者反向解析测试（`common::ngac_org::ownership_derived_user_holders`，
//! NGAC_SPEC §2.2.6，2026-09-30）。
//!
//! 口径：`TEST_DATABASE_URL="postgres://<user>@localhost:5432/aliothstudio_test"` +
//! `--ignored`（DB 型；仿 `permissions_runtime_test` 约定，夹具自建自清、前缀 `zz-own-h`）。
//!
//! 覆盖：owner（`created_by_id`）/ benefit（`ak_benefit_user`）/ permit（`ak_permit_user`）/
//! access（`ak_access_user`）四路各自命中；无关 UA ⇒ 空；`enabled = false` ⇒ 不命中。

use sqlx::PgPool;

const RT: &str = "zz-own-h";

async fn open_test_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    if url.is_none() {
        eprintln!("skipped: DATABASE_URL 未设置");
        return None;
    }
    Some(common::testing::connect_test_db().await)
}

async fn cleanup(pool: &PgPool) {
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_ownership_policy WHERE resource_type = $1")
        .bind(RT)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_subj-position\" WHERE notice LIKE 'zz-own-h%'")
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'zz-own-h%'")
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username LIKE 'zz-own-h%'")
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
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class) \
         VALUES ($1, $2) RETURNING id",
    )
    .bind(format!("zz-own-h-{name}"))
    .bind(pc)
    .fetch_one(pool)
    .await
    .expect("insert UA")
}

async fn make_user(pool: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id")
        .bind(format!("zz-own-h-{name}"))
        .fetch_one(pool)
        .await
        .expect("insert user")
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn ownership_holders_resolves_four_paths_and_ignores_disabled() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;

    let ua_owner = make_ua(&pool, "ua-owner").await;
    let ua_benefit = make_ua(&pool, "ua-benefit").await;
    let ua_permit = make_ua(&pool, "ua-permit").await;
    let ua_access = make_ua(&pool, "ua-access").await;
    let u_creator = make_user(&pool, "creator").await;
    let u_listed = make_user(&pool, "listed").await;

    // 生命周期行（属权派生 join 的载体）：创建者 = u_creator，三列 ak_* 含 u_listed。
    // 坐标三元组经 `ontology_binding::resolve` 解析（§6.12 声明即必须；禁硬编码 ZUID，
    // 同 `actor_identity_view_scope_test` 先例；`dk_function` 同时满足 §4.3.3 派生源要求）。
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("TX", "FJA", "↓_GG"))
            .await
            .expect("resolve position coords");
    let row_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_subj-position\" \
         (notice, created_by_id, updated_by_id, dk_scene, dk_factor, dk_function, \
          ak_benefit_user, ak_permit_user, ak_access_user) \
         VALUES ($1, $2, $2, $3, $4, $5, ARRAY[$6::bigint], ARRAY[$6::bigint], ARRAY[$6::bigint]) \
         RETURNING id",
    )
    .bind("zz-own-h-position")
    .bind(u_creator)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .bind(u_listed)
    .fetch_one(&pool)
    .await
    .expect("insert lifecycle fixture row");

    let right_id: i64 = sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_access_right LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("an access right must be seeded");

    sqlx::query(
        "INSERT INTO isahl_auth.ngac_ownership_policy \
         (resource_type, owner_attr_id, benefit_attr_id, permit_attr_id, access_attr_id, \
          read_right_id, write_right_id, delete_right_id, enabled) \
         VALUES ($1, $2, $3, $4, $5, $6, $6, $6, TRUE)",
    )
    .bind(RT)
    .bind(ua_owner)
    .bind(ua_benefit)
    .bind(ua_permit)
    .bind(ua_access)
    .bind(right_id)
    .execute(&pool)
    .await
    .expect("insert ownership policy fixture");

    // 四路各自命中
    let owner = common::ngac_org::ownership_derived_user_holders(&pool, RT, row_id, ua_owner)
        .await
        .expect("owner path");
    assert_eq!(
        owner.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![u_creator]
    );

    for (label, ua) in [
        ("benefit", ua_benefit),
        ("permit", ua_permit),
        ("access", ua_access),
    ] {
        let holders = common::ngac_org::ownership_derived_user_holders(&pool, RT, row_id, ua)
            .await
            .unwrap_or_else(|e| panic!("{label} path: {e}"));
        assert_eq!(
            holders.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![u_listed],
            "{label} 路应解析出被列入 ak_* 的用户"
        );
    }

    // 无关 UA ⇒ 空
    let other = make_ua(&pool, "ua-unrelated").await;
    let none = common::ngac_org::ownership_derived_user_holders(&pool, RT, row_id, other)
        .await
        .expect("unrelated path");
    assert!(none.is_empty(), "无关 UA 不应解析出持有者");

    // enabled = false ⇒ 不命中（配置面判据）
    sqlx::query(
        "UPDATE isahl_auth.ngac_ownership_policy SET enabled = FALSE WHERE resource_type = $1",
    )
    .bind(RT)
    .execute(&pool)
    .await
    .expect("disable policy");
    let disabled = common::ngac_org::ownership_derived_user_holders(&pool, RT, row_id, ua_owner)
        .await
        .expect("disabled path");
    assert!(disabled.is_empty(), "停用的属权策略不应派生持有者");

    cleanup(&pool).await;
}
