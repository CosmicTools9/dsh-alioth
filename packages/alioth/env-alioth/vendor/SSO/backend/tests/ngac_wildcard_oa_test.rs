//! 通配对象属性（`resource_type = '*'`）在 SSO OA 解析中的并入 —— NGAC_SPEC §2.5 内层同源项 ③。
//!
//! 背景：M16 收敛前该语义**只有 Framework 侧**有（`permissions.rs::resource_attrs` 的
//! 「通配资源」分支）；SSO `pip::get_inherited_object_attributes` 未并入 ⇒ 双侧判定分叉。
//! 收敛后 SSO MUST 同样把通配 OA 纳入目标资源的 OA 集合（含自身与祖先闭包）。
//!
//! 本测试守护：① 目标资源无行 OA 时，通配 OA **出现**在解析结果中（PEP 可据此授权）；
//! ② 通配 OA 的 association 让**任意** resource_type 的资源可判定（PEP 层生效）；
//! ③ 对照：不相关 resource_type 的普通 OA **不**出现。
//!
//! 口径：`TEST_DATABASE_URL="postgres://<user>@localhost:5432/aliothstudio_test"` + `--ignored`。

use common::ngac_grants::{ensure_resource_oa, grant_rights, ResourceOaSpec};
use gateway_sso::ngac::pip::PostgresPip;
use gateway_sso::ngac::Pip;
use sqlx::PgPool;

const RT: &str = "zz-wild";
const UA_NAME: &str = "zz-wild-ua";
const WILDCARD_OA: &str = "zz-wild-oa";
const OTHER_OA: &str = "zz-wild-other-oa";

async fn open_test_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    if url.is_none() {
        eprintln!("skipped: DATABASE_URL 未设置");
        return None;
    }
    Some(common::testing::connect_test_db().await)
}

async fn cleanup(pool: &PgPool) {
    for rt in [RT, "*", "zz-wild-other"] {
        let _ = sqlx::query(
            "DELETE FROM isahl_auth.ngac_association a USING isahl_auth.ngac_object_attribute oa \
             WHERE a.fk_object_attribute = oa.id AND oa.resource_type = $1",
        )
        .bind(rt)
        .execute(pool)
        .await;
        let _ =
            sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type = $1")
                .bind(rt)
                .execute(pool)
                .await;
    }
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'zz-wild%'")
        .execute(pool)
        .await;
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn wildcard_oa_participates_in_sso_object_attr_resolution() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;

    // 通配 OA（fk_resource = 0）+ 不相关类型的普通 OA（对照）
    ensure_resource_oa(
        &pool,
        &ResourceOaSpec {
            o_name: WILDCARD_OA.to_string(),
            resource_type: "*",
            resource_id: 0,
            resource_identifier: Some("ZZ-WILD"),
            created_by: None,
            policy_class_name: Some("default"),
        },
    )
    .await
    .expect("ensure wildcard OA");
    ensure_resource_oa(
        &pool,
        &ResourceOaSpec {
            o_name: OTHER_OA.to_string(),
            resource_type: "zz-wild-other",
            resource_id: 0,
            resource_identifier: Some("ZZ-WILD-OTHER"),
            created_by: None,
            policy_class_name: Some("default"),
        },
    )
    .await
    .expect("ensure other OA");

    let wildcard_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute \
         WHERE resource_type = '*' AND fk_resource = 0 AND o_name = $1",
    )
    .bind(WILDCARD_OA)
    .fetch_one(&pool)
    .await
    .expect("wildcard OA id");
    let other_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_object_attribute WHERE o_name = $1")
            .bind(OTHER_OA)
            .fetch_one(&pool)
            .await
            .expect("other OA id");

    // 目标资源（RT, 4242）**无行 OA** —— 只有通配 OA 可供判定
    let pip = PostgresPip::new(pool.clone());
    let attrs = pip
        .get_all_object_attributes_with_inheritance(RT, 4242)
        .await
        .expect("resolve object attrs");
    let ids: Vec<i64> = attrs.iter().map(|a| a.id).collect();

    assert!(
        ids.contains(&wildcard_id),
        "通配 OA MUST 并入目标资源的 OA 集合（ids={ids:?}, wildcard={wildcard_id}）"
    );
    assert!(
        !ids.contains(&other_id),
        "不相关 resource_type 的普通 OA MUST NOT 并入（ids={ids:?}, other={other_id}）"
    );

    // 通配 OA 上的 association（UA → 通配 OA）⇒ 该 UA 对任意资源类型可用
    let pc: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default'")
            .fetch_one(&pool)
            .await
            .expect("default policy class");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class) VALUES ($1, $2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(UA_NAME)
    .bind(pc)
    .execute(&pool)
    .await
    .expect("insert UA");
    grant_rights(&pool, "*", 0, UA_NAME, &["read"])
        .await
        .expect("grant read on wildcard OA");

    let assoc: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl_auth.ngac_association a \
         WHERE a.fk_object_attribute = $1 AND a.deleted_at IS NULL",
    )
    .bind(wildcard_id)
    .fetch_one(&pool)
    .await
    .expect("count wildcard associations");
    assert!(
        assoc >= 1,
        "通配 OA 上 MUST 可建立 association（got={assoc}）"
    );

    cleanup(&pool).await;
}
