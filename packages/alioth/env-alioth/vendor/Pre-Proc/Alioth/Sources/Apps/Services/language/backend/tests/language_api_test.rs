//! alioth-service-language 集成测试
//!
//! 验证 language 因子的 Language 包 CRUD + code 前缀过滤 + settings JSONB 元数据持久化。

use common::testing::{connect_test_db, setup_test_schema_light};
use sqlx::PgPool;

#[tokio::test]
async fn language_crud_via_settings_jsonb() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let uid: i64 = 1;

    // 直接通过 SQL 验证 language handler 的 settings JSONB 行为
    let code = "lang:en-US";
    let meta = serde_json::json!({
        "locale": "en-US",
        "enabled": true,
        "coverage": 85
    });

    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_prot-env_config" (notice, code, settings, created_by_id)
           VALUES ($1, $2, $3::jsonb, $4) RETURNING id"#,
    )
    .bind("English (US)")
    .bind(code)
    .bind(serde_json::to_string(&meta).unwrap_or_default())
    .bind(uid)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 验证 settings::text 读出 JSON 字符串
    let row: (Option<String>, Option<String>) = sqlx::query_as(
        r#"SELECT notice, settings::text FROM isahl."zc_id_prot-env_config"
           WHERE id = $1 AND code LIKE $2"#,
    )
    .bind(id)
    .bind("lang:%")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.0.as_deref(), Some("English (US)"));
    let parsed: serde_json::Value = serde_json::from_str(&row.1.unwrap()).unwrap();
    assert_eq!(parsed["locale"], "en-US");
    assert_eq!(parsed["enabled"], true);
    assert_eq!(parsed["coverage"], 85);

    // 软删除
    let n = sqlx::query(
        r#"UPDATE isahl."zc_id_prot-env_config"
           SET deleted_at = NOW(), updated_by_id = $2
           WHERE id = $1 AND code LIKE 'lang:%' AND deleted_at IS NULL"#,
    )
    .bind(id)
    .bind(uid)
    .execute(&pool)
    .await
    .unwrap()
    .rows_affected();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn language_code_prefix_filters_non_language_rows() {
    let pool: PgPool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let uid: i64 = 1;

    // 插入一个非 lang:* 记录
    let _other: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_prot-env_config" (notice, code, created_by_id)
           VALUES ('other', 'env:aws-prod', $1) RETURNING id"#,
    )
    .bind(uid)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 插入一个 lang:* 记录
    let _lang: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_prot-env_config" (notice, code, created_by_id)
           VALUES ('Chinese', 'lang:zh-CN', $1) RETURNING id"#,
    )
    .bind(uid)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 验证：lang record 在 lang:% 过滤下能找到
    let lang_found: (String,) = sqlx::query_as(
        r#"SELECT notice FROM isahl."zc_id_prot-env_config"
           WHERE id = $1 AND code LIKE $2 AND deleted_at IS NULL"#,
    )
    .bind(_lang)
    .bind("lang:%")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(lang_found.0, "Chinese");

    // 验证：non-lang record 在 lang:% 过滤下找不到
    let nonlang_count: (i64,) = sqlx::query_as(
        r#"SELECT COUNT(*) FROM isahl."zc_id_prot-env_config"
           WHERE id = $1 AND code LIKE $2 AND deleted_at IS NULL"#,
    )
    .bind(_other)
    .bind("lang:%")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(nonlang_count.0, 0, "non-lang record 不应被 lang:% 匹配");
}
