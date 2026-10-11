//! 创建者授权派生——PDP 侧接缝测试（derive-creator-grants-from-lifecycle）。
//!
//! 覆盖 `pip::get_accessible_resource_ids`（= `/api/ngac/pdp/list` 的行级可见性引擎）：
//! 锚点单行 + **零 association** 前置下，创建者共享 UA 的用户经派生可见；
//! 无共享 UA 不可见；创建者 rr 过期不可见；共享谓词（`ngac_host::creator_derived_permits`）
//! 与 decide/explain 的 OR 接线同源（decide_access 为 pub(crate)，按 `ngac_ownership_visibility_test`
//! 先例测接缝；OR 接线 6 行经编译与代码复核）。

use common::testing::connect_test_db;
use gateway_sso::ngac::pip::PostgresPip;
use gateway_sso::ngac::Pip;
use sqlx::PgPool;

const RT: &str = "zz-cg-pdp";

async fn open_test_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    if url.is_none() {
        eprintln!("skipped: DATABASE_URL 未设置");
        return None;
    }
    Some(connect_test_db().await)
}

async fn cleanup(pool: &PgPool) {
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user_attribute IN \
         (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'zz-cg-pdp%')",
    )
    .execute(pool)
    .await;
    let _ =
        sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE o_name LIKE 'zz-cg-pdp%'")
            .execute(pool)
            .await;
    let _ =
        sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'zz-cg-pdp%'")
            .execute(pool)
            .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username LIKE 'zz-cg-pdp%'")
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_subj-position\" WHERE notice LIKE 'zz-cg-pdp%'")
        .execute(pool)
        .await;
}

async fn make_user(pool: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id")
        .bind(format!("zz-cg-pdp-{name}"))
        .fetch_one(pool)
        .await
        .expect("insert user")
}

/// 夹具：UA + 三用户（creator/judge 绑 UA，other 不绑）+ 生命周期行（创建者=creator）+
/// **仅锚点单行**（ensure_resource_oa；零 association——派生是唯一通路）。
async fn setup(pool: &PgPool) -> (i64, i64, i64, i64, i64) {
    let creator = make_user(pool, "creator").await;
    let judge = make_user(pool, "judge").await;
    let other = make_user(pool, "other").await;

    let pc: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default'")
            .fetch_one(pool)
            .await
            .expect("default policy class");
    let ua: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class) \
         VALUES ('zz-cg-pdp-ua', $1) RETURNING id",
    )
    .bind(pc)
    .fetch_one(pool)
    .await
    .expect("insert UA");
    for u in [creator, judge] {
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute) \
             VALUES ($1, $2)",
        )
        .bind(u)
        .bind(ua)
        .execute(pool)
        .await
        .expect("insert rr");
    }

    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve coords");
    let row_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_subj-position\" \
         (notice, created_by_id, updated_by_id, dk_scene, dk_factor, dk_function) \
         VALUES ('zz-cg-pdp-row', $1, $1, $2, $3, $4) RETURNING id",
    )
    .bind(creator)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("insert lifecycle row");

    common::ngac_grants::ensure_resource_oa(
        pool,
        &common::ngac_grants::ResourceOaSpec {
            o_name: format!("zz-cg-pdp-{row_id}"),
            resource_type: RT,
            resource_id: row_id,
            resource_identifier: Some("ZZ-CG-PDP"),
            created_by: Some(creator),
            policy_class_name: Some("default"),
        },
    )
    .await
    .expect("ensure anchor OA");

    // 前置：零 association（锚点单行是唯一物化）
    let assoc: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.ngac_association a \
         JOIN isahl_auth.ngac_object_attribute oa ON oa.id = a.fk_object_attribute \
         WHERE oa.resource_type = $1 AND oa.fk_resource = $2 AND a.deleted_at IS NULL",
    )
    .bind(RT)
    .bind(row_id)
    .fetch_one(pool)
    .await
    .expect("count assoc");
    assert_eq!(assoc, 0, "前置：零 association（派生是唯一通路）");

    (creator, judge, other, ua, row_id)
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn creator_derived_grant_visible_in_pdp_list_paths() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;
    let (creator, judge, other, ua, row_id) = setup(&pool).await;
    let pip = PostgresPip::new(pool.clone());

    // ① 创建者本人 + 共享 UA 判定者：派生可见（list 引擎 = pip）
    for (u, label) in [(creator, "创建者本人"), (judge, "共享 UA 判定者")] {
        let visible = pip
            .get_accessible_resource_ids(u, RT, "read")
            .await
            .expect("visible ids");
        assert!(
            visible.contains(&row_id),
            "{label} MUST 经派生看见该行（visible={visible:?}）"
        );
    }

    // ② 无共享 UA：不可见
    let vis_other = pip
        .get_accessible_resource_ids(other, RT, "read")
        .await
        .expect("other visible");
    assert!(
        !vis_other.contains(&row_id),
        "无共享 UA MUST NOT 看见该行（visible={vis_other:?}）"
    );

    // ③ 创建者 rr 过期 ⇒ 派生失效（judge 与 creator 均不可见）
    sqlx::query(
        "UPDATE isahl_auth.ngac_user_rr_attribute SET expires_at = NOW() - INTERVAL '1 hour' \
         WHERE fk_user = $1 AND fk_user_attribute = $2",
    )
    .bind(creator)
    .bind(ua)
    .execute(&pool)
    .await
    .expect("expire creator rr");
    for u in [creator, judge] {
        let visible = pip
            .get_accessible_resource_ids(u, RT, "read")
            .await
            .expect("visible after expire");
        assert!(
            !visible.contains(&row_id),
            "创建者 rr 过期后 MUST 不可见（user={u}, visible={visible:?}）"
        );
    }

    // ④ 共享谓词（decide/explain OR 接线同源）：复活后命中 read、不命中 approve
    sqlx::query(
        "UPDATE isahl_auth.ngac_user_rr_attribute SET expires_at = NULL \
         WHERE fk_user = $1 AND fk_user_attribute = $2",
    )
    .bind(creator)
    .bind(ua)
    .execute(&pool)
    .await
    .expect("revive creator rr");
    assert!(
        ngac_host::creator_derived_permits(&pool, judge, row_id, "read")
            .await
            .expect("scalar read"),
        "共享谓词 read 命中"
    );
    assert!(
        !ngac_host::creator_derived_permits(&pool, judge, row_id, "approve")
            .await
            .expect("scalar approve"),
        "approve 不在创建者集（审核权与创建权分离）"
    );
    assert!(
        !ngac_host::creator_derived_permits(&pool, other, row_id, "read")
            .await
            .expect("scalar other"),
        "无共享 UA ⇒ 谓词不命中"
    );

    cleanup(&pool).await;
}
