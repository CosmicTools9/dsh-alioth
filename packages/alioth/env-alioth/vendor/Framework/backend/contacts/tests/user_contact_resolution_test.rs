//! `ContactsService::resolve_user_contact` 集成测试
//!
//! 契约：账号 1:1 绑定实体（`auth_users.entity_id`）→ 默认联系人（`default_contact`）
//! → 首选联系方式（`default_info`）。未绑定（`entity_id IS NULL`）/ 实体无联系人 → None；
//! 联系人无联系方式 → `info_id = None`。
//!
//! fixture 使用负 id（不与真实数据冲突），连接测试库。

use framework_contacts::{ContactsService, UserContactRef};
use sqlx::PgPool;

const USER_BOUND: i64 = -900001;
const USER_UNBOUND: i64 = -900002;
const USER_NO_INFO: i64 = -900003;
const ENTITY_MAIN: i64 = -900011;
const ENTITY_OTHER: i64 = -900012;
const ENTITY_NO_INFO: i64 = -900013;
const CONTACT_MAIN: i64 = -900021;
const CONTACT_ALT: i64 = -900022;
const CONTACT_OTHER: i64 = -900023;
const CONTACT_NO_INFO: i64 = -900024;
const INFO_DEFAULT: i64 = -900031;
const INFO_SECOND: i64 = -900032;
const INFO_OTHER: i64 = -900033;

async fn connect() -> PgPool {
    let url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| common::testing::test_database_url());
    PgPool::connect(&url).await.expect("connect test DB")
}

async fn cleanup(pool: &PgPool) {
    for sql in [
        r#"DELETE FROM isahl."zc_id_contacts_rr_infos" WHERE ref_left IN (-900021, -900022, -900023, -900024)"#,
        r#"DELETE FROM isahl."zc_id_entity_rr_contacts" WHERE ref_left IN (-900011, -900012, -900013)"#,
        r#"DELETE FROM isahl."zc_id_info-telephone" WHERE id IN (-900031, -900032, -900033)"#,
        r#"DELETE FROM isahl.zc_id_contacts WHERE id IN (-900021, -900022, -900023, -900024)"#,
        r#"DELETE FROM isahl."zc_id_empl-natural" WHERE id IN (-900011, -900012, -900013)"#,
        r#"DELETE FROM isahl_auth.auth_users WHERE id IN (-900001, -900002, -900003)"#,
    ] {
        sqlx::query(sql)
            .execute(pool)
            .await
            .expect("cleanup fixture");
    }
}

/// 建链：账号(1:1 entity_id) → 实体 → 联系人(default_contact 区分) → 联系方式(default_info 区分)
async fn seed(pool: &PgPool) {
    // 账号：绑定 / 未绑定 / 绑定到无联系方式实体
    for (user_id, entity_id) in [
        (USER_BOUND, Some(ENTITY_MAIN)),
        (USER_UNBOUND, None),
        (USER_NO_INFO, Some(ENTITY_NO_INFO)),
    ] {
        sqlx::query(
            r#"INSERT INTO isahl_auth.auth_users (id, name, username, entity_table, entity_id)
               VALUES ($1, $2, $2, 'zc_id_empl-natural', $3)"#,
        )
        .bind(user_id)
        .bind(format!("contact-resolution-{user_id}"))
        .bind(entity_id)
        .execute(pool)
        .await
        .expect("insert auth user");
    }

    for (entity_id, label) in [
        (ENTITY_MAIN, "绑定实体"),
        (ENTITY_OTHER, "他方实体"),
        (ENTITY_NO_INFO, "无联系方式实体"),
    ] {
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_empl-natural" (id, notice, code, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3,
                       (SELECT id FROM isahl.zc_id_scene LIMIT 1),
                       (SELECT id FROM isahl.zc_id_factor LIMIT 1),
                       (SELECT id FROM isahl.zc_id_function LIMIT 1))"#,
        )
        .bind(entity_id)
        .bind(label)
        .bind(format!("T-CONTACT-RES-{entity_id}"))
        .execute(pool)
        .await
        .expect("insert entity");
    }

    for (contact_id, label) in [
        (CONTACT_MAIN, "默认联系人"),
        (CONTACT_ALT, "非默认联系人"),
        (CONTACT_OTHER, "他方联系人"),
        (CONTACT_NO_INFO, "无联系方式联系人"),
    ] {
        sqlx::query(
            r#"INSERT INTO isahl.zc_id_contacts (id, notice, code, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, 0, 0, 0)"#,
        )
        .bind(contact_id)
        .bind(label)
        .bind(format!("T-CONTACT-RES-{contact_id}"))
        .execute(pool)
        .await
        .expect("insert contact");
    }

    // 实体↔联系人：ENTITY_MAIN 有默认 + 非默认两条（非默认 id 更小——判据必须来自 default_contact）
    for (entity_id, contact_id, is_default) in [
        (ENTITY_MAIN, CONTACT_MAIN, true),
        (ENTITY_MAIN, CONTACT_ALT, false),
        (ENTITY_OTHER, CONTACT_OTHER, true),
        (ENTITY_NO_INFO, CONTACT_NO_INFO, true),
    ] {
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_entity_rr_contacts" (ref_left, ref_right, default_contact)
               VALUES ($1, $2, $3)"#,
        )
        .bind(entity_id)
        .bind(contact_id)
        .bind(is_default)
        .execute(pool)
        .await
        .expect("insert entity contact link");
    }

    for (info_id, notice) in [
        (INFO_DEFAULT, "13800000001"),
        (INFO_SECOND, "13800000002"),
        (INFO_OTHER, "13800000003"),
    ] {
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_info-telephone" (id, notice, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2,
                       (SELECT id FROM isahl.zc_id_scene LIMIT 1),
                       (SELECT id FROM isahl.zc_id_factor LIMIT 1),
                       (SELECT id FROM isahl.zc_id_function LIMIT 1))"#,
        )
            .bind(info_id)
            .bind(notice)
            .execute(pool)
            .await
            .expect("insert contact info");
    }

    // 联系人↔联系方式：默认 + 非默认各一（非默认 id 同样更小）
    for (contact_id, info_id, is_default) in [
        (CONTACT_MAIN, INFO_DEFAULT, true),
        (CONTACT_MAIN, INFO_SECOND, false),
        (CONTACT_OTHER, INFO_OTHER, true),
    ] {
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_contacts_rr_infos" (ref_left, ref_right, default_info)
               VALUES ($1, $2, $3)"#,
        )
        .bind(contact_id)
        .bind(info_id)
        .bind(is_default)
        .execute(pool)
        .await
        .expect("insert contact info link");
    }
}

/// 绑定账号解析出「默认联系人 + 首选联系方式」；不串到他方实体；未绑定 → None。
#[tokio::test]
async fn resolve_bound_user_contact_prefers_defaults() {
    let pool = connect().await;
    cleanup(&pool).await;
    seed(&pool).await;

    let bound = ContactsService::resolve_user_contact(&pool, USER_BOUND)
        .await
        .expect("resolve bound user");
    assert_eq!(
        bound,
        Some(UserContactRef {
            contact_id: CONTACT_MAIN,
            info_id: Some(INFO_DEFAULT),
        }),
        "1:1 绑定账号应解析到 default_contact 联系人及其 default_info 联系方式"
    );

    let unbound = ContactsService::resolve_user_contact(&pool, USER_UNBOUND)
        .await
        .expect("resolve unbound user");
    assert_eq!(unbound, None, "未绑定账号（entity_id IS NULL）→ 解析不出");

    let no_info = ContactsService::resolve_user_contact(&pool, USER_NO_INFO)
        .await
        .expect("resolve user without contact info");
    assert_eq!(
        no_info,
        Some(UserContactRef {
            contact_id: CONTACT_NO_INFO,
            info_id: None,
        }),
        "联系人无联系方式 → 联系人有值、info_id 为空"
    );

    cleanup(&pool).await;
}
