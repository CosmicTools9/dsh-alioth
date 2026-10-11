//! 属权派生 → RLS 可见性（`pdp/list` 候选超集）集成测试 —— NGAC_SPEC §2.2.6 + §6.1b。
//!
//! 验证 A1 修复的 **RLS 半边**：某资源行的创建者（`created_by_id`）经
//! `ngac_ownership_policy.owner_attr_id` 派生 UA 后，**即使该用户对该行 OA 没有直派 association**，
//! `get_accessible_resource_ids`（PEP `pdp/list` 的可见集来源）MUST 仍能看见该行；
//! 非属主且无 association 的用户 MUST NOT 看见。
//!
//! 口径：`TEST_DATABASE_URL="postgres://<user>@localhost:5432/aliothstudio_test"` + `--ignored`。
//! 夹具自建自清（前缀 `zz-own-vis`；OA/association 硬删——`uq_ngac_oa_resource` 不随软删释放）。

use common::ngac_grants::{ensure_resource_oa, grant_rights, ResourceOaSpec};
use gateway_sso::ngac::pip::PostgresPip;
use gateway_sso::ngac::Pip;
use sqlx::PgPool;
use std::sync::LazyLock;
use tokio::sync::Mutex;

/// 文件级串行化：两用例共享 fixture 族（RT/policy 单例键 + 同名清理），并行必撞唯一键
/// （2026-09-30 实测三处：用户名/UA 名/policy resource_type）。共享是设计选择 ⇒ 串行
/// （同 ngac_grants_test 先例）。
static SERIAL_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

const RT: &str = "zz-own-vis";
/// 按 tag 唯一化（两用例并行跑时 UA 名/用户名均不可共享——唯一键竞态，2026-09-30 修复）
fn ua_name(tag: &str) -> String {
    format!("zz-own-vis-ua-{tag}")
}

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
    let _ =
        sqlx::query("DELETE FROM isahl.\"zc_id_subj-position\" WHERE notice LIKE 'zz-own-vis%'")
            .execute(pool)
            .await;
    let _ =
        sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'zz-own-vis%'")
            .execute(pool)
            .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username LIKE 'zz-own-vis%'")
        .execute(pool)
        .await;
}

async fn make_user(pool: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id")
        .bind(format!("zz-own-vis-{name}"))
        .fetch_one(pool)
        .await
        .expect("insert user")
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn owner_sees_owned_row_without_direct_association() {
    let _guard = SERIAL_LOCK.lock().await;
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;
    let (owner, stranger, row_id) = setup_fixture(&pool, "a").await;
    let pip = PostgresPip::new(pool.clone());

    // ① 属主：经属权派生 UA 命中 → 可见
    let visible_owner = pip
        .get_accessible_resource_ids(owner, RT, "read")
        .await
        .expect("owner visible ids");
    assert!(
        visible_owner.contains(&row_id),
        "属主 MUST 经属权派生 UA 看见自己创建的行（visible={visible_owner:?}, row={row_id}）"
    );

    // ② 非属主且无该 UA：不可见
    let visible_stranger = pip
        .get_accessible_resource_ids(stranger, RT, "read")
        .await
        .expect("stranger visible ids");
    assert!(
        !visible_stranger.contains(&row_id),
        "非属主且无 association MUST NOT 看见该行（visible={visible_stranger:?}）"
    );

    cleanup(&pool).await;
}

/// PEP 单条判定（`decide_access`）所消费的**资源受限 UA 集**接缝（`pub` 面）：
/// 属主对目标资源 MUST 派生到属权 UA；非属主 MUST NOT。
///
/// 为什么测接缝而非 `decide_access`：后者为 `pub(crate)`（集成测试 crate 不可达）；
/// 而它是 decide/explain 的唯一 UA 来源（单点调用），接缝通过即判定链输入正确。
#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn decide_seam_derives_ownership_ua_for_owner_only() {
    let _guard = SERIAL_LOCK.lock().await;
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool).await;
    let (owner, stranger, row_id) = setup_fixture(&pool, "b").await;
    let pip = PostgresPip::new(pool.clone());

    let owner_attrs = pip
        .get_resource_scoped_user_attributes(owner, RT, row_id)
        .await
        .expect("owner scoped attrs");
    assert!(
        owner_attrs.iter().any(|a| a.o_name == ua_name("b")),
        "属主的资源受限 UA 集 MUST 含属权派生 UA（got={:?}）",
        owner_attrs
            .iter()
            .map(|a| a.o_name.clone())
            .collect::<Vec<_>>()
    );

    let stranger_attrs = pip
        .get_resource_scoped_user_attributes(stranger, RT, row_id)
        .await
        .expect("stranger scoped attrs");
    assert!(
        !stranger_attrs.iter().any(|a| a.o_name == ua_name("b")),
        "非属主 MUST NOT 派生到该 UA"
    );

    cleanup(&pool).await;
}

/// 夹具：属主/陌生人 + 属权 UA + 属权策略 + 生命周期行（创建者=属主，坐标经解析）+
/// 行 OA + **仅 UA 关联**（属主本人无任何 association）。返回 (owner, stranger, row_id)。
async fn setup_fixture(pool: &PgPool, tag: &str) -> (i64, i64, i64) {
    let owner = make_user(pool, &format!("owner-{tag}")).await;
    let stranger = make_user(pool, &format!("stranger-{tag}")).await;

    // 属权派生目标 UA（唯一实现 `ngac_org::OWNERSHIP_CTE` 以 `owner_attr_id` 指向它）
    let pc: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default'")
            .fetch_one(pool)
            .await
            .expect("default policy class");
    let ua = ua_name(tag);
    let ua_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class) \
         VALUES ($1, $2) RETURNING id",
    )
    .bind(&ua)
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

    // 生命周期行（属权派生 join 载体）：创建者 = owner；坐标经 ontology_binding 解析（§6.12/§4.3.3）
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve coords");
    let row_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_subj-position\" \
         (notice, created_by_id, updated_by_id, dk_scene, dk_factor, dk_function) \
         VALUES ($1, $2, $2, $3, $4, $5) RETURNING id",
    )
    .bind("zz-own-vis-row")
    .bind(owner)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("insert lifecycle row");

    // 行 OA 注册 + **只给 UA 关联**（不给 owner 本人任何 association —— 属权派生是唯一通路）
    ensure_resource_oa(
        pool,
        &ResourceOaSpec {
            o_name: format!("{RT}-{row_id}"),
            resource_type: RT,
            resource_id: row_id,
            resource_identifier: Some("ZZ-OWN-VIS"),
            created_by: Some(owner),
            policy_class_name: Some("default"),
        },
    )
    .await
    .expect("ensure row OA");
    grant_rights(pool, RT, row_id, &ua, &["read"])
        .await
        .expect("grant read to ownership UA");

    (owner, stranger, row_id)
}
