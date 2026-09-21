//! 主体证照写入幂等集成测试（change: fix-subject-identity-write-idempotency）
//!
//! 被测单元 = `subjects.rs::write_subject_identity`（`POST /subjects` identities 分支的
//! **唯一写入实现**，handler 与其共用）：
//! ① 同 `cert_no` 连续两次写件 → 恰 1 行活动 `zc_id_identity`（`code` == `cert_no` == `identity`）
//!    + 恰 1 行活动桥 `zc_id_entity_rr_identity`；
//! ② 种子形态（`code` = 统一社会信用代码，含种子形态桥）先存在 → API 写件复用既有 id，
//!    活动证照行与桥行均不增。
//!
//! 依赖：test 库主体叶表 `zc_id_orga-non-banking-legal` + 字典 `zc_id_cate-identity`
//! BUSINESS_LICENSE + 维度行 JE/FJA/↑_DA。自清理：尾部硬删（桥 → 证照 → 主体）。

use common::testing::connect_test_db;
use identity_org::handlers::identities::category_id_by_code;
use identity_org::handlers::subjects::write_subject_identity;
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

/// 18 位统一社会信用代码形态的唯一证件号（跨运行/跨用例不冲突）
fn unique_uscc() -> String {
    format!("91330100{:010}", nanos() % 10_000_000_000)
}

/// 建测试主体（前真叶表 `zc_id_orga-non-banking-legal`），返回 (id, notice, code)
async fn create_test_subject(pool: &PgPool, tag: &str) -> (i64, String, String) {
    let sfx = suffix();
    let notice = format!("幂等测试主体-{tag}-{sfx}");
    let code = format!("T-IDEM-{tag}-{sfx}");
    // 坐标声明（坐标写入契约 §6.12：声明即必须）——与同 crate 既有用例同口径 TX/FJA/↓_GG
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
    (id, notice, code)
}

/// 活动证照行计数（按 code）
async fn live_identities(
    pool: &PgPool,
    cert_no: &str,
) -> Vec<(i64, Option<String>, Option<String>)> {
    sqlx::query_as(
        r#"SELECT id, code, identity FROM isahl."zc_id_identity"
           WHERE code = $1 AND deleted_at IS NULL ORDER BY id"#,
    )
    .bind(cert_no)
    .fetch_all(pool)
    .await
    .expect("select live identities")
}

/// 活动桥行计数（主体 ↔ 证照）
async fn live_bridges(pool: &PgPool, subject_id: i64, identity_id: i64) -> i64 {
    sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl."zc_id_entity_rr_identity"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .bind(identity_id)
    .fetch_one(pool)
    .await
    .expect("count live bridges")
}

/// 尾部硬删（桥 → 证照 → 主体），测试库零残留
async fn cleanup(pool: &PgPool, subject_id: i64, identity_id: i64) {
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_entity_rr_identity"
           WHERE ref_left = $1 OR ref_right = $2"#,
    )
    .bind(subject_id)
    .bind(identity_id)
    .execute(pool)
    .await
    .expect("delete bridges");
    sqlx::query(r#"DELETE FROM isahl."zc_id_identity" WHERE id = $1"#)
        .bind(identity_id)
        .execute(pool)
        .await
        .expect("delete identity");
    sqlx::query(r#"DELETE FROM isahl."zc_id_orga-non-banking-legal" WHERE id = $1"#)
        .bind(subject_id)
        .execute(pool)
        .await
        .expect("delete subject");
}

/// ① 同 cert_no 连续两次写件 → 恰 1 行活动证照（code == cert_no == identity）+ 恰 1 行活动桥
#[tokio::test]
async fn repeated_write_keeps_single_live_identity_and_bridge() {
    let pool = connect_test_db().await;
    let (subject_id, notice, _subject_code) = create_test_subject(&pool, "repeat").await;
    let cert_no = unique_uscc();
    let dname = format!("幂等测试证照-{}", suffix());
    let category_id = category_id_by_code(&pool, "BUSINESS_LICENSE")
        .await
        .expect("category BUSINESS_LICENSE");

    let mut tx = pool.begin().await.expect("begin tx");
    let first = write_subject_identity(
        &mut tx,
        subject_id,
        &notice,
        0,
        &cert_no,
        &dname,
        category_id,
        1,
    )
    .await
    .expect("first write");
    // 第二次同 cert_no（模拟重复建档 / 种子重放后的 API 再写）
    let second = write_subject_identity(
        &mut tx,
        subject_id,
        &notice,
        1,
        &cert_no,
        &dname,
        category_id,
        1,
    )
    .await
    .expect("second write");
    tx.commit().await.expect("commit");

    assert_eq!(second, first, "同 cert_no 第二次写件必须复用既有证照 id");

    let rows = live_identities(&pool, &cert_no).await;
    assert_eq!(
        rows.len(),
        1,
        "同 cert_no 活动证照必须恰 1 行，实得 {:?}",
        rows
    );
    assert_eq!(rows[0].0, first, "活动行 id 必须 = 首次写件返回 id");
    assert_eq!(
        rows[0].1.as_deref(),
        Some(cert_no.as_str()),
        "证照行 code 必须 = cert_no（修复前为空）"
    );
    assert_eq!(
        rows[0].2.as_deref(),
        Some(cert_no.as_str()),
        "证照行 identity 必须 = cert_no"
    );

    let bridges = live_bridges(&pool, subject_id, first).await;
    assert_eq!(bridges, 1, "重复写件不得叠桥（活动桥行恰 1）");

    cleanup(&pool, subject_id, first).await;
}

/// ② 种子形态证照（code = USCC + 种子形态桥）先存在 → API 写件复用既有 id，活动行不增
#[tokio::test]
async fn api_write_reuses_seed_shaped_identity() {
    let pool = connect_test_db().await;
    let (subject_id, notice, _subject_code) = create_test_subject(&pool, "seed").await;
    let cert_no = unique_uscc();
    let dname = format!("幂等测试证照-{}", suffix());
    let category_id = category_id_by_code(&pool, "BUSINESS_LICENSE")
        .await
        .expect("category BUSINESS_LICENSE");

    // 种子形态（同 Pre-Proc/WZ/seed/seed-wz-trading-subjects.sql §5/§6）：证照行 + 桥行
    // （坐标取种子口径 TX/FJA/↓_GG，与 API 路径 JE/FJA/↑_DA 无关——本用例只验复用与不叠行）
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("TX", "FJA", "↓_GG"))
            .await
            .expect("resolve seed coords");
    let seed_identity_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_identity"
           (code, notice, identity, ck_category, created_by_id, updated_by_id,
            dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, 1, 1, $5, $6, $7) RETURNING id"#,
    )
    .bind(&cert_no)
    .bind(format!("营业执照-{dname}"))
    .bind(&cert_no)
    .bind(category_id)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(&pool)
    .await
    .expect("insert seed-shaped identity");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_entity_rr_identity"
           (notice, ref_left, ref_right, created_by_id, updated_by_id)
           VALUES ('营业执照', $1, $2, 1, 1)"#,
    )
    .bind(subject_id)
    .bind(seed_identity_id)
    .execute(&pool)
    .await
    .expect("insert seed-shaped bridge");

    let mut tx = pool.begin().await.expect("begin tx");
    let reused = write_subject_identity(
        &mut tx,
        subject_id,
        &notice,
        0,
        &cert_no,
        &dname,
        category_id,
        1,
    )
    .await
    .expect("api write over seed row");
    tx.commit().await.expect("commit");

    assert_eq!(
        reused, seed_identity_id,
        "API 写件必须复用种子形态既有证照 id（不新建）"
    );

    let rows = live_identities(&pool, &cert_no).await;
    assert_eq!(rows.len(), 1, "种子 + API 双写后活动证照仍恰 1 行");
    assert_eq!(rows[0].1.as_deref(), Some(cert_no.as_str()));

    let bridges = live_bridges(&pool, subject_id, seed_identity_id).await;
    assert_eq!(bridges, 1, "种子形态桥已存在 → API 不得再叠一条");

    cleanup(&pool, subject_id, seed_identity_id).await;
}
