//! Requirement Repository — `zc_id_event` CRUD + 类目关联桥接。
//!
//! 读侧（list/get）委托 `GenericRepository::list_refs/get_refs`（含 `_refs` 名称解析）；
//! 写侧（create/update）自定义 INSERT/UPDATE，事务内维护：
//! 1. 主表行（`isahl."zc_id_event"`，dk_* 写 NULL）；
//! 2. 类目关联（`zc_id_lifecycle_r_category` 单值替换：先软删旧行再插入）。
//!
//! `timeline` 列保持事件流语义（本服务不读写）。

use async_trait::async_trait;
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository, ListQuery, PaginatedResponse};
use sqlx::PgPool;

use crate::models::{CreateRequirementRequest, Requirement, UpdateRequirementRequest};

#[derive(Clone)]
pub struct RequirementRepository {
    generic: GenericRepository<Requirement>,
    pool: PgPool,
}

impl RequirementRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 类目单值替换（软删旧关联 → 插入新关联）
    async fn replace_category_in_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: i64,
        category: Option<i64>,
        user_id: i64,
    ) -> Result<(), ApiError> {
        sqlx::query(
            r#"UPDATE isahl.zc_id_lifecycle_r_category
               SET deleted_at = now(), deleted_by_id = $2
               WHERE ref_left = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::from)?;
        if let Some(cat) = category {
            sqlx::query(
                r#"INSERT INTO isahl.zc_id_lifecycle_r_category (ref_left, ref_right, created_by_id)
                   VALUES ($1, $2, $3)"#,
            )
            .bind(id)
            .bind(cat)
            .bind(user_id)
            .execute(&mut **tx)
            .await
            .map_err(ApiError::from)?;
        }
        Ok(())
    }

    /// 按 ID 读取（含 _refs 解析）
    pub async fn get_refs(&self, id: i64) -> Result<Option<Requirement>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    /// 维度选择器：类目（zc_id_category）
    pub async fn list_categories(&self) -> Result<Vec<crate::models::DimensionOption>, ApiError> {
        let rows: Vec<(i64, String)> = sqlx::query_as(
            r#"SELECT id, notice FROM isahl.zc_id_category WHERE deleted_at IS NULL ORDER BY notice"#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(ApiError::from)?;
        Ok(rows
            .into_iter()
            .map(|(id, name)| crate::models::DimensionOption { id, name })
            .collect())
    }

    /// 维度选择器：场所（zc_id_lifecycle 叶表）
    pub async fn list_places(&self) -> Result<Vec<crate::models::DimensionOption>, ApiError> {
        let rows: Vec<(i64, String)> = sqlx::query_as(
            r#"SELECT id, notice FROM isahl.zc_id_lifecycle WHERE deleted_at IS NULL ORDER BY notice"#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(ApiError::from)?;
        Ok(rows
            .into_iter()
            .map(|(id, name)| crate::models::DimensionOption { id, name })
            .collect())
    }
}

#[async_trait]
impl AliothRepository<Requirement, CreateRequirementRequest, UpdateRequirementRequest, ApiError>
    for RequirementRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Requirement>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<Requirement>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateRequirementRequest,
        user_id: i64,
    ) -> Result<Requirement, ApiError> {
        let mut tx = self.pool.begin().await.map_err(ApiError::from)?;

        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_event"
               (notice, code, comments, fk_place, created_by_id,
                dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, NULL, NULL, NULL)
               RETURNING id"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(req.place)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from)?;

        self.replace_category_in_tx(&mut tx, id, req.category, user_id)
            .await?;

        tx.commit().await.map_err(ApiError::from)?;

        self.get_refs(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("requirement {} not found", id)))
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateRequirementRequest,
        user_id: i64,
    ) -> Result<Option<Requirement>, ApiError> {
        if self.get_refs(id).await?.is_none() {
            return Ok(None);
        }

        // 主表字段合并更新（Option.or 保持现值）
        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_event"
               SET notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   fk_place = COALESCE($4, fk_place),
                   updated_by_id = $5
               WHERE id = $6 AND deleted_at IS NULL"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(req.place)
        .bind(user_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(ApiError::from)?;
        if rows.rows_affected() == 0 {
            return Ok(None);
        }

        // 类目关联单值替换（仅在请求显式提供时变更）
        if req.category.is_some() {
            let mut tx = self.pool.begin().await.map_err(ApiError::from)?;
            self.replace_category_in_tx(&mut tx, id, req.category, user_id)
                .await?;
            tx.commit().await.map_err(ApiError::from)?;
        }

        self.get_refs(id).await
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
