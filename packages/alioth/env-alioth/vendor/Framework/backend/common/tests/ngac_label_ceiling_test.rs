//! 分级（密级）上限谓词集成测试（NGAC_SPEC §2.7，D-1b）。
//!
//! 唯一实现 = `common::ngac_org::{label_clearance_denies, label_clearance_denies_rule}`
//! （SQL 版供单资源路径 decide/gate；纯 Rust 版供批量路径 `pdp/list` 逐行复用，规则一致）。
//!
//! 覆盖矩阵（三要素 × 方向 × 缺标签极性）：
//!
//! | 用例 | 白名单 | 资源标签 | 主体 clearance | 期望 |
//! |---|---|---|---|---|
//! | a 低 clearance 读高标签 | ✓ | rank 5 | rank 1 | **拒绝**（含 admin/association 存在） |
//! | b 高 clearance 读同标签 | ✓ | rank 5 | rank 5 | 放行（association 命中） |
//! | c 非白名单资源类型 | ✗ | rank 5 | rank 1 | 放行（谓词不介入） |
//! | d 白名单但资源无标签 | ✓ | 无 | rank 1 | 放行（谓词不介入） |
//! | e 无 clearance（fail-closed） | ✓ | rank 5 | 无 | **拒绝** |
//!
//! 运行（连共享测试库，`common::testing::connect_test_db` 强制 `*_test` 库）：
//!   bash scripts/cargo-test.sh -p common --test ngac_label_ceiling_test
//!
//! 数据卫生：自建行一律以 `zz-lc-itest` 前缀硬删（OA 的 `(resource_type, fk_resource)`
//! 唯一索引不随软删释放 ⇒ 必须硬删，同 `permissions_runtime_test` 先例）。

use common::permissions::require_resource_access;
use common::testing::connect_test_db;
use ngac_host::{label_clearance_denies, label_clearance_denies_rule};
use sqlx::PgPool;

const MARKER: &str = "zz-lc-itest";
const RT_WHITELISTED: &str = "zz_lc_res";
const RT_FREE: &str = "zz_lc_res_free";
const FK_SECRET: i64 = 990001;
const FK_FREE: i64 = 990002;
const FK_NOLABEL: i64 = 990003;

struct Fixture {
    low_user: i64,
    high_user: i64,
}

/// 清理（幂等；建前与测后各一次）。顺序遵守外键：association → rr → UA/OA → 用户 → 标签/类。
async fn cleanup(pool: &PgPool) {
    let p = format!("{MARKER}%");
    for sql in [
        "DELETE FROM isahl_auth.ngac_association WHERE o_name LIKE $1",
        "DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE o_name LIKE $1",
        "DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE $1",
        "DELETE FROM isahl_auth.ngac_object_attribute WHERE o_name LIKE $1",
        "DELETE FROM isahl_auth.auth_users WHERE username LIKE $1",
        "DELETE FROM isahl_auth.org_policy_class WHERE code LIKE $1",
        "DELETE FROM isahl_auth.org_policy_label WHERE code LIKE $1",
    ] {
        sqlx::query(sql)
            .bind(&p)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("cleanup {sql}: {e}"));
    }
}

async fn default_policy_class(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("default policy class must be seeded")
}

/// 建 UA（带 property）+ 用户 + 绑定，返回 (user_id, ua_id)。
async fn mk_subject(pool: &PgPool, slug: &str, clearance: Option<&str>) -> (i64, i64) {
    let class_id = default_policy_class(pool).await;
    let ua: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, property) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("{MARKER}_ua_{slug}"))
    .bind(class_id)
    .bind(clearance.map(|c| serde_json::json!({ "clearance": c })))
    .fetch_one(pool)
    .await
    .expect("insert fixture UA");
    let user: i64 =
        sqlx::query_scalar("INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id")
            .bind(format!("{MARKER}_user_{slug}"))
            .fetch_one(pool)
            .await
            .expect("insert fixture user");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (o_name, fk_user, fk_user_attribute) \
         VALUES ($1, $2, $3)",
    )
    .bind(format!("{MARKER}_bind_{slug}"))
    .bind(user)
    .bind(ua)
    .execute(pool)
    .await
    .expect("bind fixture UA");
    (user, ua)
}

/// 建 OA（resource_type/fk_resource 固定，property 可带 label）+ read 关联（`$2` = UA）。
async fn mk_resource(
    pool: &PgPool,
    slug: &str,
    rt: &str,
    fk_resource: i64,
    label: Option<&str>,
) -> i64 {
    let class_id = default_policy_class(pool).await;
    let oa: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_object_attribute \
            (o_name, fk_policy_class, resource_type, fk_resource, property) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(format!("{MARKER}_oa_{slug}"))
    .bind(class_id)
    .bind(rt)
    .bind(fk_resource)
    .bind(label.map(|l| serde_json::json!({ "label": l })))
    .fetch_one(pool)
    .await
    .expect("insert fixture OA");
    oa
}

async fn grant_read(pool: &PgPool, slug: &str, ua: i64, oa: i64) {
    let class_id = default_policy_class(pool).await;
    // `ngac_access_right` 无 `fk_policy_class`/`deleted_at`（列：id, o_name, created_at, updated_at），
    // 且 seed 已含 `read`（实测 17 项）。缺失时才补建（前缀行随 cleanup 硬删）。
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'read' LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .expect("read access right lookup");
    let ar: i64 = match existing {
        Some(id) => id,
        None => sqlx::query_scalar(
            "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ($1) RETURNING id",
        )
        .bind(format!("{MARKER}_ar_read_{slug}"))
        .fetch_one(pool)
        .await
        .expect("insert read access right"),
    };
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_association \
            (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class) \
         VALUES ($1, $2, $3, ARRAY[$4], $5)",
    )
    .bind(format!("{MARKER}_assoc_{slug}"))
    .bind(ua)
    .bind(oa)
    .bind(ar)
    .bind(class_id)
    .execute(pool)
    .await
    .expect("insert association");
}

/// 全夹具：白名单类（仅 `RT_WHITELISTED`）+ 两档标签 + 低/高 clearance 主体 + 三资源。
async fn setup(pool: &PgPool) -> Fixture {
    cleanup(pool).await;
    sqlx::query(
        "INSERT INTO isahl_auth.org_policy_class \
            (code, notice, state, version, scope, ua_template, audit_required) \
         VALUES ($1, $2, 'active', 1, $3, '{}'::jsonb, false)",
    )
    .bind(format!("{MARKER}_wl"))
    .bind("label ceiling itest whitelist")
    .bind(serde_json::json!({ "resource_types": [RT_WHITELISTED] }))
    .execute(pool)
    .await
    .expect("insert whitelist class");
    for (code, rank) in [("zz-lc-itest_low", 1), ("zz-lc-itest_high", 5)] {
        sqlx::query(
            "INSERT INTO isahl_auth.org_policy_label \
                (code, rank, domain, notice, is_active) \
             VALUES ($1, $2, 'data', 'itest', true)",
        )
        .bind(code)
        .bind(rank)
        .execute(pool)
        .await
        .expect("insert label");
    }

    let (low_user, ua_low) = mk_subject(pool, "low", Some("zz-lc-itest_low")).await;
    let (high_user, ua_high) = mk_subject(pool, "high", Some("zz-lc-itest_high")).await;

    let oa_secret = mk_resource(
        pool,
        "secret",
        RT_WHITELISTED,
        FK_SECRET,
        Some("zz-lc-itest_high"),
    )
    .await;
    let oa_free = mk_resource(pool, "free", RT_FREE, FK_FREE, Some("zz-lc-itest_high")).await;
    let oa_nolabel = mk_resource(pool, "nolabel", RT_WHITELISTED, FK_NOLABEL, None).await;

    grant_read(pool, "low_secret", ua_low, oa_secret).await;
    grant_read(pool, "high_secret", ua_high, oa_secret).await;
    grant_read(pool, "low_free", ua_low, oa_free).await;
    grant_read(pool, "low_nolabel", ua_low, oa_nolabel).await;

    Fixture {
        low_user,
        high_user,
    }
}

#[tokio::test]
async fn label_ceiling_denies_low_clearance_and_is_inert_without_data() {
    let pool: PgPool = connect_test_db().await;
    let fx = setup(&pool).await;

    // a) 白名单 + 资源 rank 5 + 主体 rank 1 ⇒ 拒绝（即使 read 关联存在）
    assert!(
        label_clearance_denies(&pool, RT_WHITELISTED, FK_SECRET, fx.low_user)
            .await
            .expect("predicate"),
        "low clearance must be denied on high-label resource"
    );
    let blocked =
        require_resource_access(&pool, fx.low_user, RT_WHITELISTED, FK_SECRET, "read").await;
    assert!(
        blocked.is_err(),
        "gate must deny despite association: {blocked:?}"
    );

    // b) 同标签等级（5 ≥ 5）⇒ 放行（谓词不拒 + 关联命中）
    assert!(
        !label_clearance_denies(&pool, RT_WHITELISTED, FK_SECRET, fx.high_user)
            .await
            .expect("predicate"),
        "clearance == label rank must pass"
    );
    let allowed =
        require_resource_access(&pool, fx.high_user, RT_WHITELISTED, FK_SECRET, "read").await;
    assert!(allowed.is_ok(), "gate must allow: {allowed:?}");

    // c) 非白名单资源类型 ⇒ 谓词不介入（带标签也没用）
    assert!(
        !label_clearance_denies(&pool, RT_FREE, FK_FREE, fx.low_user)
            .await
            .expect("predicate"),
        "non-whitelisted resource type must be unaffected"
    );
    let free = require_resource_access(&pool, fx.low_user, RT_FREE, FK_FREE, "read").await;
    assert!(free.is_ok(), "non-whitelisted gate must allow: {free:?}");

    // d) 白名单但资源无标签 ⇒ 谓词不介入
    assert!(
        !label_clearance_denies(&pool, RT_WHITELISTED, FK_NOLABEL, fx.low_user)
            .await
            .expect("predicate"),
        "resource without label must be unaffected"
    );

    // e) 资源有标签、主体无 clearance ⇒ 拒绝（fail-closed 极性）
    let (no_clearance_user, _) = mk_subject(&pool, "noclearance", None).await;
    assert!(
        label_clearance_denies(&pool, RT_WHITELISTED, FK_SECRET, no_clearance_user)
            .await
            .expect("predicate"),
        "missing clearance must fail closed"
    );

    cleanup(&pool).await;
}

/// 批量路径复用的纯规则：与 SQL 版同规则（缺 clearance → rank 0 → 拒绝；无标签/非白名单 → 不介入）。
#[test]
fn label_ceiling_rule_matches_sql_semantics() {
    assert!(label_clearance_denies_rule(true, Some(5), Some(1)));
    assert!(!label_clearance_denies_rule(true, Some(5), Some(5)));
    assert!(!label_clearance_denies_rule(true, Some(5), Some(9)));
    assert!(
        label_clearance_denies_rule(true, Some(5), None),
        "fail-closed"
    );
    assert!(
        !label_clearance_denies_rule(true, None, Some(1)),
        "no label ⇒ inert"
    );
    assert!(
        !label_clearance_denies_rule(false, Some(5), Some(1)),
        "not whitelisted ⇒ inert"
    );
}
