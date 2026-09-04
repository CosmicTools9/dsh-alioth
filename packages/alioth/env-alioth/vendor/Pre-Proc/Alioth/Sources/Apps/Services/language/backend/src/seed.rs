//! 语言包种子数据 — 预填充常用语言包
//!
//! 幂等执行：以 `lang:<code>` 为键逐条 ensure/update，不依赖全表是否已有数据；
//! 缺失则插入，已存在则规范化 settings（locale/region/enabled/coverage），不会
//! 因部分旧数据跳过全部 4 条。

use common::error::AliothError;
use rust_decimal::Decimal;
use sqlx::PgPool;
use std::str::FromStr;

const SEED_USER_ID: i64 = 1;

const SEED_LANGUAGES: &[(&str, &str, &str, &str)] = &[
    ("简体中文", "zh-CN", "中国大陆", "1.0"),
    ("English", "en", "United States", "1.0"),
    ("日本語", "ja", "日本", "0.72"),
    ("한국어", "ko", "대한민국", "0.0"),
];

pub async fn seed_languages(pool: &PgPool) -> Result<usize, AliothError> {
    let mut updated = 0usize;

    for (name, code, region, coverage_str) in SEED_LANGUAGES {
        let full_code = format!("lang:{}", code);
        let coverage = Decimal::from_str(coverage_str)
            .map_err(|e| AliothError::Internal(format!("invalid coverage value: {}", e)))?;

        let settings = serde_json::json!({
            "locale": code,
            "region": region,
            "enabled": true,
            "coverage": coverage,
        });
        let settings_json = serde_json::to_string(&settings).unwrap_or_default();

        let existing_id: Option<i64> = sqlx::query_scalar(
            r#"SELECT id FROM isahl."zc_id_prot-env_config"
               WHERE code = $1 AND deleted_at IS NULL
               LIMIT 1
               FOR UPDATE"#,
        )
        .bind(&full_code)
        .fetch_optional(pool)
        .await
        .map_err(|e| AliothError::Internal(e.to_string()))?;

        if let Some(id) = existing_id {
            sqlx::query(
                r#"UPDATE isahl."zc_id_prot-env_config"
                   SET notice = $1, settings = $2::jsonb, updated_at = now(), updated_by_id = $3
                   WHERE id = $4"#,
            )
            .bind(name)
            .bind(&settings_json)
            .bind(SEED_USER_ID)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| AliothError::Internal(e.to_string()))?;
            log::info!("[lang-seed] updated: {} ({})", name, code);
        } else {
            sqlx::query(
                r#"INSERT INTO isahl."zc_id_prot-env_config" (notice, code, settings, created_by_id)
                   VALUES ($1, $2, $3::jsonb, $4)"#,
            )
            .bind(name)
            .bind(&full_code)
            .bind(&settings_json)
            .bind(SEED_USER_ID)
            .execute(pool)
            .await
            .map_err(|e| AliothError::Internal(e.to_string()))?;
            log::info!("[lang-seed] inserted: {} ({})", name, code);
        }
        updated += 1;
    }

    log::info!("[lang-seed] ensured {} language packs", updated);
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::testing::{connect_test_db, setup_test_schema_light};

    #[tokio::test]
    async fn seed_languages_idempotent() {
        let pool = connect_test_db().await;
        setup_test_schema_light(&pool).await.unwrap();

        // Clear previous test data (Alioth namespace: no automatic TRUNCATE)
        sqlx::query(r#"DELETE FROM isahl."zc_id_prot-env_config" WHERE code LIKE 'lang:%'"#)
            .execute(&pool)
            .await
            .unwrap();

        let first = seed_languages(&pool).await.unwrap();
        assert_eq!(first, SEED_LANGUAGES.len());

        let second = seed_languages(&pool).await.unwrap();
        assert_eq!(
            second,
            SEED_LANGUAGES.len(),
            "second run should still update all rows"
        );

        let count: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*) FROM isahl."zc_id_prot-env_config" WHERE code LIKE 'lang:%'"#,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, SEED_LANGUAGES.len() as i64);
    }
    #[tokio::test]
    async fn seed_languages_repairs_partial_or_stale_record() {
        let pool = connect_test_db().await;
        setup_test_schema_light(&pool).await.unwrap();

        // Clear previous test data to avoid code conflict with seed lang:zh-CN
        sqlx::query(r#"DELETE FROM isahl."zc_id_prot-env_config" WHERE code LIKE 'lang:%'"#)
            .execute(&pool)
            .await
            .unwrap();

        // Simulate a legacy record where locale is a region name instead of a code.
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_prot-env_config" (notice, code, settings, created_by_id)
               VALUES ('简体中文', 'lang:zh-CN',
                       jsonb_build_object('locale', '中国大陆', 'enabled', true, 'coverage', 1.0),
                       1)"#,
        )
        .execute(&pool)
        .await
        .unwrap();

        let seeded = seed_languages(&pool).await.unwrap();
        assert_eq!(seeded, SEED_LANGUAGES.len());

        let locale: String = sqlx::query_scalar(
            r#"SELECT settings->>'locale' FROM isahl."zc_id_prot-env_config" WHERE code = 'lang:zh-CN'"#,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            locale, "zh-CN",
            "seed should normalize locale to language code"
        );
    }
}
