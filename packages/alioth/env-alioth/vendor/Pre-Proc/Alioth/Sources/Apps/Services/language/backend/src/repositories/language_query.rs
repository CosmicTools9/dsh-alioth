//! 语言 Repository — 通过 code LIKE 'lang:%' 查询 isahl.zc_id_prot-env_config
//!
//! 元数据（locale/enabled/coverage）存储在 settings JSONB 列，
//! SELECT 时以 settings AS notice 返回，兼容前端 toFrontend() 解析。
//! Handler 层数据访问统一收敛于此（F7 分层）。

use common::error::AliothError;
use sqlx::{AssertSqlSafe, PgPool};

use crate::models::Language;

/// 语言包 code 前缀 — 行归属过滤（本服务只操作 lang:* 行）
pub const PREFIX: &str = "lang:";

/// 列表/单条共用的 SELECT 列（settings JSONB 展开为顶层字段）。
const SELECT_COLUMNS: &str = r#"SELECT id, notice AS name, code,
       settings->>'locale' AS locale,
       NULLIF(settings->>'enabled','')::boolean AS enabled,
       NULLIF(settings->>'coverage','')::numeric AS coverage,
       created_at, updated_at, deleted_at
FROM isahl."zc_id_prot-env_config""#;

/// 语言仓储
#[derive(Clone)]
pub struct LanguageRepository {
    pool: PgPool,
}

impl LanguageRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 列表 — 支持可见 ID 过滤（`parse_visible_ids` 授权范围）
    ///
    /// `visible` 语义：
    /// - `None`：无 `visible` header → 不过滤
    /// - `Some(空集)`：显式无授权（`none` header）→ 恒假谓词零行
    /// - `Some(非空)`：`id = ANY($2)`
    pub async fn list(&self, visible: Option<&[i64]>) -> Result<Vec<Language>, AliothError> {
        let mut items_sql = format!(
            "{} WHERE code LIKE $1 AND deleted_at IS NULL",
            SELECT_COLUMNS
        );
        if let Some(ids) = visible {
            if ids.is_empty() {
                items_sql.push_str(" AND false");
            } else {
                items_sql.push_str(" AND id = ANY($2)");
            }
        }
        items_sql.push_str(" ORDER BY id");

        let mut qb = sqlx::query_as::<_, Language>(AssertSqlSafe(items_sql.as_str()))
            .bind(format!("{}%", PREFIX));
        if let Some(ids) = visible {
            if !ids.is_empty() {
                qb = qb.bind(ids);
            }
        }
        qb.fetch_all(&self.pool).await.map_err(AliothError::from)
    }

    /// 单条 — 命中返回 `Some`，否则 `None`
    pub async fn get(&self, id: i64) -> Result<Option<Language>, AliothError> {
        let sql = format!(
            "{} WHERE id = $1 AND code LIKE $2 AND deleted_at IS NULL",
            SELECT_COLUMNS
        );
        sqlx::query_as::<_, Language>(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .bind(format!("{}%", PREFIX))
            .fetch_optional(&self.pool)
            .await
            .map_err(AliothError::from)
    }

    /// 创建语言包 — INSERT 后回读实体
    pub async fn create(
        &self,
        name: &str,
        code: &str,
        meta: &serde_json::Value,
        user_id: i64,
    ) -> Result<Language, AliothError> {
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_prot-env_config"" (notice, code, settings, created_by_id)
               VALUES ($1, $2, $3::jsonb, $4) RETURNING id"#,
        )
        .bind(name)
        .bind(code)
        .bind(serde_json::to_string(meta).unwrap_or_default())
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(AliothError::from)?;

        let sql = format!("{} WHERE id = $1", SELECT_COLUMNS);
        sqlx::query_as::<_, Language>(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(AliothError::from)
    }

    /// 更新语言包 — UPDATE（settings 合并）后回读实体；未命中返回 `None`
    pub async fn update(
        &self,
        id: i64,
        name: Option<&str>,
        meta: &serde_json::Value,
        user_id: i64,
    ) -> Result<Option<Language>, AliothError> {
        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_prot-env_config""
               SET notice = COALESCE($1, notice),
                   settings = COALESCE(settings, '{}'::jsonb) || $2::jsonb,
                   updated_at = NOW(), updated_by_id = $3
               WHERE id = $4 AND code LIKE $5 AND deleted_at IS NULL"#,
        )
        .bind(name)
        .bind(serde_json::to_string(meta).unwrap_or_default())
        .bind(user_id)
        .bind(id)
        .bind(format!("{}%", PREFIX))
        .execute(&self.pool)
        .await
        .map_err(AliothError::from)?;

        if rows.rows_affected() == 0 {
            return Ok(None);
        }

        let sql = format!("{} WHERE id = $1", SELECT_COLUMNS);
        let item = sqlx::query_as::<_, Language>(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(AliothError::from)?;
        Ok(Some(item))
    }

    /// 软删除语言包 — 命中返回 `true`，否则 `false`
    pub async fn delete(&self, id: i64, user_id: i64) -> Result<bool, AliothError> {
        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_prot-env_config""
               SET deleted_at = NOW(), updated_by_id = $2
               WHERE id = $1 AND code LIKE $3 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .bind(format!("{}%", PREFIX))
        .execute(&self.pool)
        .await
        .map_err(AliothError::from)?;
        Ok(rows.rows_affected() > 0)
    }
}
