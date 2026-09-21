//! 主体删除级联回归测试（change: cascade-subject-delete-relations）
//!
//! 被测单元 = `subjects.rs::delete_subject_cascade`（`DELETE /service/isahl-db/subjects/{id}` 的
//! **唯一实现**，handler 与其共用）。判据（级联集合枚举见该 change 的 `design.md` §1/§2）：
//! ① 主体行软删（`zc_id_subjects.deleted_at IS NOT NULL`）；
//! ② 三类关系行全部软删——证照桥 `zc_id_entity_rr_identity`、联系人桥 `zc_id_entity_rr_contacts`
//!    （含私有链 `zc_id_contacts_rr_infos` / 值叶表 / `zc_id_contacts`）、视角关联行
//!    `zc_id_subj-post_rr_view` + 宿主标签行 `zc_id_relation-post_view_r_tags`
//!    （另含默认储元的 `zc_id_subjects_rr_account` / `zc_id_subjects_rr_place` 归属桥）；
//! ③ **活跃桥指向已删主体 = 0**（缺陷判据：修复前零级联 ⇒ 计数 ≠ 0，用例必失败）；
//! ④ 重复删除 → 404（`AliothError::NotFound`），且不改写首次删除的 `deleted_at`。
//!
//! 写入侧用 create 路径的**真实单元**（`write_subject_identity` / `add_entity_contact` /
//! `ensure_subject_view_pairs` + `sync_view_tags`），非手写桥行 SQL；储元归属桥按 create 主干
//! 同形补两条（`create_subject` L1452–L1492）。自清理：尾部硬删。

use common::testing::connect_test_db;
use common::AliothError;
use identity_org::handlers::identities::category_id_by_code;
use identity_org::handlers::subjects::{
    add_entity_contact, delete_subject_cascade, ensure_subject_view_pairs, sync_view_tags,
    write_subject_identity,
};
use sqlx::PgPool;
use std::time::{SystemTime, UNIX_EPOCH};

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn suffix() -> String {
    format!("{:x}", nanos() % 0xF_FFFF_FFFF)
}

/// 探针表 → 静态 SQL：表名在**编译期**由 `concat!` 固化（无运行期拼串、无 `AssertSqlSafe`）。
/// 表集 = 本例断言涉及的叶/桥表（调用方以字面量传名）；新增表 = 宏调用处加一行，
/// 未登记表名 fail-visible（不回落、不静默）。
macro_rules! soft_deleted_sqls {
    ($($table:literal),+ $(,)?) => {
        &[$((
            $table,
            concat!(
                "SELECT deleted_at IS NOT NULL FROM isahl.\"", $table, "\" WHERE id = $1"
            ),
        )),+]
    };
}

const SOFT_DELETED_SQL: &[(&str, &str)] = soft_deleted_sqls![
    "zc_id_subjects",
    "zc_id_entity_rr_identity",
    "zc_id_entity_rr_contacts",
    "zc_id_contacts_rr_infos",
    "zc_id_contacts",
    "zc_id_contact_infos",
    "zc_id_subj-post_rr_view",
    "zc_id_relation-post_view_r_tags",
    "zc_id_subj-position",
    "zc_id_identity",
    "zc_id_stor-acc-cash",
];

fn soft_deleted_sql(table: &str) -> &'static str {
    SOFT_DELETED_SQL
        .iter()
        .find(|(t, _)| *t == table)
        .map(|(_, sql)| *sql)
        .unwrap_or_else(|| panic!("未登记表名: {table}"))
}

/// 行是否已软删（`deleted_at IS NOT NULL`；表名 = 测试内常量）
async fn soft_deleted(pool: &PgPool, table: &str, id: i64) -> bool {
    sqlx::query_scalar(soft_deleted_sql(table))
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("probe {table}#{id}: {e}"))
}

/// 「活跃桥指向已删主体」计数（缺陷判据 ③）：级联集合内仍活动、宿主主体已软删的行数。
/// 表集 = `delete_subject_cascade` 的级联白名单（`design.md` §2）。
async fn dangling_bridges(pool: &PgPool, subject_id: i64) -> i64 {
    sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM (
             SELECT 1 FROM isahl."zc_id_entity_rr_identity" r JOIN isahl.zc_id_subjects s ON s.id = r.ref_left WHERE r.deleted_at IS NULL AND s.deleted_at IS NOT NULL AND s.id = $1
             UNION ALL SELECT 1 FROM isahl."zc_id_entity_rr_contacts" r JOIN isahl.zc_id_subjects s ON s.id = r.ref_left WHERE r.deleted_at IS NULL AND s.deleted_at IS NOT NULL AND s.id = $1
             UNION ALL SELECT 1 FROM isahl.zc_id_subjects_rr_account r JOIN isahl.zc_id_subjects s ON s.id = r.ref_left WHERE r.deleted_at IS NULL AND s.deleted_at IS NOT NULL AND s.id = $1
             UNION ALL SELECT 1 FROM isahl.zc_id_subjects_rr_place r JOIN isahl.zc_id_subjects s ON s.id = r.ref_left WHERE r.deleted_at IS NULL AND s.deleted_at IS NOT NULL AND s.id = $1
             UNION ALL SELECT 1 FROM isahl."zc_id_subj-post_rr_view" r JOIN isahl.zc_id_subjects s ON s.id = r.ref_right WHERE r.deleted_at IS NULL AND s.deleted_at IS NOT NULL AND s.id = $1
             UNION ALL SELECT 1 FROM isahl."zc_id_subj-post_rr_employee" r JOIN isahl.zc_id_subjects s ON s.id = r.ref_right WHERE r.deleted_at IS NULL AND s.deleted_at IS NOT NULL AND s.id = $1
             UNION ALL SELECT 1 FROM isahl."zc_id_subj-org_rr_employee" r JOIN isahl.zc_id_subjects s ON s.id = r.ref_right WHERE r.deleted_at IS NULL AND s.deleted_at IS NOT NULL AND s.id = $1
             UNION ALL SELECT 1 FROM isahl."zc_id_subj-group_rr_member" r JOIN isahl.zc_id_subjects s ON s.id = r.ref_right WHERE r.deleted_at IS NULL AND s.deleted_at IS NOT NULL AND s.id = $1
             UNION ALL SELECT 1 FROM isahl."zc_id_relation-cooperation_r_evaluation" r JOIN isahl.zc_id_subjects s ON s.id = r.ref_left WHERE r.deleted_at IS NULL AND s.deleted_at IS NOT NULL AND s.id = $1
             UNION ALL SELECT 1 FROM isahl."zc_id_relation-post_view_r_tags" t JOIN isahl."zc_id_subj-post_rr_view" r ON r.id = t.ref_left JOIN isahl.zc_id_subjects s ON s.id = r.ref_right WHERE t.deleted_at IS NULL AND r.deleted_at IS NULL AND s.deleted_at IS NOT NULL AND s.id = $1
             UNION ALL SELECT 1 FROM isahl.zc_id_contacts c JOIN isahl."zc_id_entity_rr_contacts" rc ON rc.ref_right = c.id JOIN isahl.zc_id_subjects s ON s.id = rc.ref_left WHERE c.deleted_at IS NULL AND rc.deleted_at IS NULL AND s.deleted_at IS NOT NULL AND s.id = $1
           ) x"#,
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await
    .expect("dangling bridge count")
}

/// 该主体名下**活跃关系行**计数（删除前判别力前置：>0 才说明用例能抓住零级联缺陷）
async fn active_relation_rows(pool: &PgPool, subject_id: i64) -> i64 {
    sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM (
             SELECT 1 FROM isahl."zc_id_entity_rr_identity" r WHERE r.ref_left = $1 AND r.deleted_at IS NULL
             UNION ALL SELECT 1 FROM isahl."zc_id_entity_rr_contacts" r WHERE r.ref_left = $1 AND r.deleted_at IS NULL
             UNION ALL SELECT 1 FROM isahl.zc_id_subjects_rr_account r WHERE r.ref_left = $1 AND r.deleted_at IS NULL
             UNION ALL SELECT 1 FROM isahl.zc_id_subjects_rr_place r WHERE r.ref_left = $1 AND r.deleted_at IS NULL
             UNION ALL SELECT 1 FROM isahl."zc_id_subj-post_rr_view" r WHERE r.ref_right = $1 AND r.deleted_at IS NULL
             UNION ALL SELECT 1 FROM isahl."zc_id_relation-post_view_r_tags" t JOIN isahl."zc_id_subj-post_rr_view" r ON r.id = t.ref_left WHERE r.ref_right = $1 AND r.deleted_at IS NULL AND t.deleted_at IS NULL
             UNION ALL SELECT 1 FROM isahl.zc_id_contacts c JOIN isahl."zc_id_entity_rr_contacts" rc ON rc.ref_right = c.id WHERE rc.ref_left = $1 AND rc.deleted_at IS NULL AND c.deleted_at IS NULL
           ) x"#,
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await
    .expect("active relation rows")
}

/// 建测试主体（前真叶表 `zc_id_orga-non-banking-legal`），返回 id
async fn create_test_subject(pool: &PgPool, tag: &str) -> (i64, String) {
    let sfx = suffix();
    let notice = format!("级联删除测试主体-{tag}-{sfx}");
    let code = format!("T-CASCDEL-{tag}-{sfx}");
    // 坐标三元组（坐标写入契约 §6.12 声明即必须；类派生源 §4.3.3）——与同 crate 既有用例同口径
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve subject coords");
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_orga-non-banking-legal"
               (notice, code, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, 1, $3, $4, $5) RETURNING id"#,
    )
    .bind(&notice)
    .bind(&code)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("insert test subject");
    (id, notice)
}

struct Fixture {
    subject_id: i64,
    identity_id: i64,
    identity_bridge_id: i64,
    contact_id: i64,
    contact_bridge_id: i64,
    info_ids: Vec<i64>,
    rr_infos_id: i64,
    pair_ids: Vec<i64>,
    tag_ids: Vec<i64>,
    post_id: i64,
    cash_id: i64,
    place_id: i64,
}

impl Fixture {
    /// 尾部硬删自清理（顺序：标签 → 关联行 → 自动岗位 → 桥 → 链 → 储元 → 主体）
    async fn cleanup(&self, pool: &PgPool) {
        sqlx::query(
            r#"DELETE FROM isahl."zc_id_relation-post_view_r_tags" WHERE ref_left = ANY($1)"#,
        )
        .bind(&self.pair_ids)
        .execute(pool)
        .await
        .expect("cleanup tags");
        sqlx::query(r#"DELETE FROM isahl."zc_id_subj-post_rr_view" WHERE ref_right = $1"#)
            .bind(self.subject_id)
            .execute(pool)
            .await
            .expect("cleanup view pairs");
        sqlx::query(
            r#"DELETE FROM isahl."zc_id_subj-position" WHERE code = 'POST-AUTO-' || $1::text"#,
        )
        .bind(self.subject_id)
        .execute(pool)
        .await
        .expect("cleanup auto position");
        sqlx::query(r#"DELETE FROM isahl."zc_id_entity_rr_identity" WHERE ref_left = $1"#)
            .bind(self.subject_id)
            .execute(pool)
            .await
            .expect("cleanup identity bridges");
        sqlx::query(r#"DELETE FROM isahl.zc_id_identity WHERE id = $1"#)
            .bind(self.identity_id)
            .execute(pool)
            .await
            .expect("cleanup identity");
        sqlx::query(r#"DELETE FROM isahl."zc_id_entity_rr_contacts" WHERE ref_left = $1"#)
            .bind(self.subject_id)
            .execute(pool)
            .await
            .expect("cleanup contact bridges");
        // 值叶表随父表 DELETE 一并清理（PG 继承）
        sqlx::query(r#"DELETE FROM isahl."zc_id_contact_infos" WHERE id = ANY($1)"#)
            .bind(&self.info_ids)
            .execute(pool)
            .await
            .expect("cleanup contact infos");
        sqlx::query(r#"DELETE FROM isahl.zc_id_contacts_rr_infos WHERE ref_left = $1"#)
            .bind(self.contact_id)
            .execute(pool)
            .await
            .expect("cleanup rr_infos");
        sqlx::query(r#"DELETE FROM isahl.zc_id_contacts WHERE id = $1"#)
            .bind(self.contact_id)
            .execute(pool)
            .await
            .expect("cleanup contacts");
        sqlx::query(r#"DELETE FROM isahl.zc_id_subjects_rr_account WHERE ref_left = $1"#)
            .bind(self.subject_id)
            .execute(pool)
            .await
            .expect("cleanup rr_account");
        sqlx::query(r#"DELETE FROM isahl."zc_id_stor-acc-cash" WHERE id = $1"#)
            .bind(self.cash_id)
            .execute(pool)
            .await
            .expect("cleanup cash account");
        sqlx::query(r#"DELETE FROM isahl.zc_id_subjects_rr_place WHERE ref_left = $1"#)
            .bind(self.subject_id)
            .execute(pool)
            .await
            .expect("cleanup rr_place");
        sqlx::query(r#"DELETE FROM isahl."zc_id_stor-plc-asset" WHERE id = $1"#)
            .bind(self.place_id)
            .execute(pool)
            .await
            .expect("cleanup asset place");
        sqlx::query(r#"DELETE FROM isahl."zc_id_orga-non-banking-legal" WHERE id = $1"#)
            .bind(self.subject_id)
            .execute(pool)
            .await
            .expect("cleanup subject");
    }
}

/// 建「证照 + 联系人 + 视角标签 + 默认储元归属桥」的完整测试主体（create 路径真实写入单元）
async fn build_fixture(pool: &PgPool, tag: &str) -> Fixture {
    let (subject_id, notice) = create_test_subject(pool, tag).await;
    // 坐标三元组（§6.12 声明即必须）：储元叶表插入同口径取值
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve stor coords");
    let sfx = suffix();
    let cert_no = format!("91330100{:010}", nanos() % 10_000_000_000);
    let category_id = category_id_by_code(pool, "BUSINESS_LICENSE")
        .await
        .expect("category BUSINESS_LICENSE");

    let mut tx = pool.begin().await.expect("begin tx");
    let identity_id = write_subject_identity(
        &mut tx,
        subject_id,
        &notice,
        0,
        &cert_no,
        &format!("级联测试证照-{sfx}"),
        category_id,
        1,
    )
    .await
    .expect("write identity");
    let contact_id = add_entity_contact(
        &mut tx,
        subject_id,
        &notice,
        None,
        Some("telephone"),
        &format!("0571-{}-{}", nanos() % 10_000_000, sfx),
        true,
        1,
    )
    .await
    .expect("write contact");
    let pair_ids = ensure_subject_view_pairs(&mut tx, subject_id, None, 1)
        .await
        .expect("ensure view pairs");
    assert!(!pair_ids.is_empty(), "create 路径必须挂出视角关联行");
    for pair_id in &pair_ids {
        sync_view_tags(&mut tx, *pair_id, &["VIEW-CUST".to_string()], 1)
            .await
            .expect("sync view tags");
    }
    tx.commit().await.expect("commit create");

    // 默认储元归属桥（create_subject 主干 L1452–L1492 同形：账户-现金 + 财产储位）
    let cash_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_stor-acc-cash"
               (notice, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, 1, $2, $3, $4) RETURNING id"#,
    )
    .bind(format!("{notice} 现金账户"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("insert cash account");
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_subjects_rr_account (notice, ref_left, ref_right, created_by_id, updated_by_id)
           VALUES ($1, $2, $3, 1, 1)"#,
    )
    .bind(format!("subject-{subject_id} 账户"))
    .bind(subject_id)
    .bind(cash_id)
    .execute(pool)
    .await
    .expect("insert account bridge");
    let place_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_stor-plc-asset"
               (notice, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, 1, $2, $3, $4) RETURNING id"#,
    )
    .bind(format!("{notice} 财产储位"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("insert asset place");
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_subjects_rr_place (notice, ref_left, ref_right, created_by_id, updated_by_id)
           VALUES ($1, $2, $3, 1, 1)"#,
    )
    .bind(format!("subject-{subject_id} 储位"))
    .bind(subject_id)
    .bind(place_id)
    .execute(pool)
    .await
    .expect("insert place bridge");

    let identity_bridge_id: i64 = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_entity_rr_identity" WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await
    .expect("identity bridge id");
    let contact_bridge_id: i64 = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_entity_rr_contacts" WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await
    .expect("contact bridge id");
    let rr_infos_id: i64 = sqlx::query_scalar(
        r#"SELECT id FROM isahl.zc_id_contacts_rr_infos WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(contact_id)
    .fetch_one(pool)
    .await
    .expect("rr_infos id");
    let info_ids: Vec<i64> = sqlx::query_scalar(
        r#"SELECT ref_right FROM isahl.zc_id_contacts_rr_infos WHERE ref_left = $1"#,
    )
    .bind(contact_id)
    .fetch_all(pool)
    .await
    .expect("info ids");
    let tag_ids: Vec<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_relation-post_view_r_tags" WHERE ref_left = ANY($1) AND deleted_at IS NULL"#,
    )
    .bind(&pair_ids)
    .fetch_all(pool)
    .await
    .expect("tag ids");
    let post_id: i64 = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_subj-position" WHERE code = 'POST-AUTO-' || $1::text AND deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await
    .expect("auto position id");

    Fixture {
        subject_id,
        identity_id,
        identity_bridge_id,
        contact_id,
        contact_bridge_id,
        info_ids,
        rr_infos_id,
        pair_ids,
        tag_ids,
        post_id,
        cash_id,
        place_id,
    }
}

/// ① 主体软删 ② 三类关系行全软删 ③ 活跃桥指向已删主体 = 0 ④ 重复删除 404
#[tokio::test]
async fn delete_subject_cascades_relations_and_clears_dangling_bridges() {
    let pool = connect_test_db().await;
    let f = build_fixture(&pool, "cascade").await;

    // 前置（判别力）：删除前确有活跃关系行 —— 零级联实现下删除后 ③ 计数必 ≠ 0
    assert!(
        active_relation_rows(&pool, f.subject_id).await > 0,
        "前置：删除前必须有活跃关系行（否则用例无判别力）"
    );
    assert!(!f.tag_ids.is_empty(), "前置：视角标签行必须已写入");

    let cascaded = delete_subject_cascade(&pool, f.subject_id, 1)
        .await
        .expect("delete subject");
    assert_eq!(cascaded.identity_bridges, 1, "级联计数 {cascaded:?}");
    assert_eq!(cascaded.contact_bridges, 1, "级联计数 {cascaded:?}");
    assert_eq!(
        cascaded.contact_chain, 3,
        "链三段（值行 + rr_infos + 联系人）计数 {cascaded:?}"
    );
    assert_eq!(cascaded.account_bridges, 1, "级联计数 {cascaded:?}");
    assert_eq!(cascaded.place_bridges, 1, "级联计数 {cascaded:?}");
    assert_eq!(
        cascaded.view_pairs,
        f.pair_ids.len() as u64,
        "级联计数 {cascaded:?}"
    );
    assert_eq!(
        cascaded.view_tags,
        f.tag_ids.len() as u64,
        "级联计数 {cascaded:?}"
    );
    assert_eq!(cascaded.auto_positions, 1, "级联计数 {cascaded:?}");

    // ① 主体软删
    assert!(
        soft_deleted(&pool, "zc_id_subjects", f.subject_id).await,
        "主体行必须软删"
    );
    // ② 三类关系行全部软删
    assert!(
        soft_deleted(&pool, "zc_id_entity_rr_identity", f.identity_bridge_id).await,
        "证照桥必须软删"
    );
    assert!(
        soft_deleted(&pool, "zc_id_entity_rr_contacts", f.contact_bridge_id).await,
        "联系人桥必须软删"
    );
    assert!(
        soft_deleted(&pool, "zc_id_contacts_rr_infos", f.rr_infos_id).await,
        "联系人链 rr_infos 必须软删"
    );
    assert!(
        soft_deleted(&pool, "zc_id_contacts", f.contact_id).await,
        "私有联系人本体必须软删"
    );
    for info_id in &f.info_ids {
        assert!(
            soft_deleted(&pool, "zc_id_contact_infos", *info_id).await,
            "联系方式值行 {info_id} 必须软删"
        );
    }
    for pair_id in &f.pair_ids {
        assert!(
            soft_deleted(&pool, "zc_id_subj-post_rr_view", *pair_id).await,
            "视角关联行 {pair_id} 必须软删"
        );
    }
    for tag_id in &f.tag_ids {
        assert!(
            soft_deleted(&pool, "zc_id_relation-post_view_r_tags", *tag_id).await,
            "宿主标签行 {tag_id} 必须随关联行软删"
        );
    }
    assert!(
        soft_deleted(&pool, "zc_id_subj-position", f.post_id).await,
        "POST-AUTO 自动岗位必须随主体软删"
    );
    // 可共享本体行保留（只删桥）：证照按 code 复用、储元可被其他归属桥引用
    assert!(
        !soft_deleted(&pool, "zc_id_identity", f.identity_id).await,
        "证照本体不得被级联软删"
    );
    assert!(
        !soft_deleted(&pool, "zc_id_stor-acc-cash", f.cash_id).await,
        "账户储元本体不得被级联软删"
    );

    // ③ 活跃桥指向已删主体 = 0（悬空引用清零）
    assert_eq!(
        dangling_bridges(&pool, f.subject_id).await,
        0,
        "删除后 MUST NOT 留活跃关系行指向已删主体"
    );

    // ④ 重复删除 → 404，且不改写首次删除的 deleted_at
    let err = delete_subject_cascade(&pool, f.subject_id, 1)
        .await
        .expect_err("重复删除必须返回 NotFound");
    assert!(
        matches!(err, AliothError::NotFound(_)),
        "重复删除错误类型必须 NotFound，实得 {err:?}"
    );

    f.cleanup(&pool).await;
}

/// 共享守卫：联系人与自动岗位被其他实体/主体引用时，级联 MUST NOT 误删
#[tokio::test]
async fn delete_subject_keeps_shared_contact_chain() {
    let pool = connect_test_db().await;
    let f = build_fixture(&pool, "shared").await;
    let (other_id, other_notice) = create_test_subject(&pool, "shared-other").await;

    // 另一个实体共享同一联系人（桥 ref_left = 另一主体，ref_right = 同一联系人）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_entity_rr_contacts"
           (id, code, notice, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_uid(251), $1, $2, $3, $4, 1)"#,
    )
    .bind(format!("REL-CT-{other_id}"))
    .bind(format!("{other_notice} 联系方式"))
    .bind(other_id)
    .bind(f.contact_id)
    .execute(&pool)
    .await
    .expect("share contact");

    let cascaded = delete_subject_cascade(&pool, f.subject_id, 1)
        .await
        .expect("delete subject");

    assert_eq!(
        cascaded.contact_bridges, 1,
        "本主体联系人桥必须软删 {cascaded:?}"
    );
    assert_eq!(
        cascaded.contact_chain, 0,
        "共享联系人链 MUST NOT 被软删 {cascaded:?}"
    );
    assert!(
        !soft_deleted(&pool, "zc_id_contacts", f.contact_id).await,
        "共享联系人本体必须保持活跃"
    );
    assert!(
        !soft_deleted(&pool, "zc_id_contacts_rr_infos", f.rr_infos_id).await,
        "共享联系人链 rr_infos 必须保持活跃"
    );
    assert_eq!(
        dangling_bridges(&pool, f.subject_id).await,
        0,
        "本主体侧不得留活跃关系行"
    );

    // 清理共享桥（桥 → 联系人链 → 另一主体），再走常规自清理
    sqlx::query(r#"DELETE FROM isahl."zc_id_entity_rr_contacts" WHERE ref_left = $1"#)
        .bind(other_id)
        .execute(&pool)
        .await
        .expect("cleanup shared bridge");
    sqlx::query(r#"DELETE FROM isahl."zc_id_orga-non-banking-legal" WHERE id = $1"#)
        .bind(other_id)
        .execute(&pool)
        .await
        .expect("cleanup other subject");
    f.cleanup_shared(&pool).await;
}

impl Fixture {
    /// 共享场景自清理（联系人链未被级联，须显式清理）
    async fn cleanup_shared(&self, pool: &PgPool) {
        sqlx::query(r#"DELETE FROM isahl."zc_id_contact_infos" WHERE id = ANY($1)"#)
            .bind(&self.info_ids)
            .execute(pool)
            .await
            .expect("cleanup shared infos");
        sqlx::query(r#"DELETE FROM isahl.zc_id_contacts_rr_infos WHERE ref_left = $1"#)
            .bind(self.contact_id)
            .execute(pool)
            .await
            .expect("cleanup shared rr_infos");
        sqlx::query(r#"DELETE FROM isahl.zc_id_contacts WHERE id = $1"#)
            .bind(self.contact_id)
            .execute(pool)
            .await
            .expect("cleanup shared contacts");
        self.cleanup(pool).await;
    }
}
