//! 联系方式 ↔ 联系人 双向解析集成测试（refactor-chat-ai-subject-identity-memory D6）。
//!
//! 覆盖新增的两个聚合函数：
//! - `ContactsService::resolve_contact_of_info`（联系方式 → 联系人，反向单跳）
//! - `ContactsService::resolve_preferred_info`（联系人 → 首选联系方式，正向单跳）
//!
//! 方向契约：消息层身份空间是**联系方式** id，联系人是聚合层产物。
//! 用唯一 code 自清（测试库 isahl 数据面），不依赖固定 id。

use ::common::testing::connect_test_db;
use framework_contacts::ContactsService;
use sqlx::PgPool;
use std::sync::LazyLock;

/// 本二进制内两用例共用同一组 fixture code（CODES）——并行会互删（cleanup 按 code
/// 清场），以进程级异步锁串行化（用例断言按 code，参数化会牵动夹具语义）。
static FIXTURE_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

const CODES: [&str; 3] = [
    "contact-rr-rev-test",
    "info-rev-test",
    "info-rev-test-other",
];

async fn cleanup(pool: &PgPool) {
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_contacts_rr_infos" WHERE ref_right IN (
               SELECT id FROM isahl.zc_id_contact_infos WHERE code = ANY($1)
           )"#,
    )
    .bind(&CODES[..])
    .execute(pool)
    .await
    .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_contact_infos WHERE code = ANY($1)"#)
        .bind(&CODES[..])
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_contacts WHERE code = $1"#)
        .bind(CODES[0])
        .execute(pool)
        .await
        .ok();
}

async fn insert_contact(pool: &PgPool, code: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl.zc_id_contacts (code, notice, dk_scene, dk_factor, dk_function)
           VALUES ($1, $1,
                   (SELECT id FROM isahl.zc_id_scene    WHERE code = 'JE'  AND deleted_at IS NULL LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor   WHERE code = 'GEC' AND deleted_at IS NULL LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↑_DA' AND deleted_at IS NULL LIMIT 1)) RETURNING id"#,
    )
    .bind(code)
    .fetch_one(pool)
    .await
    .expect("insert contact")
}

async fn insert_info(pool: &PgPool, code: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_info-isahl" (code, notice, dk_scene, dk_factor, dk_function)
           VALUES ($1, $1,
                   (SELECT id FROM isahl.zc_id_scene WHERE code = 'RR' AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_factor WHERE code = 'PFA' AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↓_MA' AND deleted_at IS NULL))
           RETURNING id"#,
    )
    .bind(code)
    .fetch_one(pool)
    .await
    .expect("insert contact info")
}

#[tokio::test]
async fn reverse_and_forward_contact_info_resolution() {
    let _serial = FIXTURE_LOCK.lock().await;
    let pool = connect_test_db().await;
    cleanup(&pool).await;

    let contact_id = insert_contact(&pool, CODES[0]).await;
    let info_id = insert_info(&pool, CODES[1]).await;
    let orphan_info_id = insert_info(&pool, CODES[2]).await;

    // 关联：联系人 ↔ 联系方式（default_info 优先）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_contacts_rr_infos" (ref_left, ref_right, default_info)
           VALUES ($1, $2, TRUE)"#,
    )
    .bind(contact_id)
    .bind(info_id)
    .execute(&pool)
    .await
    .expect("link contact info");

    // 反向：联系方式 → 联系人
    assert_eq!(
        ContactsService::resolve_contact_of_info(&pool, info_id)
            .await
            .expect("reverse resolve"),
        Some(contact_id),
        "已关联的联系方式必须反解到其联系人"
    );
    // 无关联的联系方式 → None（不臆造）
    assert_eq!(
        ContactsService::resolve_contact_of_info(&pool, orphan_info_id)
            .await
            .expect("reverse resolve orphan"),
        None
    );

    // 正向：联系人 → 首选联系方式（与反向构成往返一致）
    assert_eq!(
        ContactsService::resolve_preferred_info(&pool, contact_id)
            .await
            .expect("forward resolve"),
        Some(info_id)
    );

    cleanup(&pool).await;
}

/// 多联系方式择一：`default_info = TRUE` 者胜出（与既有 `resolve_user_contact` 同规则）。
#[tokio::test]
async fn preferred_info_honours_default_flag() {
    let _serial = FIXTURE_LOCK.lock().await;
    let pool = connect_test_db().await;
    cleanup(&pool).await;

    let contact_id = insert_contact(&pool, CODES[0]).await;
    let info_id = insert_info(&pool, CODES[1]).await;
    let other_info_id = insert_info(&pool, CODES[2]).await;

    // 非默认先插、默认后插：择一规则须取 default_info=TRUE 的一条
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_contacts_rr_infos" (ref_left, ref_right, default_info)
           VALUES ($1, $2, FALSE)"#,
    )
    .bind(contact_id)
    .bind(other_info_id)
    .execute(&pool)
    .await
    .expect("link non-default");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_contacts_rr_infos" (ref_left, ref_right, default_info)
           VALUES ($1, $2, TRUE)"#,
    )
    .bind(contact_id)
    .bind(info_id)
    .execute(&pool)
    .await
    .expect("link default");

    assert_eq!(
        ContactsService::resolve_preferred_info(&pool, contact_id)
            .await
            .expect("preferred"),
        Some(info_id),
        "default_info 优先"
    );
    assert_eq!(
        ContactsService::resolve_contact_of_info(&pool, other_info_id)
            .await
            .expect("reverse"),
        Some(contact_id),
        "非默认联系方式同样反解到该联系人"
    );

    cleanup(&pool).await;
}
