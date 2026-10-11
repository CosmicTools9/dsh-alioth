//! D-2b 派生/迁移幂等集成测试（?11，D-2 在生产跑迁移前 MUST）。
//!
//! 覆盖（common::ngac_policy / common::ngac_org 唯一实现，SSO/Gateway 消费同源）：
//! - [`common::ngac_policy::derive_from_class`] 幂等：active class（
//!   `ua_template.name_rule = 'position:{code}'`）+ active rule 预置后
//!   连跑两次 → 首次 UA/OA/association 各 ≥1 新增，第二次全 0；
//! - [`common::ngac_org::migrate_legacy_position_associations`] 幂等：存量实例码
//!   UA（`position:{实例code}`，cognition 物化标记）+ 存活 association + 真实岗位行
//!   （类列按 `dk_function` 派生 `_f_='实现'/ _t_='实例'`＝`common::real_position_row!` 通过，
//!   `ck_category` → 基表类别行）→ 连跑两次 → 首次复制 ≥1、
//!   第二次 0，legacy association 原样保留。
//!
//! 运行（连共享测试库，须 `*_test` 库；common::testing 强制校验）：
//!   CARGO_TARGET_DIR=<仓父目录>/.tmp/alioth-check DATABASE_URL=postgres://localhost/aliothstudio_test \
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

/// 分级标签（§2.7 资源侧 carrier）：canonical 声明 → 派生集合 OA `property.label`。
const LABEL_A: &str = "zz_itest_label_a";
const LABEL_B: &str = "zz_itest_label_b";

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
            prohibition_template jsonb,
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
        "ALTER TABLE isahl_auth.org_policy_class ADD COLUMN IF NOT EXISTS prohibition_template jsonb",
    )
    .execute(pool)
    .await
    .expect("ensure org_policy_class.prohibition_template column");
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.org_policy_rule (
            id bigserial PRIMARY KEY,
            policy_class_id bigint NOT NULL,
            resource_type text NOT NULL,
            actions jsonb NOT NULL DEFAULT '[]',
            label_code text,
            obligation jsonb,
            condition jsonb,
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
    // 早期 bootstrap 建的表可能缺接线列（加性自愈；生产 DDL 已在位时为空转）。
    sqlx::query("ALTER TABLE isahl_auth.org_policy_rule ADD COLUMN IF NOT EXISTS obligation jsonb")
        .execute(pool)
        .await
        .expect("ensure org_policy_rule.obligation column");
    sqlx::query("ALTER TABLE isahl_auth.org_policy_rule ADD COLUMN IF NOT EXISTS condition jsonb")
        .execute(pool)
        .await
        .expect("ensure org_policy_rule.condition column");
}

/// 派生 UA（`position:{class.code}`）上的主体 clearance（裁决 B-0 的生产方）。
async fn derived_ua_clearance(pool: &PgPool, ua_name: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT property ->> 'clearance' FROM isahl_auth.ngac_user_attribute \
          WHERE o_name = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(ua_name)
    .fetch_one(pool)
    .await
    .expect("read derived UA clearance")
}

/// 派生集合 OA（`fk_resource = 0`）上的分级标签（§2.7 资源侧 carrier）。
async fn derived_collection_label(pool: &PgPool, resource_type: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT property ->> 'label' FROM isahl_auth.ngac_object_attribute \
          WHERE resource_type = $1 AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(resource_type)
    .fetch_one(pool)
    .await
    .expect("read derived collection OA label")
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
            (code, notice, scope, ua_template, state, label_code)
        VALUES ($1, $1 || '-notice', '{"actions":["read"]}', '{"name_rule":"position:{code}"}', 'active', $2)
        RETURNING id
        "#,
    )
    .bind(&code)
    .bind(LABEL_A)
    .fetch_one(&pool)
    .await
    .expect("insert active class");
    let rule_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl_auth.org_policy_rule
            (policy_class_id, subject_code, resource_type, actions, state)
        VALUES ($1, 'subj-itest', $2, '["read"]', 'active')
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

    // 分级标签（NGAC_SPEC §2.7 资源侧 carrier）：canonical 声明 MUST 落到派生集合 OA 的
    // `property.label`，且重派生按**差异**刷新（无变更 ⇒ 0，避免把刷新计入新增）。
    assert_eq!(
        derived_collection_label(&pool, &resource_type)
            .await
            .as_deref(),
        Some(LABEL_A),
        "首次派生 MUST 把 class.label_code 写入集合 OA property.label"
    );
    assert_eq!(st2.oa_labels_refreshed, 0, "标签未变 ⇒ MUST NOT 刷新");
    assert_eq!(
        derived_ua_clearance(&pool, &ua_name).await.as_deref(),
        Some(LABEL_A),
        "裁决 B-0：首次派生 MUST 把 class.label_code 写入派生 UA property.clearance"
    );
    assert_eq!(
        st2.ua_clearance_refreshed, 0,
        "标签未变 ⇒ MUST NOT 刷新 UA clearance"
    );
    sqlx::query("UPDATE isahl_auth.org_policy_class SET label_code = $2 WHERE id = $1")
        .bind(class_id)
        .bind(LABEL_B)
        .execute(&pool)
        .await
        .expect("update class label");
    let st3 = derive_from_class(&pool, class_id)
        .await
        .expect("third derive");
    assert_eq!(
        st3.oa_labels_refreshed, 1,
        "标签变更后重派生 MUST 刷新 1 行，got {st3:?}"
    );
    assert_eq!(st3.oa_created, 0, "刷新 MUST NOT 计入新增（幂等判据）");
    assert_eq!(
        st3.ua_clearance_refreshed, 1,
        "裁决 B-0：标签变更后重派生 MUST 刷新派生 UA 的 clearance，got {st3:?}"
    );
    assert_eq!(
        derived_ua_clearance(&pool, &ua_name).await.as_deref(),
        Some(LABEL_B),
        "裁决 B-0：刷新后派生 UA clearance MUST = 新 canonical 声明"
    );
    assert_eq!(
        derived_collection_label(&pool, &resource_type)
            .await
            .as_deref(),
        Some(LABEL_B),
        "刷新后集合 OA 标签 MUST = 新 canonical 声明"
    );

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
        sqlx::query("UPDATE isahl_auth.ngac_user_attribute SET deleted_at = NOW() WHERE id = $1")
            .bind(u)
            .execute(&pool)
            .await
            .expect("soft-delete derived UA");
    }
    if let Some(o) = oa_id {
        sqlx::query("UPDATE isahl_auth.ngac_object_attribute SET deleted_at = NOW() WHERE id = $1")
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

    // 基表类别行（tableoid = isahl.zc_id_category：直接 INSERT 基表——认知派生/迁移只认基表，
    // 子族字典行不派生，org_tree.rs:715 裁决；zc_id_cate-organization 子表行会被 tableoid 过滤）
    let cat_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_category\" (code, notice, created_at, updated_at) \
         VALUES ($1, $2, NOW(), NOW()) RETURNING id",
    )
    .bind(&cat_code)
    .bind(MARKER)
    .fetch_one(&pool)
    .await
    .expect("insert base category row");

    // 真实岗位行：类列由 dk_function 前缀派生（`_f_='实现'/ _t_='实例'`，即
    // `common::real_position_row!` 通过的行——类列 NULL **不是**真实岗位判据）、
    // ck_category → 基表类别行
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("TX", "FJA", "↓_GG"))
            .await
            .expect("resolve dk coords");
    let pos_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_subj-position\" \
            (code, ck_category, _f_, _t_, notice, created_at, updated_at, dk_scene, dk_factor, dk_function) \
         VALUES ($1, $2, '实现', '实例', $3, NOW(), NOW(), $4, $5, $6) RETURNING id",
    )
    .bind(&inst_code)
    .bind(cat_id)
    .bind(MARKER)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
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
    sqlx::query(
        "UPDATE isahl_auth.ngac_user_attribute SET deleted_at = NOW() WHERE id IN ($1, $2)",
    )
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
    assert_eq!(
        src_alive, 1,
        "legacy association 必须保留（deleted_at IS NULL）"
    );
}

// ------------------------------------------- 规则义务派生（wire-org-policy-declaration-fields）

/// `org_policy_rule.obligation` → `isahl_auth.ngac_obligation`：
/// 插入 → 幂等零新增 → 源变更差异刷新 → 源清空退役（系统墓碑）→ 源重填幂等复活 →
/// 类失效随撤销扫帚同批退役。
#[tokio::test]
async fn derive_rule_obligation_is_idempotent_and_retires() {
    let pool = connect_test_db().await;
    ensure_org_policy_ddl(&pool).await;
    // `ngac_obligation` 由 SSO ensure 建立；共享测试库缺表时自建最小契约面（幂等）。
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.ngac_obligation (
            id bigserial PRIMARY KEY,
            o_name text,
            event_pattern text,
            response_ops jsonb DEFAULT '[]'::jsonb,
            is_active boolean DEFAULT TRUE,
            priority integer DEFAULT 0,
            conditions jsonb DEFAULT '{}'::jsonb,
            phase text NOT NULL DEFAULT 'post',
            created_at timestamptz NOT NULL DEFAULT NOW(),
            updated_at timestamptz NOT NULL DEFAULT NOW(),
            created_by_id bigint,
            updated_by_id bigint,
            deleted_at timestamptz
        )"#,
    )
    .execute(&pool)
    .await
    .expect("bootstrap ngac_obligation DDL");

    let suf = mk_suffix();
    let code = format!("ngac_oblig_itest_{suf}");
    let resource_type = format!("ngac_itest_ob_{suf}");

    let class_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.org_policy_class
               (code, notice, scope, ua_template, state)
           VALUES ($1, $1 || '-notice', '{"actions":["read"]}',
                   '{"name_rule":"position:{code}"}', 'active')
           RETURNING id"#,
    )
    .bind(&code)
    .fetch_one(&pool)
    .await
    .expect("insert active class");
    let rule_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.org_policy_rule
               (policy_class_id, subject_code, resource_type, actions, obligation, state)
           VALUES ($1, 'subj-ob-itest', $2, '["read","approve"]', '"log_access"', 'active')
           RETURNING id"#,
    )
    .bind(class_id)
    .bind(&resource_type)
    .fetch_one(&pool)
    .await
    .expect("insert active rule with obligation");
    let o_name = format!("org_policy:{code}:{rule_id}");

    // (event_pattern, response_ops, is_active, phase, soft_deleted)
    let read_row = |pool: PgPool, name: String| async move {
        sqlx::query_as::<
            _,
            (
                Option<String>,
                Option<serde_json::Value>,
                bool,
                String,
                bool,
            ),
        >(
            "SELECT event_pattern, response_ops, is_active, phase, (deleted_at IS NOT NULL) \
               FROM isahl_auth.ngac_obligation WHERE o_name = $1 ORDER BY id DESC LIMIT 1",
        )
        .bind(name)
        .fetch_optional(&pool)
        .await
        .expect("read derived obligation row")
    };

    // 1) 首次派生：插入一行，事件绑定定位资源类型 + 动作集，phase=post（不改判）。
    let st1 = derive_from_class(&pool, class_id).await.expect("derive #1");
    assert_eq!(st1.obligations_created, 1, "首次派生应建 1 行，got {st1:?}");
    let (pat, ops, active, phase, deleted) = read_row(pool.clone(), o_name.clone())
        .await
        .expect("derived row exists after first derive");
    let pat = pat.expect("event_pattern MUST be set");
    assert!(
        pat.contains(&format!("resource.starts_with(\"{resource_type}:\")")),
        "event_pattern 须以 resource 前缀定位资源类型，got {pat}"
    );
    assert!(
        pat.contains("[\"read\", \"approve\"].contains(action)"),
        "非空动作集须追加 contains 判据，got {pat}"
    );
    assert_eq!(ops, Some(serde_json::json!("log_access")));
    assert!(active, "派生行 MUST is_active");
    assert_eq!(phase, "post", "规则义务 MUST 为 post 相（不改判）");
    assert!(!deleted);

    // 2) 幂等：重复派生零新增、零刷新。
    let st2 = derive_from_class(&pool, class_id).await.expect("derive #2");
    assert_eq!(
        (
            st2.obligations_created,
            st2.obligations_refreshed,
            st2.obligations_retired
        ),
        (0, 0, 0),
        "重复派生 MUST 零变更，got {st2:?}"
    );

    // 3) 源变更 ⇒ 差异刷新（1 行）。
    sqlx::query(
        "UPDATE isahl_auth.org_policy_rule SET obligation = '\"log_delete\"' WHERE id = $1",
    )
    .bind(rule_id)
    .execute(&pool)
    .await
    .expect("change rule obligation");
    let st3 = derive_from_class(&pool, class_id).await.expect("derive #3");
    assert_eq!(
        st3.obligations_refreshed, 1,
        "源变更须差异刷新 1 行，got {st3:?}"
    );
    assert_eq!(st3.obligations_created, 0, "刷新 MUST NOT 计入新增");
    let (_, ops3, _, _, del3) = read_row(pool.clone(), o_name.clone()).await.expect("row");
    assert_eq!(ops3, Some(serde_json::json!("log_delete")));
    assert!(!del3);

    // 4) 源清空 ⇒ 退役（系统墓碑）。
    sqlx::query("UPDATE isahl_auth.org_policy_rule SET obligation = NULL WHERE id = $1")
        .bind(rule_id)
        .execute(&pool)
        .await
        .expect("clear rule obligation");
    let st4 = derive_from_class(&pool, class_id).await.expect("derive #4");
    assert_eq!(st4.obligations_retired, 1, "源清空须退役 1 行，got {st4:?}");
    let (_, _, _, _, del4) = read_row(pool.clone(), o_name.clone()).await.expect("row");
    assert!(del4, "退役行 MUST 软删");

    // 5) 源重填 ⇒ 幂等复活系统墓碑（created=0 / refreshed=1）。
    sqlx::query(
        "UPDATE isahl_auth.org_policy_rule SET obligation = '\"log_access\"' WHERE id = $1",
    )
    .bind(rule_id)
    .execute(&pool)
    .await
    .expect("refill rule obligation");
    let st5 = derive_from_class(&pool, class_id).await.expect("derive #5");
    assert_eq!(
        (st5.obligations_created, st5.obligations_refreshed),
        (0, 1),
        "重填须复活系统墓碑（非新建），got {st5:?}"
    );
    let (_, _, _, _, del5) = read_row(pool.clone(), o_name.clone()).await.expect("row");
    assert!(!del5, "复活后 MUST 存活");

    // 6) 类失效 ⇒ 撤销扫帚同批退役派生义务。
    sqlx::query("UPDATE isahl_auth.org_policy_class SET state = 'retired' WHERE id = $1")
        .bind(class_id)
        .execute(&pool)
        .await
        .expect("retire class");
    let rev = common::ngac_policy::revoke_expired_class_derivations(&pool)
        .await
        .expect("revocation sweep");
    assert!(
        rev.obligations_revoked >= 1,
        "失效类的派生义务须同批退役，got {rev:?}"
    );
    let (_, _, _, _, del6) = read_row(pool.clone(), o_name.clone()).await.expect("row");
    assert!(del6, "失效类派生义务 MUST 软删");

    // 清理（不污染共享测试库）
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
}

// ------------------------- 切片 B：规则条件 + 负向模板派生（wire-org-policy-declaration-fields）

/// `org_policy_rule.condition` → 派生 association `conditions.rhai`（多 rule `||` 合成）；
/// `org_policy_class.prohibition_template` → 派生 `ngac_prohibition`（`conditions.rhai`）。
/// 覆盖：首次派生 / 幂等零变更 / 条件差异刷新 / 模板清空退役。
#[tokio::test]
async fn derive_conditions_and_prohibitions_are_wired_and_idempotent() {
    let pool = connect_test_db().await;
    ensure_org_policy_ddl(&pool).await;
    let suf = mk_suffix();
    let code = format!("ngac_cond_itest_{suf}");
    let ua_name = format!("position:{code}");
    let rt = format!("ngac_itest_cond_{suf}");
    let tmpl = "action == \"read\"";

    let class_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.org_policy_class
               (code, notice, scope, ua_template, prohibition_template, state)
           VALUES ($1, $1 || '-notice', '{"actions":["read","write"]}',
                   '{"name_rule":"position:{code}"}', $2, 'active')
           RETURNING id"#,
    )
    .bind(&code)
    .bind(serde_json::json!(tmpl))
    .fetch_one(&pool)
    .await
    .expect("insert class with prohibition_template");
    let cond_a = "action == \"read\"";
    let cond_b = "action == \"write\"";
    for (actions, cond) in [("[\"read\"]", cond_a), ("[\"write\"]", cond_b)] {
        sqlx::query(
            r#"INSERT INTO isahl_auth.org_policy_rule
                   (policy_class_id, subject_code, resource_type, actions, condition, state)
               VALUES ($1, 'subj-cond-itest', $2, $3::jsonb, $4, 'active')"#,
        )
        .bind(class_id)
        .bind(&rt)
        .bind(actions)
        .bind(serde_json::json!(cond))
        .execute(&pool)
        .await
        .expect("insert rule with condition");
    }

    let assoc_rhai = |pool: PgPool, ua: String, rt: String| async move {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT a.conditions ->> 'rhai' FROM isahl_auth.ngac_association a \
               JOIN isahl_auth.ngac_user_attribute ua ON ua.id = a.fk_user_attribute \
               JOIN isahl_auth.ngac_object_attribute oa ON oa.id = a.fk_object_attribute \
              WHERE ua.o_name = $1 AND oa.resource_type = $2 AND oa.fk_resource = 0 \
                AND a.deleted_at IS NULL",
        )
        .bind(ua)
        .bind(rt)
        .fetch_optional(&pool)
        .await
        .expect("read association rhai")
        .flatten()
    };
    let proh_name = format!("org_policy:{code}:deny-{rt}");
    let proh_row = |pool: PgPool, name: String| async move {
        sqlx::query_as::<_, (bool, Option<String>, bool)>(
            "SELECT (deleted_at IS NOT NULL), conditions ->> 'rhai', is_active \
               FROM isahl_auth.ngac_prohibition WHERE o_name = $1 ORDER BY id DESC LIMIT 1",
        )
        .bind(name)
        .fetch_optional(&pool)
        .await
        .expect("read prohibition")
    };

    // ① 首次派生：条件合成落 association；模板落 prohibition
    let st1 = derive_from_class(&pool, class_id).await.expect("derive #1");
    let expected = format!("({cond_a}) || ({cond_b})");
    assert_eq!(
        assoc_rhai(pool.clone(), ua_name.clone(), rt.clone())
            .await
            .as_deref(),
        Some(expected.as_str()),
        "同 resource_type 两 rule 的条件 MUST 合成为 (c1) || (c2)"
    );
    assert!(
        st1.conditions_refreshed >= 1,
        "首次派生应刷新 association 条件，got {st1:?}"
    );
    assert_eq!(
        st1.prohibitions_created, 1,
        "负向模板应派生 1 行 prohibition，got {st1:?}"
    );
    let (del, rhai, active) = proh_row(pool.clone(), proh_name.clone())
        .await
        .expect("prohibition row exists");
    assert!(!del && active);
    assert_eq!(rhai.as_deref(), Some(tmpl));

    // ② 幂等：重复派生零变更
    let st2 = derive_from_class(&pool, class_id).await.expect("derive #2");
    assert_eq!(
        (
            st2.conditions_refreshed,
            st2.prohibitions_created,
            st2.prohibitions_refreshed,
            st2.prohibitions_retired
        ),
        (0, 0, 0, 0),
        "重复派生 MUST 零变更，got {st2:?}"
    );

    // ③ 条件差异刷新：r1 条件改为与 r2 相同 ⇒ 去重后合成值变化 ⇒ 刷新 1 行
    sqlx::query(
        "UPDATE isahl_auth.org_policy_rule SET condition = $2 WHERE policy_class_id = $1 AND actions = '[\"read\"]'::jsonb",
    )
    .bind(class_id)
    .bind(serde_json::json!(cond_b))
    .execute(&pool)
    .await
    .expect("change rule condition");
    let st3 = derive_from_class(&pool, class_id).await.expect("derive #3");
    assert_eq!(
        st3.conditions_refreshed, 1,
        "条件变更 MUST 差异刷新 1 行，got {st3:?}"
    );
    assert_eq!(
        assoc_rhai(pool.clone(), ua_name.clone(), rt.clone())
            .await
            .as_deref(),
        Some(cond_b),
        "去重后合成值 MUST 收敛为单条条件"
    );

    // ④ 模板清空 ⇒ 派生 prohibition 退役（系统墓碑）
    sqlx::query("UPDATE isahl_auth.org_policy_class SET prohibition_template = NULL WHERE id = $1")
        .bind(class_id)
        .execute(&pool)
        .await
        .expect("clear prohibition template");
    let st4 = derive_from_class(&pool, class_id).await.expect("derive #4");
    assert_eq!(
        st4.prohibitions_retired, 1,
        "模板清空 MUST 退役派生 prohibition，got {st4:?}"
    );
    let (del4, _, active4) = proh_row(pool.clone(), proh_name.clone())
        .await
        .expect("row");
    assert!(del4 && !active4, "退役行 MUST 软删且 is_active=false");

    // 清理（不污染共享测试库）
    sqlx::query("UPDATE isahl_auth.ngac_prohibition SET deleted_at = NOW() WHERE o_name = $1")
        .bind(&proh_name)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(
        "UPDATE isahl_auth.org_policy_rule SET deleted_at = NOW() WHERE policy_class_id = $1",
    )
    .bind(class_id)
    .execute(&pool)
    .await
    .expect("soft-delete rules");
    sqlx::query("UPDATE isahl_auth.org_policy_class SET deleted_at = NOW() WHERE id = $1")
        .bind(class_id)
        .execute(&pool)
        .await
        .expect("soft-delete class");
}
