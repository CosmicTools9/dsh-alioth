//! Framework 内层判定（defense-in-depth）的**属权派生**路径测试 —— NGAC_SPEC §2.2.6 + §2.5。
//!
//! 验证 A1 的 Framework 半边：`require_resource_access`（PEP 之后的内层门）MUST 通过
//! `ngac_ownership_policy` 的资源受限派生把属主接到目标 UA 上——**即使属主对该行 OA 没有任何
//! 直派 association**，只要该 UA 有 association 即放行；停用属权策略（`enabled = false`）后
//! MUST 拒绝（证明判定确实消费了该策略而非旁路放行）。
//!
//! 口径：`TEST_DATABASE_URL="postgres://<user>@localhost:5432/aliothstudio_test"` + `--ignored`。
//! 夹具自建自清（前缀 `zz-own-fw`，OA/association 硬删）。

use common::ngac_grants::{ensure_resource_oa, grant_rights, ResourceOaSpec};
use common::permissions::require_resource_access;
use sqlx::PgPool;

const RT: &str = "zz-own-fw";
const UA_NAME: &str = "zz-own-fw-ua";

async fn open_test_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    if url.is_none() {
        eprintln!("skipped: DATABASE_URL 未设置");
        return None;
    }
    Some(common::testing::connect_test_db().await)
}

async fn cleanup(pool: &PgPool) {
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.ngac_association a USING isahl_auth.ngac_object_attribute oa \
         WHERE a.fk_object_attribute = oa.id AND oa.resource_type = $1",
    )
    .bind(RT)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type = $1")
        .bind(RT)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.ngac_ownership_policy WHERE resource_type = $1")
        .bind(RT)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_subj-position\" WHERE notice LIKE 'zz-own-fw%'")
        .execute(pool)
        .await;
    let _ =
        sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'zz-own-fw%'")
            .execute(pool)
            .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username LIKE 'zz-own-fw%'")
        .execute(pool)
        .await;
}

/// 夹具：属主/陌生人 + 属权策略（owner_attr_id = 专属 UA）+ 生命周期行（创建者=属主）
/// + 行 OA + **仅 UA 关联**（属主本人零 association）。返回 (owner, stranger, row_id)。
async fn setup_fixture(pool: &PgPool) -> (i64, i64, i64) {
    let owner: i64 =
        sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id")
            .bind("zz-own-fw-owner")
            .fetch_one(pool)
            .await
            .expect("insert owner");
    let stranger: i64 =
        sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id")
            .bind("zz-own-fw-stranger")
            .fetch_one(pool)
            .await
            .expect("insert stranger");

    let pc: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default'")
            .fetch_one(pool)
            .await
            .expect("default policy class");
    let ua_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class) \
         VALUES ($1, $2) RETURNING id",
    )
    .bind(UA_NAME)
    .bind(pc)
    .fetch_one(pool)
    .await
    .expect("insert UA");

    let right_id: i64 = sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_access_right LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("an access right must be seeded");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_ownership_policy \
         (resource_type, owner_attr_id, read_right_id, write_right_id, delete_right_id, enabled) \
         VALUES ($1, $2, $3, $3, $3, TRUE)",
    )
    .bind(RT)
    .bind(ua_id)
    .bind(right_id)
    .execute(pool)
    .await
    .expect("insert ownership policy");

    // 坐标三元组经 ontology_binding 解析（§6.12 + §4.3.3；禁硬编码 ZUID）
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve coords");
    let row_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_subj-position\" \
         (notice, created_by_id, updated_by_id, dk_scene, dk_factor, dk_function) \
         VALUES ($1, $2, $2, $3, $4, $5) RETURNING id",
    )
    .bind("zz-own-fw-row")
    .bind(owner)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("insert lifecycle row");

    ensure_resource_oa(
        pool,
        &ResourceOaSpec {
            o_name: format!("{RT}-{row_id}"),
            resource_type: RT,
            resource_id: row_id,
            resource_identifier: Some("ZZ-OWN-FW"),
            created_by: Some(owner),
            policy_class_name: Some("default"),
        },
    )
    .await
    .expect("ensure row OA");
    grant_rights(pool, RT, row_id, UA_NAME, &["read"])
        .await
        .expect("grant read to ownership UA");

    (owner, stranger, row_id)
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn framework_inner_gate_honours_ownership_derivation() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;
    let (owner, stranger, row_id) = setup_fixture(&pool).await;

    // ① 属主：零 association，仅凭属权派生 UA 命中 → 放行
    let owner_res = require_resource_access(&pool, owner, RT, row_id, "read").await;
    assert!(
        owner_res.is_ok(),
        "属主 MUST 经属权派生 UA 在内层门放行（got={owner_res:?}）"
    );

    // ② 陌生人：无属权派生、无 association → 拒绝（fail-closed）
    let stranger_res = require_resource_access(&pool, stranger, RT, row_id, "read").await;
    assert!(
        stranger_res.is_err(),
        "非属主 MUST 被拒（got={stranger_res:?}）"
    );

    // ③ 停用属权策略 ⇒ 属主亦被拒（证明判定确实消费该策略，而非旁路放行）
    sqlx::query(
        "UPDATE isahl_auth.ngac_ownership_policy SET enabled = FALSE WHERE resource_type = $1",
    )
    .bind(RT)
    .execute(&pool)
    .await
    .expect("disable policy");
    let disabled_res = require_resource_access(&pool, owner, RT, row_id, "read").await;
    assert!(
        disabled_res.is_err(),
        "属权策略停用后 MUST 拒绝（got={disabled_res:?}）"
    );

    cleanup(&pool).await;
}
