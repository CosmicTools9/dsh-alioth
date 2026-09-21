//! 运行环境 Repository — 标准 CRUD 实现
//!
//! Repository 持有 PgPool。环境实体映射到 isahl."zc_id_prot-env_config"。

use crate::models;
use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::error::AliothError;
use crud::AliothRepository;
use sqlx::PgPool;

// ── 运行环境 Environment Repository ──────────────────────────────────────────
// ontology 映射：
// - status: zc_id_lifecycle_r_primary-status (ref_left=env.id, ref_right=status.id)
//   -> zc_id_stus-protocol
// - type/services/uptime: settings JSONB
// - comments: 物理列

#[derive(Clone)]
pub struct EnvironmentRepository {
    pool: PgPool,
}

impl EnvironmentRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 环境日志统计 — 按日志类别聚合 `isahl."zc_id_even-log"`。
    /// 语义迁移：模型已删 `level` 列（无替代标量）——按 `ck_category`（zc_id_cate-log
    /// 动作类别 code）聚合；handler 的 info/warn/error 等槽位得不到匹配即为 0（端点保活）。
    pub async fn stats(&self) -> Result<Vec<(String, i64)>, AliothError> {
        sqlx::query_as::<_, (String, i64)>(
            r#"SELECT COALESCE(cc.code, 'uncategorized')::text AS level, COUNT(*)::bigint AS cnt
               FROM isahl."zc_id_even-log" e
               LEFT JOIN isahl."zc_id_cate-log" cc ON cc.id = e.ck_category AND cc.deleted_at IS NULL
               WHERE e.deleted_at IS NULL
               GROUP BY 1"#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AliothError::from)
    }
}

impl From<PgPool> for EnvironmentRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

/// 用于 list/get 的 JOIN 查询字段列表（编译期字面量——站点一律 `concat!` 展开）。
macro_rules! environment_select_fields {
    () => {
        r#"
e.id, e.notice AS name, e.code AS host,
settings->>'os' AS os,
settings->>'runtime' AS runtime,
settings->>'type' AS type_,
rps.ref_right AS status,
(settings->>'services')::int AS services,
settings->>'uptime' AS uptime,
e.comments, e.settings,
rps._refs AS _refs,
e.created_at, e.updated_at, e.deleted_at"#
    };
}
fn merge_env_settings(
    current: Option<&serde_json::Value>,
    os: Option<&str>,
    type_: Option<&str>,
    runtime: Option<&str>,
    services: Option<i32>,
    uptime: Option<&str>,
) -> serde_json::Value {
    let mut base = current
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    if let Some(v) = os {
        base.insert("os".to_string(), serde_json::Value::String(v.to_string()));
    }
    if let Some(v) = type_ {
        base.insert("type".to_string(), serde_json::Value::String(v.to_string()));
    }
    if let Some(v) = runtime {
        base.insert(
            "runtime".to_string(),
            serde_json::Value::String(v.to_string()),
        );
    }
    if let Some(v) = services {
        base.insert(
            "services".to_string(),
            serde_json::Value::Number((v as i64).into()),
        );
    }
    if let Some(v) = uptime {
        base.insert(
            "uptime".to_string(),
            serde_json::Value::String(v.to_string()),
        );
    }
    serde_json::Value::Object(base)
}

#[async_trait]
impl
    AliothRepository<
        models::Environment,
        models::CreateEnvironmentRequest,
        models::UpdateEnvironmentRequest,
        AliothError,
    > for EnvironmentRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<models::Environment>, AliothError> {
        let page = query.page.max(1);
        let page_size = query.page_size.max(1);
        let offset = (page - 1) * page_size;

        let items: Vec<models::Environment> = sqlx::query_as::<_, models::Environment>(concat!(
            r#"SELECT "#,
            environment_select_fields!(),
            r#" FROM isahl."zc_id_prot-env_config" e
               LEFT JOIN LATERAL (
                   SELECT rps.ref_right,
                          jsonb_build_object(
                              'status',
                              (SELECT jsonb_build_object('notice', s.notice, 'code', s.code)
                               FROM isahl.zc_id_status s
                               WHERE s.id = rps.ref_right AND s.deleted_at IS NULL)
                          ) AS _refs
                   FROM isahl."zc_id_lifecycle_r_primary-status" rps
                   WHERE rps.ref_left = e.id AND rps.deleted_at IS NULL
                   LIMIT 1
               ) rps ON true
               WHERE e.deleted_at IS NULL AND e.settings ? 'type'
               ORDER BY e.id DESC LIMIT $1 OFFSET $2"#
        ))
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(AliothError::from)?;

        let count_sql = r#"SELECT COUNT(*) FROM isahl."zc_id_prot-env_config" e WHERE e.deleted_at IS NULL AND e.settings ? 'type'"#;
        let (total,): (i64,) = sqlx::query_as::<_, (i64,)>(count_sql)
            .fetch_one(&self.pool)
            .await
            .map_err(AliothError::from)?;

        Ok(PaginatedResponse {
            items,
            total,
            page,
            page_size,
        })
    }

    async fn get(&self, id: i64) -> Result<Option<models::Environment>, AliothError> {
        sqlx::query_as::<_, models::Environment>(concat!(
            r#"SELECT "#,
            environment_select_fields!(),
            r#" FROM isahl."zc_id_prot-env_config" e
               LEFT JOIN LATERAL (
                   SELECT rps.ref_right,
                          jsonb_build_object(
                              'status',
                              (SELECT jsonb_build_object('notice', s.notice, 'code', s.code)
                               FROM isahl.zc_id_status s
                               WHERE s.id = rps.ref_right AND s.deleted_at IS NULL)
                          ) AS _refs
                   FROM isahl."zc_id_lifecycle_r_primary-status" rps
                   WHERE rps.ref_left = e.id AND rps.deleted_at IS NULL
                   LIMIT 1
               ) rps ON true
               WHERE e.id = $1 AND e.deleted_at IS NULL AND e.settings ? 'type'"#
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn create(
        &self,
        req: models::CreateEnvironmentRequest,
        user_id: i64,
    ) -> Result<models::Environment, AliothError> {
        let mut tx = self.pool.begin().await.map_err(AliothError::from)?;

        let settings = merge_env_settings(
            None,
            req.os.as_deref(),
            req.type_.as_deref(),
            req.runtime.as_deref(),
            req.services,
            req.uptime.as_deref(),
        );

        // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve_conn(&mut *tx, ("JE", "GEC", "↑_DA"))
                .await
                .map_err(AliothError::from)?;
        let env_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_prot-env_config"
               (notice, code, comments, settings, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               RETURNING id"#,
        )
        .bind(&req.name)
        .bind(&req.host)
        .bind(&req.comments)
        .bind(&settings)
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(&mut *tx)
        .await
        .map_err(AliothError::from)?;

        if let Some(status_id) = req.status {
            sqlx::query(
                r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (ref_left, ref_right, created_by_id)
                   VALUES ($1, $2, $3)"#,
            )
            .bind(env_id)
            .bind(status_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
        }

        tx.commit().await.map_err(AliothError::from)?;
        self.get(env_id).await?.ok_or_else(|| {
            AliothError::NotFound(format!("Environment {} not found after create", env_id))
        })
    }

    async fn update(
        &self,
        id: i64,
        req: models::UpdateEnvironmentRequest,
        user_id: i64,
    ) -> Result<Option<models::Environment>, AliothError> {
        let mut tx = self.pool.begin().await.map_err(AliothError::from)?;

        // 读取当前 settings，合并后写回
        let current_settings: Option<serde_json::Value> = sqlx::query_scalar(
            r#"SELECT settings FROM isahl."zc_id_prot-env_config" WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(AliothError::from)?;

        let merged = match &current_settings {
            Some(cur) => merge_env_settings(
                Some(cur),
                req.os.as_deref(),
                req.type_.as_deref(),
                req.runtime.as_deref(),
                req.services,
                req.uptime.as_deref(),
            ),
            None => merge_env_settings(
                None,
                req.os.as_deref(),
                req.type_.as_deref(),
                req.runtime.as_deref(),
                req.services,
                req.uptime.as_deref(),
            ),
        };

        sqlx::query(
            r#"UPDATE isahl."zc_id_prot-env_config"
               SET notice = COALESCE($1, notice), code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   settings = $4,
                   updated_at = NOW(), updated_by_id = $5
               WHERE id = $6 AND deleted_at IS NULL"#,
        )
        .bind(&req.name)
        .bind(&req.host)
        .bind(&req.comments)
        .bind(&merged)
        .bind(user_id)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(AliothError::from)?;

        if let Some(status_id) = req.status {
            // 主状态桥 upsert：ref_left 严格唯一索引不含 deleted_at 谓词（软删行仍占位），
            // 删+插必撞 23505——单语句 ON CONFLICT 换代（publish_test 同款先例）
            sqlx::query(
                r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (ref_left, ref_right, created_by_id)
                   VALUES ($1, $2, $3)
                   ON CONFLICT (ref_left) DO UPDATE SET ref_right = EXCLUDED.ref_right,
                                                        deleted_at = NULL, updated_by_id = $3"#,
            )
            .bind(id)
            .bind(status_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
        }

        tx.commit().await.map_err(AliothError::from)?;
        self.get(id).await
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        let r = sqlx::query(r#"UPDATE isahl."zc_id_prot-env_config" SET deleted_at = NOW(), updated_by_id = $2 WHERE id = $1 AND deleted_at IS NULL"#)
            .bind(id).bind(user_id).execute(&self.pool).await.map_err(AliothError::from)?;
        if r.rows_affected() == 0 {
            return Err(AliothError::NotFound(format!(
                "Environment {} not found",
                id
            )));
        }
        Ok(())
    }

    async fn batch_delete(&self, ids: Vec<i64>, user_id: i64) -> Result<(), AliothError> {
        if ids.is_empty() {
            return Ok(());
        }
        sqlx::query(
            r#"UPDATE isahl."zc_id_prot-env_config" SET deleted_at = NOW(), updated_by_id = $2 WHERE id = ANY($1) AND deleted_at IS NULL"#,
        )
        .bind(&ids)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(AliothError::from)?;
        Ok(())
    }
}
