//! 回归测试：联系方式默认标记唯一（批注 76ddc317 / 18f2a80a「各种分类的默认只有一个」）。
//!
//! 契约：同一联系人下**每个分类至多一个 `default_info`**——分类判据 = 值行所在叶表
//! （`ContactsService::create_contact_info_direct`：kind 先归一（mobile→telephone 等），
//! 再按 `zc_id_contact_infos.tableoid` 同表判定）。同分类多条默认时**后写入者胜出**，
//! 其他分类的默认不受影响（不跨分类误清）。

use framework_contacts::models::{ContactInfoInput, CreateContactRequest};
use framework_contacts::ContactsService;
use sqlx::PgPool;

async fn test_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://isahl@localhost:5432/aliothstudio_test".to_string());
    let pool = PgPool::connect(&url).await.expect("connect test db");
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("current_database");
    assert!(db.contains("_test"), "REFUSED: non-test db {db}");
    pool
}

fn info(kind: &str, value: &str, is_default: bool) -> ContactInfoInput {
    ContactInfoInput {
        kind: kind.to_string(),
        value: value.to_string(),
        is_default,
    }
}

/// 各分类的默认条数（分类 = 值行所在叶表，与写径判据同源）
async fn defaults_by_kind(pool: &PgPool, contact_id: i64) -> Vec<(String, i64)> {
    sqlx::query_as::<_, (String, i64)>(
        r#"SELECT ct.relname::text, COUNT(*)
             FROM isahl."zc_id_contacts_rr_infos" ri
             JOIN isahl.zc_id_contact_infos ci ON ci.id = ri.ref_right
             JOIN pg_class ct ON ct.oid = ci.tableoid
            WHERE ri.ref_left = $1 AND ri.deleted_at IS NULL AND ri.default_info
            GROUP BY ct.relname ORDER BY ct.relname"#,
    )
    .bind(contact_id)
    .fetch_all(pool)
    .await
    .expect("defaults by kind")
}

/// 该分类下默认值（`None` = 无默认行）
async fn default_value(pool: &PgPool, contact_id: i64, relname: &str) -> Option<String> {
    sqlx::query_scalar(
        r#"SELECT ci.notice
             FROM isahl."zc_id_contacts_rr_infos" ri
             JOIN isahl.zc_id_contact_infos ci ON ci.id = ri.ref_right
             JOIN pg_class ct ON ct.oid = ci.tableoid
            WHERE ri.ref_left = $1 AND ri.deleted_at IS NULL AND ri.default_info
              AND ct.relname::text = $2
            LIMIT 1"#,
    )
    .bind(contact_id)
    .bind(relname)
    .fetch_optional(pool)
    .await
    .expect("default value")
}

#[tokio::test]
async fn default_flag_is_unique_per_kind() {
    let pool = test_pool().await;
    let suffix = std::process::id().to_string();
    // 播种：同分类两条默认（手机 1 先、手机 2 后）+ 另一分类一条默认（邮箱）
    let contact = ContactsService::create_contact(
        &pool,
        CreateContactRequest {
            name: format!("默认唯一性-{suffix}"),
            code: Some(format!("CT-DUP-{suffix}")),
            comments: None,
            infos: vec![
                info("mobile", "138-0000-0001", true),
                info("mobile", "138-0000-0002", true),
                info("email", "dup-default@test.com", true),
            ],
        },
    )
    .await
    .expect("create contact");

    let rows = defaults_by_kind(&pool, contact.id).await;
    assert!(
        rows.iter().all(|(_, n)| *n == 1),
        "每个分类至多一个默认（实测 {rows:?}）"
    );
    assert_eq!(rows.len(), 2, "两个分类各一条默认（实测 {rows:?}）");
    assert_eq!(
        default_value(&pool, contact.id, "zc_id_info-telephone").await,
        Some("138-0000-0002".to_string()),
        "同分类多条默认 → 后写入者胜出（先写入者被清除默认）"
    );
    assert_eq!(
        default_value(&pool, contact.id, "zc_id_info-email").await,
        Some("dup-default@test.com".to_string()),
        "其他分类的默认 MUST NOT 被跨分类清除"
    );

    // 清理本轮播种（父表 DELETE 覆盖继承子表）
    let info_ids: Vec<i64> = sqlx::query_scalar(
        r#"SELECT ref_right FROM isahl."zc_id_contacts_rr_infos" WHERE ref_left = $1"#,
    )
    .bind(contact.id)
    .fetch_all(&pool)
    .await
    .expect("info ids");
    sqlx::query(r#"DELETE FROM isahl."zc_id_contacts_rr_infos" WHERE ref_left = $1"#)
        .bind(contact.id)
        .execute(&pool)
        .await
        .expect("cleanup rr_infos");
    sqlx::query(r#"DELETE FROM isahl.zc_id_contact_infos WHERE id = ANY($1)"#)
        .bind(&info_ids)
        .execute(&pool)
        .await
        .expect("cleanup infos");
    sqlx::query(r#"DELETE FROM isahl.zc_id_contacts WHERE id = $1"#)
        .bind(contact.id)
        .execute(&pool)
        .await
        .expect("cleanup contact");
}
