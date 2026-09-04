//! D-2b 派生/迁移幂等集成测试（?11，D-2 在生产跑迁移前 MUST）。
//!
//! 覆盖（common::ngac_policy / common::ngac_org 唯一实现，SSO/Gateway 消费同源）：
//! - [`common::ngac_policy::derive_from_class`] 幂等：active class（
//!   `ua_template.name_rule = 'position:{code}'`）+ active rule 预置后
//!   连跑两次 → 首次 UA/OA/association 各 ≥1 新增，第二次全 0；
//! - [`common::ngac_org::migrate_legacy_position_associations`] 幂等：存量实例码
//!   UA（`position:{实例code}`，cognition 物化标记）+ 存活 association + 真实岗位行
//!   （`_f_ IS NULL`，`ck_category` → 基表类别行）→ 连跑两次 → 首次复制 ≥1、
//!   第二次 0，legacy association 原样保留。
//!
//! 运行（连共享测试库，须 `*_test` 库；common::testing 强制校验）：
//!   CARGO_TARGET_DIR=/tmp/alioth-check DATABASE_URL=postgres://localhost/aliothstudio_test \
//!     cargo test -p common --test ngac_derive_test -- --nocapture
//!
//! 数据卫生：所有自建行（class/rule/UA/OA/association/岗位/类别）测试结束软删
//! （`deleted_at`），不污染共享测试库；测试自身行以 `source_kind='ngac-derive-itest'`
//! 或 notice 标记 + 时间戳后缀标识，重跑先软删旧残留（同 SSO ngac 测试前置清理先例）。

use common::ngac_org::migrate_legacy_position_associations;
use common::ngac_policy::derive_from_class;
use common::testing::connect_test_db;
use sqlx::PgPool;
use std::time::{SystemTime, UNIX_EPOCH};

/// 自建行标记（property 或 notice），重跑前置清理只删本测试家族行。
const MARKER: &str = "ngac-derive-itest";

fn mk_suffix() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis()
        .to_string()
}

/// D-1 org_policy 资产表在共享测试库缺失（DDL 不在本仓迁移链）——测试自建最小
/// 契约面（消费 SQL 用到的列 + deleted_at 软删面）。IF NOT EXISTS 幂等，
/// 并发 worker 重入安全；生产 DDL 落地后本段自动空转。
async fn ensure_org_policy_ddl(pool: &PgPool) {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.org_policy_class (
            id bigserial PRIMARY KEY,
            code text NOT NULL,
            scope jsonb NOT NULL DEFAULT '{}',
            ua_template jsonb NOT NULL DEFAULT '{}',
            label_code text,
            state text NOT NULL DEFAULT 'draft',
            created_at timestamptz NOT NULL DEFAULT NOW(),
            updated_at timestamptz NOT NULL DEFAULT NOW(),
            deleted_at timestamptz
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("bootstrap org_policy_class DDL");
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.org_policy_rule (
            id bigserial PRIMARY KEY,
            policy_class_id bigint NOT NULL,
            resource_type text NOT NULL,
            actions jsonb NOT NULL DEFAULT '[]',
            label_code text,
            state text NOT NULL DEFAULT 'draft',
            created_at timestamptz NOT NULL DEFAULT NOW(),
            updated_at timestamptz NOT NULL DEFAULT NOW(),
            deleted_at timestamptz
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("bootstrap org_policy_rule DDL");
}

async fn default_policy_class_id(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default'")
        .fetch_one(pool)
        .await
        .expect("default policy class seeded")
}

// ---------------------------------------------------------------- derive 幂等

#[tokio::test]
async fn derive_from_class_is_idempotent() {
    let pool = connect_test_db().await;
    ensure_org_policy_ddl(&pool).await;
    let suf = mk_suffix();
    let code = format!("ngac_itest_{suf}"); // class code → UA 名 position:{code}
    let ua_name = format!("position:{code}");
    let resource_type = format!("ngac_itest_cr_{suf}");

    // 预置 active class + active rule（基表类别 UA 场景：只依赖 class/rule 行，
    // UA/OA 由 derive 物化，无需业务行——直接预置 org_policy 行即可）
    let class_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl_auth.org_policy_class
            (code, scope, ua_template, state)
        VALUES ($1, '{"actions":["read"]}', '{"name_rule":"position:{code}"}', 'active')
        RETURNING id
        "#,
    )
    .bind(&code)
    .fetch_one(&pool)
    .await
    .expect("insert active class");
    let rule_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl_auth.org_policy_rule
            (policy_class_id, resource_type, actions, state)
        VALUES ($1, $2, '["read"]', 'active')
        RETURNING id
        "#,
    )
    .bind(class_id)
    .bind(&resource_type)
    .fetch_one(&pool)
    .await
    .expect("insert active rule");

    let st1 = derive_from_class(&pool, class_id)
        .await
        .expect("first derive");
    let st2 = derive_from_class(&pool, class_id)
        .await
        .expect("second derive");

    // 捕获断言证据（先于清理）：拷贝目标上的 association 存活数
    let oa_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute \
         WHERE resource_type = $1 AND fk_resource = 0 AND deleted_at IS NULL",
    )
    .bind(&resource_type)
    .fetch_optional(&pool)
    .await
    .expect("find derived OA");
    let ua_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute \
         WHERE o_name = $1 AND deleted_at IS NULL",
    )
    .bind(&ua_name)
    .fetch_optional(&pool)
    .await
    .expect("find derived UA");

    // 清理（断言前软删，panic 也不留活行）：association → OA → UA → rule → class
    if let Some(u) = ua_id {
        sqlx::query(
            "UPDATE isahl_auth.ngac_association SET deleted_at = NOW() \
             WHERE fk_user_attribute = $1 AND deleted_at IS NULL",
        )
        .bind(u)
        .execute(&pool)
        .await
        .expect("soft-delete derived associations");
    }
    if let Some(o) = oa_id {
        sqlx::query(
            "UPDATE isahl_auth.ngac_association SET deleted_at = NOW() \
             WHERE fk_object_attribute = $1 AND deleted_at IS NULL",
        )
        .bind(o)
        .execute(&pool)
        .await
        .expect("soft-delete OA associations");
    }
    if let Some(u) = ua_id {
        sqlx::query(
            "UPDATE isahl_auth.ngac_user_attribute SET deleted_at = NOW() WHERE id = $1",
        )
        .bind(u)
        .execute(&pool)
        .await
        .expect("soft-delete derived UA");
    }
    if let Some(o) = oa_id {
        sqlx::query(
            "UPDATE isahl_auth.ngac_object_attribute SET deleted_at = NOW() WHERE id = $1",
        )
        .bind(o)
        .execute(&pool)
        .await
        .expect("soft-delete derived OA");
    }
    sqlx::query("UPDATE isahl_auth.org_policy_rule SET deleted_at = NOW() WHERE id = $1")
        .bind(rule_id)
        .execute(&pool)
        .await
        .expect("soft-delete rule");
    sqlx::query("UPDATE isahl_auth.org_policy_class SET deleted_at = NOW() WHERE id = $1")
        .bind(class_id)
        .execute(&pool)
        .await
        .expect("soft-delete class");

    // 幂等断言：首次各 ≥1，第二次全 0
    assert_eq!(st1.ua_name, ua_name, "投影 UA 名 = position:{{class code}}");
    assert!(st1.ua_created >= 1, "首次应物化类别 UA，got {st1:?}");
    assert!(st1.oa_created >= 1, "首次应建集合 OA，got {st1:?}");
    assert!(
        st1.associations_created >= 1,
        "首次应建 UA→OA association（read 存量可映射），got {st1:?}"
    );
    assert!(st1.rules_processed >= 1);
    assert_eq!(
        (st2.ua_created, st2.oa_created, st2.associations_created),
        (0, 0, 0),
        "第二次派生必须零新增（幂等），got {st2:?}"
    );
}

// ------------------------------------------------------------ legacy 迁移幂等

#[tokio::test]
async fn migrate_legacy_position_associations_is_idempotent() {
    let pool = connect_test_db().await;
    let suf = mk_suffix();
    let inst_code = format!("legacycode_{suf}"); // 存量岗位实例 code
    let legacy_ua_name = format!("position:{inst_code}");
    let cat_code = format!("ccb_{suf}"); // 基表类别行 code → 目标 UA 后缀
    let dst_ua_name = format!("position:{cat_code}");
    let pc_id = default_policy_class_id(&pool).await;

    // 重跑残留清理（仅本家族标记行；类别表基表当前 0 行，仍防御）
    let _ = sqlx::query(
        "UPDATE isahl_auth.ngac_user_attribute SET deleted_at = NOW() \
         WHERE o_name IN ($1, $2) AND deleted_at IS NULL \
           AND property ->> 'source_kind' = $3",
    )
    .bind(&legacy_ua_name)
    .bind(&dst_ua_name)
    .bind(MARKER)
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "UPDATE isahl.\"zc_id_subj-position\" SET deleted_at = NOW() \
         WHERE code = $1 AND deleted_at IS NULL AND notice = $2",
    )
    .bind(&inst_code)
    .bind(MARKER)
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "UPDATE isahl.zc_id_category SET deleted_at = NOW() \
         WHERE code = $1 AND deleted_at IS NULL AND notice = $2",
    )
    .bind(&cat_code)
    .bind(MARKER)
    .execute(&pool)
    .await;

    // 基表类别行（tableoid = isahl.zc_id_category：直接 INSERT 基表）
    let cat_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.zc_id_category (code, notice, enable, created_at, updated_at) \
         VALUES ($1, $2, true, NOW(), NOW()) RETURNING id",
    )
    .bind(&cat_code)
    .bind(MARKER)
    .fetch_one(&pool)
    .await
    .expect("insert base category row");

    // 真实岗位实例行：_f_ IS NULL、ck_category → 基表类别行
    let pos_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_subj-position\" \
            (code, ck_category, _f_, notice, created_at, updated_at) \
         VALUES ($1, $2, NULL, $3, NOW(), NOW()) RETURNING id",
    )
    .bind(&inst_code)
    .bind(cat_id)
    .bind(MARKER)
    .fetch_one(&pool)
    .await
    .expect("insert live position row");

    // legacy 实例码 UA（cognition 物化标记 → legacy_ua 命中）+ 目标类别 UA
    let legacy_ua_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, property)
        VALUES ($1, $2,
                jsonb_build_object('derived_from','cognition','source_kind',$3,'source_code',$4))
        RETURNING id
        "#,
    )
    .bind(&legacy_ua_name)
    .bind(pc_id)
    .bind(MARKER)
    .bind(&inst_code)
    .fetch_one(&pool)
    .await
    .expect("insert legacy UA");
    let dst_ua_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, property)
        VALUES ($1, $2,
                jsonb_build_object('derived_from','cognition','source_kind',$3,'source_code',$4))
        RETURNING id
        "#,
    )
    .bind(&dst_ua_name)
    .bind(pc_id)
    .bind(MARKER)
    .bind(&cat_code)
    .fetch_one(&pool)
    .await
    .expect("insert dst category UA");

    // 集合 OA（本测试独占 resource_type）+ legacy association
    let resource_type = format!("ngac_itest_mig_{suf}");
    let oa_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl_auth.ngac_object_attribute
            (o_name, fk_policy_class, resource_type, fk_resource)
        VALUES ($1, $2, $3, 0) RETURNING id
        "#,
    )
    .bind(format!("{resource_type}-collection"))
    .bind(pc_id)
    .bind(&resource_type)
    .fetch_one(&pool)
    .await
    .expect("insert OA");
    let assoc_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl_auth.ngac_association
            (fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights)
        VALUES ($1, $2, $3,
                ARRAY(SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'read'))
        RETURNING id
        "#,
    )
    .bind(legacy_ua_id)
    .bind(oa_id)
    .bind(pc_id)
    .fetch_one(&pool)
    .await
    .expect("insert legacy association");

    let n1 = migrate_legacy_position_associations(&pool)
        .await
        .expect("first migrate");
    let n2 = migrate_legacy_position_associations(&pool)
        .await
        .expect("second migrate");

    // 捕获断言证据（先于清理）：目标类别 UA 上的拷贝行 + 源 legacy association 存活
    let copied: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl_auth.ngac_association \
         WHERE fk_user_attribute = $1 AND fk_object_attribute = $2 \
           AND fk_policy_class = $3 AND deleted_at IS NULL",
    )
    .bind(dst_ua_id)
    .bind(oa_id)
    .bind(pc_id)
    .fetch_one(&pool)
    .await
    .expect("count copied association");
    let src_alive: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl_auth.ngac_association \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(assoc_id)
    .fetch_one(&pool)
    .await
    .expect("count legacy association");

    // 清理：拷贝 association + legacy association → OA/UA → 岗位/类别
    sqlx::query(
        "UPDATE isahl_auth.ngac_association SET deleted_at = NOW() \
         WHERE fk_user_attribute IN ($1, $2) AND deleted_at IS NULL",
    )
    .bind(legacy_ua_id)
    .bind(dst_ua_id)
    .execute(&pool)
    .await
    .expect("soft-delete associations");
    sqlx::query("UPDATE isahl_auth.ngac_user_attribute SET deleted_at = NOW() WHERE id IN ($1, $2)")
        .bind(legacy_ua_id)
        .bind(dst_ua_id)
        .execute(&pool)
        .await
        .expect("soft-delete UAs");
    sqlx::query("UPDATE isahl_auth.ngac_object_attribute SET deleted_at = NOW() WHERE id = $1")
        .bind(oa_id)
        .execute(&pool)
        .await
        .expect("soft-delete OA");
    sqlx::query("UPDATE isahl.\"zc_id_subj-position\" SET deleted_at = NOW() WHERE id = $1")
        .bind(pos_id)
        .execute(&pool)
        .await
        .expect("soft-delete position");
    sqlx::query("UPDATE isahl.zc_id_category SET deleted_at = NOW() WHERE id = $1")
        .bind(cat_id)
        .execute(&pool)
        .await
        .expect("soft-delete category");

    assert!(n1 >= 1, "首次迁移应复制 ≥1 条 association，got {n1}");
    assert_eq!(n2, 0, "第二次迁移必须零复制（幂等），got {n2}");
    assert_eq!(
        copied, 1,
        "legacy association 应复制 1 条到目标类别 UA（copied={copied}, n1={n1}）"
    );
    assert_eq!(src_alive, 1, "legacy association 必须保留（deleted_at IS NULL）");
}
