//! contacts service 集成测试
//!
//! 验证 B1 修复：同一联系人同时含 email + phone + im 时，
//! _refs SQL 能返回全部三类信息（不会有 DISTINCT ON 互斥丢失）。
//! 验证 default_info 排序优先。
//!
//! 连接到 aliothstudio_test 库，创建 fixture 后读取验证。

use framework_contacts::ContactsService;
use sqlx::PgPool;

async fn connect() -> PgPool {
    let url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| common::testing::test_database_url());
    PgPool::connect(&url).await.expect("connect test DB")
}

/// 插入 fixture 联系人数据并返回其 ID
async fn insert_fixture(pool: &PgPool) -> i64 {
    // 清理旧 fixture
    sqlx::query("DELETE FROM isahl.\"zc_id_contacts_rr_infos\" WHERE ref_left = -1")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl.\"zc_id_info-email\" WHERE id IN (-10, -11)")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl.\"zc_id_info-telephone\" WHERE id IN (-20, -21)")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl.\"zc_id_info-im\" WHERE id = -30")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl.zc_id_contacts WHERE id = -1")
        .execute(pool)
        .await
        .ok();

    // 创建联系人
    sqlx::query(
        "INSERT INTO isahl.zc_id_contacts (id, notice, dk_scene, dk_factor, dk_function) \
         VALUES (-1, '测试联系人', 0, 0, 0)",
    )
    .execute(pool)
    .await
    .unwrap();

    // 创建 email info: 2 条，第一条 default
    sqlx::query(
        "INSERT INTO isahl.\"zc_id_info-email\" (id, notice) VALUES (-10, 'primary@test.com')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO isahl.\"zc_id_info-email\" (id, notice) VALUES (-11, 'secondary@test.com')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO isahl.\"zc_id_contacts_rr_infos\" (id, ref_left, ref_right, default_info) \
         VALUES (-100, -1, -10, true)",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO isahl.\"zc_id_contacts_rr_infos\" (id, ref_left, ref_right, default_info) \
         VALUES (-101, -1, -11, false)",
    )
    .execute(pool)
    .await
    .unwrap();

    // 创建 telephone info: 2 条，第一条 default
    sqlx::query(
        "INSERT INTO isahl.\"zc_id_info-telephone\" (id, notice) VALUES (-20, '138-0000-0001')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO isahl.\"zc_id_info-telephone\" (id, notice) VALUES (-21, '138-0000-0002')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO isahl.\"zc_id_contacts_rr_infos\" (id, ref_left, ref_right, default_info) \
         VALUES (-102, -1, -20, true)",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO isahl.\"zc_id_contacts_rr_infos\" (id, ref_left, ref_right, default_info) \
         VALUES (-103, -1, -21, false)",
    )
    .execute(pool)
    .await
    .unwrap();

    // 创建 im info: 1 条，非 default
    sqlx::query("INSERT INTO isahl.\"zc_id_info-im\" (id, notice) VALUES (-30, 'wechat_test')")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO isahl.\"zc_id_contacts_rr_infos\" (id, ref_left, ref_right, default_info) \
         VALUES (-104, -1, -30, false)",
    )
    .execute(pool)
    .await
    .unwrap();

    -1
}

/// 清理 fixture
async fn cleanup_fixture(pool: &PgPool) {
    sqlx::query("DELETE FROM isahl.\"zc_id_contacts_rr_infos\" WHERE ref_left = -1")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl.\"zc_id_info-email\" WHERE id IN (-10, -11)")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl.\"zc_id_info-telephone\" WHERE id IN (-20, -21)")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl.\"zc_id_info-im\" WHERE id = -30")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl.zc_id_contacts WHERE id = -1")
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_b1_fix_infos_include_all_types() {
    let pool = connect().await;
    insert_fixture(&pool).await;

    let (contacts, total) = ContactsService::list_contacts(&pool, 1, 10)
        .await
        .expect("list contacts should succeed");

    // 找到 fixture 联系人
    let contact = contacts
        .iter()
        .find(|c| c.id == -1)
        .expect("fixture contact should be in results");

    assert!(total >= 1, "should have at least fixture contact");

    // B1 修复验证：infos 同时包含 email + phone + im
    let emails: Vec<_> = contact.infos.iter().filter(|i| i.kind == "email").collect();
    let phones: Vec<_> = contact.infos.iter().filter(|i| i.kind == "phone").collect();
    let ims: Vec<_> = contact.infos.iter().filter(|i| i.kind == "im").collect();

    assert!(!emails.is_empty(), "should have email infos");
    assert!(!phones.is_empty(), "should have phone infos");
    assert!(!ims.is_empty(), "should have im infos");

    // 验证多方共存（B1 核心：不再互斥）
    eprintln!(
        "B1 OK: contact {} has {} email(s), {} phone(s), {} im(s)",
        contact.id,
        emails.len(),
        phones.len(),
        ims.len()
    );

    cleanup_fixture(&pool).await;
}

#[tokio::test]
async fn test_default_info_respected() {
    let pool = connect().await;
    insert_fixture(&pool).await;

    let (contacts, _) = ContactsService::list_contacts(&pool, 1, 50)
        .await
        .expect("list contacts");

    let contact = contacts
        .iter()
        .find(|c| c.id == -1)
        .expect("fixture contact should be in results");

    // default email: 第一条 (primary@test.com, default_info=true)
    let email_default = contact
        .infos
        .iter()
        .find(|i| i.kind == "email" && i.is_default)
        .map(|i| i.value.as_str());
    assert_eq!(
        email_default,
        Some("primary@test.com"),
        "first email should be marked default and listed first"
    );

    // default phone: 第一条 (138-0000-0001, default_info=true)
    let phone_default = contact
        .infos
        .iter()
        .find(|i| i.kind == "phone" && i.is_default)
        .map(|i| i.value.as_str());
    assert_eq!(
        phone_default,
        Some("138-0000-0001"),
        "first phone should be marked default"
    );

    // 便捷字段 email = default email
    assert_eq!(contact.email.as_deref(), Some("primary@test.com"));

    // 便捷字段 phone = default phone
    assert_eq!(contact.phone.as_deref(), Some("138-0000-0001"));

    cleanup_fixture(&pool).await;
}

#[tokio::test]
async fn test_pagination_works() {
    let pool = connect().await;

    let (page1, total) = ContactsService::list_contacts(&pool, 1, 10)
        .await
        .expect("page 1");
    assert!(page1.len() <= 10, "page1 should fit page_size");

    if total > 10 {
        let (page2, _) = ContactsService::list_contacts(&pool, 2, 10)
            .await
            .expect("page 2");
        if !page1.is_empty() && !page2.is_empty() {
            assert_ne!(page1[0].id, page2[0].id, "pages should not overlap");
        }
    }
}
