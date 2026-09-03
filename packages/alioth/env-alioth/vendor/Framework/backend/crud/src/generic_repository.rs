//! GenericRepository — 标准 CRUD 的默认实现
//!
//! 为只有标准列表/获取/删除需求的实体提供开箱即用的 Repository 实现。
//! 模块只需实现 `create`/`update`（涉及字段映射和本体绑定），
//! `list`/`get`/`delete` 由 `GenericRepository` 自动委托给 `QueryBuilder`。
//!
//! # 使用示例
//!
//! ```rust,ignore
//! use crud::{AliothRepository, GenericRepository};
//!
//! #[derive(Clone)]
//! pub struct LlmConfigRepository {
//!     generic: GenericRepository<LlmConfig>,
//! }
//!
//! impl From<PgPool> for LlmConfigRepository {
//!     fn from(pool: PgPool) -> Self {
//!         Self { generic: GenericRepository::new(pool) }
//!     }
//! }
//!
//! #[async_trait]
//! impl AliothRepository<LlmConfig, LlmConfigInput, LlmConfigInput, ApiError> for LlmConfigRepository {
//!     async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<LlmConfig>, ApiError> {
//!         self.generic.list(query).await
//!     }
//!
//!     async fn get(&self, id: i64) -> Result<Option<LlmConfig>, ApiError> {
//!         self.generic.get(id).await
//!     }
//!
//!     async fn create(&self, req: LlmConfigInput, user_id: i64) -> Result<LlmConfig, ApiError> {
//!         // 模块自定义 INSERT 逻辑 ...
//!     }
//!
//!     async fn update(&self, id: i64, req: LlmConfigInput, user_id: i64) -> Result<Option<LlmConfig>, ApiError> {
//!         // 模块自定义 UPDATE 逻辑 ...
//!     }
//!
//!     async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
//!         self.generic.delete(id, user_id).await
//!     }
//! }
//! ```

use sqlx::PgPool;

use crate::entity::AliothDbEntity;
use crate::pagination::{ListQuery, PaginatedResponse};
use crate::query_builder::QueryBuilder;
use crate::reference::HasReferenceJoins;
use common::AliothError;

/// 标准 CRUD Repository 的通用实现
///
/// 封装 `QueryBuilder` 的标准 list/get/delete 操作。
/// 模块级 Repository 可通过组合此结构体消除 list/get/delete 的样板代码。
pub struct GenericRepository<E: AliothDbEntity> {
    pool: PgPool,
    _phantom: std::marker::PhantomData<E>,
}

impl<E: AliothDbEntity> Clone for GenericRepository<E> {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<E: AliothDbEntity> std::fmt::Debug for GenericRepository<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenericRepository")
            .field("entity", &std::any::type_name::<E>())
            .finish()
    }
}

impl<E: AliothDbEntity> GenericRepository<E> {
    /// 创建新的 GenericRepository 实例
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            _phantom: std::marker::PhantomData,
        }
    }

    /// 获取内部的数据库连接池引用
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// 高级分页列表（支持过滤 + 排序）
    pub async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<E>, AliothError> {
        QueryBuilder::<E>::from_list_query(&self.pool, query)
            .fetch(query.page, query.page_size)
            .await
    }

    /// 带 RLS 行级过滤的列表查询
    pub async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<PaginatedResponse<E>, AliothError> {
        let mut qb = QueryBuilder::<E>::from_list_query(&self.pool, query);
        if let Some(ids) = visible_ids {
            qb = qb.with_visible_ids(ids.to_vec());
        }
        if let Some(cols) = authorized_columns {
            qb = qb.with_authorized_columns(cols.to_vec());
        }
        qb.fetch(query.page, query.page_size).await
    }

    /// 根据 ID 获取实体
    pub async fn get(&self, id: i64) -> Result<Option<E>, AliothError> {
        QueryBuilder::<E>::get(&self.pool, id, None, None).await
    }

    /// 根据 ID 获取实体（含 RLS 校验）
    pub async fn get_with_rls(
        &self,
        id: i64,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<Option<E>, AliothError> {
        QueryBuilder::<E>::get(&self.pool, id, visible_ids, authorized_columns).await
    }

    /// 软删除实体
    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        // 1. Fetch entity for trigger
        let entity = QueryBuilder::<E>::get(&self.pool, id, None, None).await?;
        if entity.is_none() {
            return Err(AliothError::NotFound(format!(
                "{} {} not found",
                E::table_name(),
                id
            )));
        }

        // 2. Soft delete
        let rows = QueryBuilder::<E>::soft_delete(&self.pool, id, user_id).await?;
        if rows == 0 {
            return Err(AliothError::NotFound(format!(
                "{} {} not found",
                E::table_name(),
                id
            )));
        }

        // 3. Trigger AFTER DELETE for soft delete NGAC sync
        if let Some(ref e) = entity {
            let result_map = crate::trigger::to_record(e)
                .map_err(|err| AliothError::Database(err.to_string()))?;
            let _ = crate::trigger::execute_after_delete(
                &self.pool,
                E::trigger_table_name(),
                &result_map,
                Some(user_id),
            )
            .await;

            // 4. Audit outbox（ADR D-010）：删除事件入队（轻量直插路径）
            let event = crate::audit_outbox::OutboxEvent::for_table(
                E::table_name(),
                id,
                crate::audit_outbox::AuditAction::Delete,
            )
            .with_user(user_id)
            .with_values(Some(e), None::<&E>);
            if let Err(err) = crate::audit_outbox::enqueue(&self.pool, &event).await {
                // 审计失败不回滚业务（ADR D-010 分离语义）；事件丢失由
                // 写路径盘点与滞后观测兜底，日志留证
                common::telemetry::warn!(
                    "audit_outbox enqueue failed ({} {}): {}",
                    E::table_name(),
                    id,
                    err
                );
            }
        }

        Ok(())
    }

    /// 批量软删除实体
    pub async fn batch_delete(&self, ids: Vec<i64>, user_id: i64) -> Result<(), AliothError> {
        if ids.is_empty() {
            return Ok(());
        }

        // 1. Fetch entities for triggers
        let mut entities = Vec::new();
        for &id in &ids {
            if let Some(entity) = QueryBuilder::<E>::get(&self.pool, id, None, None).await? {
                entities.push(entity);
            }
        }

        // 2. Batch soft delete
        let rows = QueryBuilder::<E>::batch_soft_delete(&self.pool, &ids, user_id).await?;
        if rows == 0 {
            return Err(AliothError::NotFound(format!(
                "{} none of the provided IDs were found",
                E::table_name(),
            )));
        }

        // 3. Trigger AFTER DELETE for each entity
        for entity in &entities {
            let result_map = crate::trigger::to_record(entity)
                .map_err(|err| AliothError::Database(err.to_string()))?;
            let _ = crate::trigger::execute_after_delete(
                &self.pool,
                E::trigger_table_name(),
                &result_map,
                Some(user_id),
            )
            .await;
        }

        // 4. Audit outbox（ADR D-010）：逐实体入队
        for entity in &entities {
            let event = crate::audit_outbox::OutboxEvent::for_table(
                E::table_name(),
                entity.id(),
                crate::audit_outbox::AuditAction::Delete,
            )
            .with_user(user_id)
            .with_values(Some(entity), None::<&E>);
            if let Err(err) = crate::audit_outbox::enqueue(&self.pool, &event).await {
                common::telemetry::warn!(
                    "audit_outbox enqueue failed ({} {}): {}",
                    E::table_name(),
                    entity.id(),
                    err
                );
            }
        }

        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 关键词搜索——仅当 E: KeywordSearchable 时可用
// ═══════════════════════════════════════════════════════════════════════════════

impl<E: AliothDbEntity + crate::search::KeywordSearchable> GenericRepository<E> {
    /// 关键词搜索（返回分页结果，不含引用解析）
    pub async fn search(
        &self,
        query: &ListQuery,
        keyword: &str,
    ) -> Result<PaginatedResponse<E>, AliothError> {
        QueryBuilder::<E>::from_list_query(&self.pool, query)
            .with_keyword(keyword)
            .fetch(query.page, query.page_size)
            .await
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 引用解析扩展——仅当 E: HasReferenceJoins 时可用
// ═══════════════════════════════════════════════════════════════════════════════

impl<E: AliothDbEntity + HasReferenceJoins> GenericRepository<E> {
    /// 分页列表（含引用解析，输出 `_refs` 嵌入 JSONB）
    pub async fn list_refs(&self, query: &ListQuery) -> Result<PaginatedResponse<E>, AliothError> {
        QueryBuilder::<E>::from_list_query(&self.pool, query)
            .fetch_refs(query.page, query.page_size)
            .await
    }

    /// 分页列表（含引用解析 + RLS）
    pub async fn list_refs_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<PaginatedResponse<E>, AliothError> {
        let mut qb = QueryBuilder::<E>::from_list_query(&self.pool, query);
        if let Some(ids) = visible_ids {
            qb = qb.with_visible_ids(ids.to_vec());
        }
        if let Some(cols) = authorized_columns {
            qb = qb.with_authorized_columns(cols.to_vec());
        }
        qb.fetch_refs(query.page, query.page_size).await
    }

    /// 根据 ID 获取实体（含引用解析）
    pub async fn get_refs(
        &self,
        id: i64,
        authorized_columns: Option<&[String]>,
    ) -> Result<Option<E>, AliothError> {
        QueryBuilder::<E>::get_refs(&self.pool, id, authorized_columns).await
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 引用解析 + 关键词搜索——仅当 E: HasReferenceJoins + KeywordSearchable 时可用
// ═══════════════════════════════════════════════════════════════════════════════

impl<E: AliothDbEntity + HasReferenceJoins + crate::search::KeywordSearchable>
    GenericRepository<E>
{
    /// 关键词搜索（含引用解析）
    pub async fn search_refs(
        &self,
        query: &ListQuery,
        keyword: &str,
    ) -> Result<PaginatedResponse<E>, AliothError> {
        QueryBuilder::<E>::from_list_query(&self.pool, query)
            .with_keyword(keyword)
            .fetch_refs(query.page, query.page_size)
            .await
    }
}
