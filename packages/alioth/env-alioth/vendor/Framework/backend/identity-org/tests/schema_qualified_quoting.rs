//! 回归测试：schema 限定表名的引号书写（`isahl."table"` 而非 `"isahl.table"`）
//!
//! 背景：`repository/{license,environment}.rs` 的 UPDATE 曾写作 `UPDATE "isahl.zc_id_..."`
//! —— PostgreSQL 中引号内是**单个**标识符，语句必然报「关系 "isahl.zc_id_..." 不存在」。
//! 本测试走 Repository 的 create + update 真实路径（修复前 update 两条用例必失败）。
//!
//! 判据来源：openspec/specs/sql-schema-qualified-naming/spec.md
//! 依赖：test 库存在 `isahl."zc_id_prot-env_config"` / `isahl."zc_id_prod-license-purchase"`
//! 及 dk 坐标解析所需的 isahl_meta 维度行（样本行经 `create` 生产路径建立，坐标由
//! `ontology_binding::resolve` 注入，不手写 ZUID）。

use crud::repository::AliothRepository as _;
use identity_org::models::*;
use identity_org::repository::*;
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

#[tokio::test]
async fn environment_update_persists_name() {
    let pool = test_pool().await;
    let repo = EnvironmentRepository::new(pool.clone());

    let created = repo
        .create(
            CreateEnvironmentRequest {
                name: "回归-环境".into(),
            },
            1,
        )
        .await
        .expect("创建环境配置样本（生产路径，dk 坐标已注入）");

    let updated = repo
        .update(
            created.id,
            UpdateEnvironmentRequest {
                name: Some("回归-环境-改".into()),
            },
            1,
        )
        .await
        .expect("环境配置更新（修复前：关系不存在）")
        .expect("返回更新后的行");
    assert_eq!(updated.name, "回归-环境-改");

    let read_back: String =
        sqlx::query_scalar(r#"SELECT notice FROM isahl."zc_id_prot-env_config" WHERE id = $1"#)
            .bind(created.id)
            .fetch_one(&pool)
            .await
            .expect("读回");
    assert_eq!(read_back, "回归-环境-改", "更新已落库");

    sqlx::query(r#"DELETE FROM isahl."zc_id_prot-env_config" WHERE id = $1"#)
        .bind(created.id)
        .execute(&pool)
        .await
        .expect("清理样本行");
}

#[tokio::test]
async fn license_update_persists_name_and_quantity() {
    let pool = test_pool().await;
    let repo = LicenseRepository::new(pool.clone());

    let created = repo
        .create(
            CreateLicenseRequest {
                name: "回归-许可证".into(),
                qk_qty: Some(3),
                qk_period: None,
            },
            1,
        )
        .await
        .expect("创建许可证样本（生产路径，dk 坐标已注入）");

    let updated = repo
        .update(
            created.id,
            UpdateLicenseRequest {
                name: Some("回归-许可证-改".into()),
                qk_qty: Some(7),
                qk_period: None,
            },
            1,
        )
        .await
        .expect("许可证更新（修复前：关系不存在）")
        .expect("返回更新后的行");
    assert_eq!(updated.name, "回归-许可证-改");
    assert_eq!(updated.qk_qty, Some(7));

    sqlx::query(r#"DELETE FROM isahl."zc_id_prod-license-purchase" WHERE id = $1"#)
        .bind(created.id)
        .execute(&pool)
        .await
        .expect("清理样本行");
}
