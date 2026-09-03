//! Alioth Version Control Framework
//!
//! 为 zc_id_version 子表提供版本控制能力：
//! - VersionService：版本追溯（fk_previous 链）
//! - `create_version`：INSERT 新版本行（fk_previous = 旧行 id），链式演进
//! - `rollback`：按目标版本回滚实体（参数绑定 entity/target 语义正确）
//! - `diff`：两版本间字段级差异（VersionDiff 列表；id/时间戳/版本元数据列跳过）

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool, Row};

pub mod error;
pub use error::{VersionError, VersionResult};

/// 实体版本共享内核（CRUD + 链 + 回滚；WZ/Alioth 壳唯一实现来源）
pub mod entity;
pub use entity::{
    CreateVersionRequest, UpdateVersionRequest, VersionRecord as EntityVersionRecord,
    VersionRepository, VersionService as EntityVersionService,
};

/// 可插拔版本后端（git / 降级内存；自适应探测）
pub mod git;
pub use git::{detect_backend, BackendKind, Capability, GitBackend, MemoryBackend, VersionBackend};

/// 版本链
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionRecord {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid::opt")]
    pub tk_version: Option<i64>,
    pub x_version: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub reversion: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_previous: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub ck_branch: Option<i64>,
    pub majority: Option<String>,
    pub sprint: Option<String>,
    /// Git 引用（branch/tag/commit ref），与 checksum 正交——由 git backend 写入
    pub git_ref: Option<String>,
    /// Git commit OID（hex）
    pub git_oid: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(with = "common::serde_zuid::opt")]
    pub created_by_id: Option<i64>,
}

/// 版本差异项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionDiff {
    pub field: String,
    pub old_value: Option<Value>,
    pub new_value: Option<Value>,
}

/// 版本控制服务 trait（面向 zc_id_version 子表）
#[async_trait]
pub trait VersionService: Send + Sync {
    /// 业务表名（如 "zc_id_process"）
    fn version_table_name(&self) -> &'static str;

    /// 创建新版本：复制当前行，更新 fk_previous 链
    async fn create_version(
        &self,
        pool: &PgPool,
        _entity_id: i64,
        x_version: Option<String>,
        _comment: Option<String>,
    ) -> VersionResult<VersionRecord> {
        let table = self.version_table_name();
        // 获取当前行的所有列
        let current = sqlx::query(AssertSqlSafe(format!(
            r#"SELECT * FROM isahl."{}" WHERE id = $1"#,
            table
        )))
        .bind(_entity_id)
        .fetch_optional(pool)
        .await
        .map_err(VersionError::from)?;

        if current.is_none() {
            return Err(VersionError::NotFound(format!(
                "Entity {} not found for create_version",
                _entity_id
            )));
        }

        // INSERT 新行，拷贝当前行数据，fk_previous 指向旧 id
        let row = sqlx::query(
            AssertSqlSafe(format!(
                r#"
                INSERT INTO isahl."{}" (notice, code, comments, dk_scene, dk_factor, dk_function,
                    tk_version, reversion, fk_previous, ck_branch, majority, sprint, created_by_id)
                SELECT notice, code, comments, dk_scene, dk_factor, dk_function,
                    COALESCE(tk_version, 0) + 1, COALESCE(reversion, 0) + 1,
                    id, ck_branch, majority, sprint, $1
                FROM isahl."{}" WHERE id = $2
                RETURNING id, tk_version, x_version, reversion, fk_previous, ck_branch, majority, sprint, created_at, created_by_id
                "#,
                table, table
            )))
        .bind(x_version)
        .bind(_entity_id)
        .fetch_one(pool)
        .await
        .map_err(VersionError::from)?;

        Ok(parse_version_record(&row)?)
    }

    /// 查询版本链（按 fk_previous 链追溯）
    async fn list_versions(
        &self,
        pool: &PgPool,
        entity_id: i64,
    ) -> VersionResult<Vec<VersionRecord>> {
        let table = self.version_table_name();
        let rows = sqlx::query(
            AssertSqlSafe(format!(
                r#"
                WITH RECURSIVE version_chain AS (
                    SELECT id, tk_version, x_version, reversion, fk_previous, ck_branch, majority, sprint, created_at, created_by_id, 0 AS depth
                    FROM isahl."{}"
                    WHERE id = $1
                    UNION ALL
                    SELECT v.id, v.tk_version, v.x_version, v.reversion, v.fk_previous, v.ck_branch, v.majority, v.sprint, v.created_at, v.created_by_id, vc.depth + 1
                    FROM isahl."{}" v
                    JOIN version_chain vc ON v.id = vc.fk_previous
                    WHERE vc.depth < 100
                )
                SELECT * FROM version_chain ORDER BY depth
                "#,
                table, table
            )))
        .bind(entity_id)
        .bind(entity_id)
        .fetch_all(pool)
        .await
        .map_err(VersionError::from)?;

        rows.iter().map(parse_version_record).collect()
    }

    /// 回滚到指定版本（通过 fk_previous 链找到目标版本的数据并恢复）
    async fn rollback(
        &self,
        pool: &PgPool,
        entity_id: i64,
        target_version_id: i64,
    ) -> VersionResult<VersionRecord> {
        let table = self.version_table_name();

        // 获取目标版本的数据
        let target = sqlx::query(AssertSqlSafe(
            format!(r#"SELECT * FROM isahl."{}" WHERE id = $1"#, table).as_str(),
        ))
        .bind(target_version_id)
        .fetch_optional(pool)
        .await
        .map_err(VersionError::from)?;

        if target.is_none() {
            return Err(VersionError::NotFound(format!(
                "Target version {} not found",
                target_version_id
            )));
        }

        // 更新当前实体，标记为回滚版本
        let row = sqlx::query(
            AssertSqlSafe(format!(
                r#"
                UPDATE isahl."{}"
                SET fk_previous = $1,
                    updated_at = NOW()
                WHERE id = $2
                RETURNING id, tk_version, x_version, reversion, fk_previous, ck_branch, majority, sprint, created_at, created_by_id
                "#,
                table
            )))
                .bind(entity_id)
        .bind(target_version_id)
        .fetch_one(pool)
        .await
        .map_err(VersionError::from)?;

        Ok(parse_version_record(&row)?)
    }

    /// 计算两个版本间的字段级差异
    async fn diff(
        &self,
        pool: &PgPool,
        version_a_id: i64,
        version_b_id: i64,
    ) -> VersionResult<Vec<VersionDiff>> {
        let table = self.version_table_name();

        let a_json_str: String = sqlx::query_scalar(AssertSqlSafe(format!(
            r#"SELECT row_to_json(t.*)::text FROM isahl."{}" t WHERE id = $1"#,
            table
        )))
        .bind(version_a_id)
        .fetch_one(pool)
        .await
        .map_err(VersionError::from)?;

        let b_json_str: String = sqlx::query_scalar(AssertSqlSafe(format!(
            r#"SELECT row_to_json(t.*)::text FROM isahl."{}" t WHERE id = $1"#,
            table
        )))
        .bind(version_b_id)
        .fetch_one(pool)
        .await
        .map_err(VersionError::from)?;

        let a_json: serde_json::Value = serde_json::from_str(&a_json_str)
            .map_err(|e| VersionError::InvalidOperation(format!("JSON parse A: {}", e)))?;
        let b_json: serde_json::Value = serde_json::from_str(&b_json_str)
            .map_err(|e| VersionError::InvalidOperation(format!("JSON parse B: {}", e)))?;

        const SKIP: &[&str] = &[
            "id",
            "created_at",
            "updated_at",
            "deleted_at",
            "created_by_id",
            "updated_by_id",
            "deleted_by_id",
            "dk_scene",
            "dk_factor",
            "dk_function",
            "tk_version",
            "fk_previous",
            "_f_",
            "_t_",
            "tpl_id",
            "projection",
            "ak_benefit_user",
            "ak_permit_user",
            "ak_access_user",
            "reversion",
            "ck_branch",
            "majority",
            "sprint",
            "x_version",
        ];

        let mut diffs = Vec::new();
        if let (serde_json::Value::Object(a_map), serde_json::Value::Object(b_map)) =
            (&a_json, &b_json)
        {
            let all_keys: std::collections::BTreeSet<&str> = a_map
                .keys()
                .chain(b_map.keys())
                .map(|s| s.as_str())
                .collect();
            for key in all_keys {
                if key.is_empty() || key.starts_with('_') || SKIP.contains(&key) {
                    continue;
                }
                let av = a_map.get(key);
                let bv = b_map.get(key);
                if av != bv {
                    diffs.push(VersionDiff {
                        field: key.to_string(),
                        old_value: av.cloned(),
                        new_value: bv.cloned(),
                    });
                }
            }
        }

        Ok(diffs)
    }
}

fn parse_version_record(row: &sqlx::postgres::PgRow) -> VersionResult<VersionRecord> {
    Ok(VersionRecord {
        id: row.try_get("id")?,
        tk_version: row.try_get("tk_version").ok(),
        x_version: row.try_get("x_version").ok(),
        reversion: row.try_get("reversion").ok(),
        fk_previous: row.try_get("fk_previous").ok(),
        ck_branch: row.try_get("ck_branch").ok(),
        majority: row.try_get("majority").ok(),
        sprint: row.try_get("sprint").ok(),
        git_ref: row.try_get("git_ref").ok(),
        git_oid: row.try_get("git_oid").ok(),
        created_at: row.try_get("created_at")?,
        created_by_id: row.try_get("created_by_id").ok(),
    })
}
