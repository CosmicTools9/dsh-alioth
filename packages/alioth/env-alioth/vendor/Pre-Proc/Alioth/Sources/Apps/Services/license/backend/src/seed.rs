//! 许可证通用种子数据
//!
//! 为 system-settings 模块预置演示许可证与必要的引用字典。

use crate::models::CreateLicenseRequest;
use crate::repositories::LicenseRepository;
use chrono::{Duration, Utc};
use common::error::AliothError;
use crud::AliothRepository;
use sqlx::PgPool;

const SEED_USER_ID: i64 = 1;

#[derive(Debug, Clone)]
struct SeedLicense {
    name: &'static str,
    key: &'static str,
    vendor: &'static str,
    kind: &'static str,
    seats: i64,
    expires_days: i64,
    status: &'static str,
}

fn all_seed_licenses() -> Vec<SeedLicense> {
    vec![
        SeedLicense {
            name: "Alioth Studio 企业版",
            key: "LIC-ALIOTH-ENT-001",
            vendor: "Alioth Studio",
            kind: "订阅",
            seats: 500,
            expires_days: 365,
            status: "healthy",
        },
        SeedLicense {
            name: "Microsoft 365 E5",
            key: "LIC-MS-E5-002",
            vendor: "Microsoft",
            kind: "订阅",
            seats: 1200,
            expires_days: 60,
            status: "warning",
        },
        SeedLicense {
            name: "Adobe Creative Cloud",
            key: "LIC-ADOBE-CC-003",
            vendor: "Adobe",
            kind: "订阅",
            seats: 80,
            expires_days: 7,
            status: "unknown",
        },
        SeedLicense {
            name: "JetBrains All Products Pack",
            key: "LIC-JB-ALL-004",
            vendor: "JetBrains",
            kind: "永久",
            seats: 50,
            expires_days: 9999,
            status: "healthy",
        },
        SeedLicense {
            name: "Figma 专业版",
            key: "LIC-FIGMA-PRO-005",
            vendor: "Figma",
            kind: "试用",
            seats: 30,
            expires_days: 14,
            status: "unknown",
        },
    ]
}

async fn ensure_subject(pool: &PgPool, notice: &str, code: &str) -> Result<i64, AliothError> {
    let id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl.zc_id_subjects WHERE notice = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(notice)
    .fetch_optional(pool)
    .await
    .map_err(AliothError::from)?;
    if let Some(id) = id {
        return Ok(id);
    }
    sqlx::query_scalar(
        "INSERT INTO isahl.zc_id_subjects (notice, code, created_by_id) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(notice)
    .bind(code)
    .bind(SEED_USER_ID)
    .fetch_one(pool)
    .await
    .map_err(AliothError::from)
}

async fn ensure_category(pool: &PgPool, notice: &str, code: &str) -> Result<i64, AliothError> {
    let id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl.zc_id_category WHERE notice = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(notice)
    .fetch_optional(pool)
    .await
    .map_err(AliothError::from)?;
    if let Some(id) = id {
        return Ok(id);
    }
    sqlx::query_scalar(
        "INSERT INTO isahl.zc_id_category (notice, code, created_by_id) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(notice)
    .bind(code)
    .bind(SEED_USER_ID)
    .fetch_one(pool)
    .await
    .map_err(AliothError::from)
}

async fn ensure_status(
    pool: &PgPool,
    notice: &str,
    flag: &str,
    comments: &str,
) -> Result<i64, AliothError> {
    let id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl.zc_id_status WHERE notice = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(notice)
    .fetch_optional(pool)
    .await
    .map_err(AliothError::from)?;
    if let Some(id) = id {
        return Ok(id);
    }
    sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_status (notice, flag, comments, created_by_id)
           VALUES ($1, $2::isahl.status_flag, $3, $4) RETURNING id"#,
    )
    .bind(notice)
    .bind(flag)
    .bind(comments)
    .bind(SEED_USER_ID)
    .fetch_one(pool)
    .await
    .map_err(AliothError::from)
}

/// 向数据库预置通用许可证记录。
///
/// 幂等执行：按 license code 去重；已存在非删除记录则跳过。
/// 会自动创建必要的供应商、类型与状态字典。
pub async fn seed_licenses(pool: &PgPool) -> Result<usize, AliothError> {
    let status_map = [
        ("healthy", ("start", "正常运行/有效")),
        ("warning", ("doing", "需要关注/即将过期")),
        ("unknown", ("end", "未知/已过期")),
    ];

    let mut status_ids = std::collections::HashMap::new();
    for (notice, (flag, comments)) in status_map.iter() {
        let id = ensure_status(pool, notice, flag, comments).await?;
        status_ids.insert(*notice, id);
    }

    let vendors = [
        ("Alioth Studio", "alioth-studio"),
        ("Microsoft", "microsoft"),
        ("Adobe", "adobe"),
        ("JetBrains", "jetbrains"),
        ("Figma", "figma"),
    ];
    let mut vendor_ids = std::collections::HashMap::new();
    for (notice, code) in vendors.iter() {
        let id = ensure_subject(pool, notice, code).await?;
        vendor_ids.insert(*notice, id);
    }

    let kinds = [
        ("订阅", "subscription"),
        ("永久", "perpetual"),
        ("试用", "trial"),
    ];
    let mut kind_ids = std::collections::HashMap::new();
    for (notice, code) in kinds.iter() {
        let id = ensure_category(pool, notice, code).await?;
        kind_ids.insert(*notice, id);
    }

    let repo = LicenseRepository::from(pool.clone());
    let mut inserted = 0usize;

    for lic in all_seed_licenses() {
        let exists: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM isahl."zc_id_prod-license-purchase"
                   WHERE code = $1 AND deleted_at IS NULL
               )"#,
        )
        .bind(lic.key)
        .fetch_one(pool)
        .await
        .map_err(AliothError::from)?;
        if exists {
            continue;
        }

        let vendor_id = vendor_ids.get(lic.vendor).copied();
        let kind_id = kind_ids.get(lic.kind).copied();
        let status_id = status_ids.get(lic.status).copied();

        let expires = Utc::now() + Duration::days(lic.expires_days);

        repo.create(
            CreateLicenseRequest {
                name: lic.name.to_string(),
                key: Some(lic.key.to_string()),
                vendor: vendor_id,
                kind: kind_id,
                vendor_name: None,
                kind_name: None,
                seats: Some(lic.seats),
                expires: Some(expires),
                status: status_id,
            },
            SEED_USER_ID,
        )
        .await?;
        inserted += 1;
    }

    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::testing::{connect_test_db, setup_test_schema_light};

    #[tokio::test]
    async fn seed_licenses_is_idempotent() {
        let pool = connect_test_db().await;
        setup_test_schema_light(&pool).await.unwrap();
        let first = seed_licenses(&pool)
            .await
            .expect("first seed should succeed");
        let second = seed_licenses(&pool)
            .await
            .expect("second seed should succeed");

        assert_eq!(second, 0, "re-seeding should be idempotent");
        assert!(first >= 5, "should seed at least 5 licenses");
    }
}
