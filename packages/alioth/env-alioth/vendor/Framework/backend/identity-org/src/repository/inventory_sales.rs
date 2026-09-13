// InventorySalesRepository（split 自 repository.rs 单体，④ 候选）
//! 身份实体 Repository — 标准 CRUD 实现
//!
//! Identity 使用自定义 Repository，其余实体组合 GenericRepository，
//! 仅自定义 create/update 的 INSERT/UPDATE SQL。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::repository::AliothRepository;
use crud::GenericRepository;
use sqlx::{AssertSqlSafe, PgPool};

use crate::models::{CreateInventorySalesRequest, InventorySales, UpdateInventorySalesRequest};

// ═══════════════════════════════════════════════
// InventorySales Repository — zc_id_production_rr_storage（库存统计关系）
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct InventorySalesRepository {
    generic: GenericRepository<InventorySales>,
    pool: PgPool,
}

impl InventorySalesRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 校验 ref_left：必须是 zc_id_production 的一个有效成员
    async fn validate_ref_left(&self, fk: Option<i64>) -> Result<(), ApiError> {
        if let Some(prod_id) = fk {
            let exists: (bool,) = sqlx::query_as(
                r#"SELECT EXISTS(SELECT 1 FROM isahl.zc_id_production WHERE id = $1 AND deleted_at IS NULL)"#
            )
            .bind(prod_id)
            .fetch_one(&self.pool)
            .await
            .map_err(ApiError::from)?;

            if !exists.0 {
                return Err(ApiError::Validation {
                    field: "ref_left".into(),
                    message: format!(
                        "无效的 production 引用: ref_left={} 不存在或已被删除",
                        prod_id
                    ),
                });
            }
        }
        Ok(())
    }
}

impl From<PgPool> for InventorySalesRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        InventorySales,
        CreateInventorySalesRequest,
        UpdateInventorySalesRequest,
        ApiError,
    > for InventorySalesRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<InventorySales>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<InventorySales>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateInventorySalesRequest,
        user_id: i64,
    ) -> Result<InventorySales, ApiError> {
        // 校验 ref_left 必须指向 zc_id_production 的一个有效成员
        self.validate_ref_left(req.ref_left).await?;

        sqlx::query_as::<_, InventorySales>(
            r#"INSERT INTO "isahl"."zc_id_file_rr_url" (code, notice, comments, ref_left, ref_right, qk_p_capacity, qk_qty, sk_unit, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING id, code, notice, comments, ref_left, ref_right, qk_p_capacity, qk_qty, sk_unit, created_at, updated_at, deleted_at"#,
        )
        .bind(req.code)
        .bind(req.notice)
        .bind(req.comments)
        .bind(req.ref_left)
        .bind(req.ref_right)
        .bind(req.qk_p_capacity)
        .bind(req.qk_qty)
        .bind(req.sk_unit)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateInventorySalesRequest,
        user_id: i64,
    ) -> Result<Option<InventorySales>, ApiError> {
        // 校验 ref_left 必须指向 zc_id_production 的一个有效成员
        self.validate_ref_left(req.ref_left).await?;

        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if let Some(ref _v) = req.code {
            idx += 1;
            sets.push(format!("code = ${}", idx));
        }
        if let Some(ref _v) = req.notice {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
        if let Some(ref _v) = req.comments {
            idx += 1;
            sets.push(format!("comments = ${}", idx));
        }
        if let Some(_v) = &req.ref_left {
            idx += 1;
            sets.push(format!("ref_left = ${}", idx));
        }
        if let Some(_v) = &req.ref_right {
            idx += 1;
            sets.push(format!("ref_right = ${}", idx));
        }
        if let Some(_v) = &req.qk_p_capacity {
            idx += 1;
            sets.push(format!("qk_p_capacity = ${}", idx));
        }
        if let Some(_v) = &req.qk_qty {
            idx += 1;
            sets.push(format!("qk_qty = ${}", idx));
        }
        if let Some(_v) = &req.sk_unit {
            idx += 1;
            sets.push(format!("sk_unit = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;
        let sql = format!(
            r#"UPDATE "isahl"."zc_id_production_rr_storage" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, ref_left, ref_right, qk_p_capacity, qk_qty, sk_unit, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );
        let mut q = sqlx::query_as::<_, InventorySales>(AssertSqlSafe(sql.as_str()));
        if let Some(v) = &req.code {
            q = q.bind(v);
        }
        if let Some(v) = &req.notice {
            q = q.bind(v);
        }
        if let Some(v) = &req.comments {
            q = q.bind(v);
        }
        if let Some(v) = &req.ref_left {
            q = q.bind(v);
        }
        if let Some(v) = &req.ref_right {
            q = q.bind(v);
        }
        if let Some(v) = &req.qk_p_capacity {
            q = q.bind(v);
        }
        if let Some(v) = &req.qk_qty {
            q = q.bind(v);
        }
        if let Some(v) = &req.sk_unit {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
