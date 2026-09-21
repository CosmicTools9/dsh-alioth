//! 版本通用种子数据
//!
//! 为 `isahl.zc_id_version` 预置演示版本链。
//! 幂等执行：按 (tpl_id, version_number) 去重，已存在非删除记录则跳过。

use chrono::Utc;
use common::error::AliothError;
use sqlx::PgPool;

const SEED_USER_ID: i64 = 1;

#[derive(Debug, Clone)]
struct SeedVersion {
    tpl_id: i64,
    version_number: i64,
}

fn all_seed_versions() -> Vec<SeedVersion> {
    vec![
        // 模板/实体 1 的版本链
        SeedVersion {
            tpl_id: 1,
            version_number: 1,
        },
        SeedVersion {
            tpl_id: 1,
            version_number: 2,
        },
        SeedVersion {
            tpl_id: 1,
            version_number: 3,
        },
        // 模板/实体 2 的版本链
        SeedVersion {
            tpl_id: 2,
            version_number: 1,
        },
        SeedVersion {
            tpl_id: 2,
            version_number: 2,
        },
    ]
}

/// 向数据库预置通用版本链记录。
///
/// 幂等：同一 `tpl_id` + `version_number` 已存在非删除记录则跳过。
/// 按 version_number 升序插入并自动维护 `fk_previous` 链。
pub async fn seed_versions(pool: &PgPool) -> Result<usize, AliothError> {
    let mut inserted = 0usize;
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID（循环外解析一次）
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("JE", "GEB", "↑_DA"))
        .await
        .map_err(AliothError::from)?;

    for v in all_seed_versions() {
        let exists: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM isahl.zc_id_version
                   WHERE tpl_id = $1
                     AND tk_version = $2
                     AND deleted_at IS NULL
               )"#,
        )
        .bind(v.tpl_id)
        .bind(v.version_number)
        .fetch_one(pool)
        .await
        .map_err(AliothError::from)?;
        if exists {
            continue;
        }

        // 新记录 fk_previous 为 NULL，成为新的链头。
        let new_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_bom-file"
               (tpl_id, tk_version, created_by_id, created_at, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               RETURNING id"#,
        )
        .bind(v.tpl_id)
        .bind(v.version_number)
        .bind(SEED_USER_ID)
        .bind(Utc::now())
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(pool)
        .await
        .map_err(AliothError::from)?;

        // 旧链头（同一 tpl_id，fk_previous IS NULL，id != new_id）指向新记录。
        sqlx::query(
            r#"UPDATE isahl.zc_id_version
               SET fk_previous = $1, updated_at = NOW()
               WHERE tpl_id = $2
                 AND id != $1
                 AND fk_previous IS NULL
                 AND deleted_at IS NULL"#,
        )
        .bind(new_id)
        .bind(v.tpl_id)
        .execute(pool)
        .await
        .map_err(AliothError::from)?;

        inserted += 1;
    }

    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::testing::{connect_test_db, setup_test_schema_light};

    /// 清场 zc_id_version：动态收集全部入向外键（含继承子表）的引用集，只删未被引用行。
    /// 硬编码清单必腐（bill_check_ext 等扩展表随业务演进持续新增）。
    async fn clean_version_table(pool: &sqlx::PgPool) {
        sqlx::query(
            r#"DO $$
DECLARE r record;
BEGIN
    CREATE TEMP TABLE IF NOT EXISTS _keep_version_ids(id bigint PRIMARY KEY);
    TRUNCATE _keep_version_ids;
    FOR r IN
        SELECT c.conrelid::regclass::text AS tbl, a.attname AS col
        FROM pg_constraint c
        JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = ANY (c.conkey)
        WHERE c.contype = 'f' AND c.confrelid IN (
            SELECT t.oid FROM pg_class t
            WHERE t.oid = 'isahl.zc_id_version'::regclass
               OR t.oid IN (
                   WITH RECURSIVE inh AS (
                       SELECT inhrelid FROM pg_inherits
                       WHERE inhparent = 'isahl.zc_id_version'::regclass
                       UNION ALL
                       SELECT i.inhrelid FROM pg_inherits i JOIN inh ON i.inhparent = inh.inhrelid
                   )
                   SELECT inhrelid FROM inh
               )
        )
    LOOP
        EXECUTE format(
            'INSERT INTO _keep_version_ids SELECT DISTINCT s.%I FROM %s s WHERE s.%I IS NOT NULL ON CONFLICT DO NOTHING',
            r.col, r.tbl, r.col
        );
    END LOOP;
    -- ONLY：本表自持行；族内后代表（law/stan-* 等 384 表）不受影响
    DELETE FROM ONLY isahl.zc_id_version t WHERE NOT EXISTS (SELECT 1 FROM _keep_version_ids k WHERE k.id = t.id);
    DROP TABLE _keep_version_ids;
END $$"#,
        )
        .execute(pool)
        .await
        .expect("clean version table");
    }

    #[tokio::test]
    async fn seed_versions_is_idempotent() {
        let pool = connect_test_db().await;
        setup_test_schema_light(&pool).await.unwrap();
        // 清理版本表（测试库共享，幂等断言依赖空起点）——入向 FK 随业务演进新增
        // （实测 wz_fssc.bill_check_ext 引用），动态收集全部入向引用集后只删未引用行
        clean_version_table(&pool).await;
        // 精确清本种子自有行（叶表 zc_id_bom-file）：不动族内模型级行
        sqlx::query(
            r#"DELETE FROM isahl."zc_id_bom-file"
               WHERE (tpl_id, tk_version) IN ((1,1), (1,2), (1,3), (2,1), (2,2))"#,
        )
        .execute(&pool)
        .await
        .expect("clear seed rows");
        let _first = seed_versions(&pool)
            .await
            .expect("first seed should succeed");
        let second = seed_versions(&pool)
            .await
            .expect("second seed should succeed");

        // 共享测试库可能已有同 (tpl_id, tk_version) 行（幂等即跳过），故断言包含式：
        // 5 组种子版本全部在册 + 二次调用零新增
        assert_eq!(second, 0, "re-seeding should be idempotent");
        let seeded: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*) FROM isahl."zc_id_bom-file"
               WHERE deleted_at IS NULL
                 AND (tpl_id, tk_version) IN ((1,1), (1,2), (1,3), (2,1), (2,2))"#,
        )
        .fetch_one(&pool)
        .await
        .expect("count seeded versions");
        assert_eq!(seeded, 5, "种子 5 组版本应全部在册");
    }
}
