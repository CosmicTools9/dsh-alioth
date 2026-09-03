//! AliothRepository trait 与 SubtableRouter seam
//!
//! 提供标准化的 CRUD Repository 接口，取代旧版的 CrudService + CrudRepository。

use async_trait::async_trait;

use crate::entity::AliothDbEntity;
use crate::pagination::{ListQuery, PaginatedResponse};
use common::AliothError;

/// 子表路由 seam
///
/// 当实体 INSERT 需要根据 discriminator 路由到不同物理子表时，模块实现此 trait。
/// 例如 `zc_id_production` 根据业务类型路由到 `zc_id_prod-made`、`zc_id_prod-sales` 等。
pub trait SubtableRouter {
    /// 根据 discriminator 解析目标子表物理名
    ///
    /// `discriminator` 为 `None` 时返回默认子表。
    fn resolve_subtable(&self, discriminator: Option<&str>) -> Result<&'static str, AliothError>;
}

/// 标准 CRUD Repository 接口
///
/// 模块为每个实体实现此 trait。标准读操作（`list`/`get`/`delete`）可委托给 `QueryBuilder`；
/// `create`/`update` 因涉及字段映射、子表路由、事务控制，由模块自行实现。
///
/// # 类型参数
/// - `E`: 实体类型，必须实现 `AliothDbEntity`
/// - `C`: 创建请求 DTO
/// - `U`: 更新请求 DTO
/// - `Err`: 错误类型，必须能从 `sqlx::Error` 和 `AliothError` 转换
#[async_trait]
pub trait AliothRepository<E, C, U, Err>: Clone + Send + Sync + 'static
where
    E: AliothDbEntity,
    C: Send + Sync + 'static,
    U: Send + Sync + 'static,
    Err: std::error::Error + From<sqlx::Error> + From<AliothError> + Send + Sync + 'static,
{
    /// 高级分页列表（支持过滤 + 排序）
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<E>, Err>;

    async fn list_with_rls(
        &self,
        query: &ListQuery,
        _visible_ids: Option<&[i64]>,
        _authorized_columns: Option<&[String]>,
    ) -> Result<PaginatedResponse<E>, Err> {
        self.list(query).await
    }

    /// 根据 ID 获取实体
    async fn get(&self, id: i64) -> Result<Option<E>, Err>;

    async fn get_with_rls(
        &self,
        id: i64,
        _visible_ids: Option<&[i64]>,
        _authorized_columns: Option<&[String]>,
    ) -> Result<Option<E>, Err> {
        self.get(id).await
    }

    /// 创建实体
    async fn create(&self, req: C, user_id: i64) -> Result<E, Err>;

    /// 更新实体
    async fn update(&self, id: i64, req: U, user_id: i64) -> Result<Option<E>, Err>;

    /// 删除实体（软删除）
    async fn delete(&self, id: i64, user_id: i64) -> Result<(), Err>;

    async fn create_with_rls(
        &self,
        req: C,
        user_id: i64,
        _dk_ctx: Option<&common::dk_context::DkContext>,
    ) -> Result<E, Err> {
        self.create(req, user_id).await
    }

    async fn update_with_rls(
        &self,
        id: i64,
        req: U,
        user_id: i64,
        _dk_ctx: Option<&common::dk_context::DkContext>,
    ) -> Result<Option<E>, Err> {
        self.update(id, req, user_id).await
    }

    async fn delete_with_rls(
        &self,
        id: i64,
        user_id: i64,
        _dk_ctx: Option<&common::dk_context::DkContext>,
    ) -> Result<(), Err> {
        self.delete(id, user_id).await
    }

    /// 批量删除实体（软删除）
    ///
    /// 默认返回 NotImplemented，模块可覆盖以支持批量删除。
    async fn batch_delete(&self, _ids: Vec<i64>, _user_id: i64) -> Result<(), Err> {
        Err(
            AliothError::NotImplemented("batch_delete not implemented for this entity".to_string())
                .into(),
        )
    }
}

/// 统一 Repository 构造宏 —— 建仓库的唯一方式（替代各 service 本地 `make_repo!` 复制品）。
///
/// 生成：`Clone` 结构体（内含 `GenericRepository<E>`）、`new(pool)`、`From<PgPool>`、
/// `AliothRepository` 实现（delete 一律委托 `GenericRepository::delete`）。
///
/// # 变体 1 — 只读仓库
///
/// ```ignore
/// make_repository!(StatusRepository, Status, CreateStatusRequest, UpdateStatusRequest);
/// ```
///
/// list/get 委托 `GenericRepository::list_refs/get_refs`；create/update 返回
/// `NotImplemented`（HTTP 501）。适用于路由仅暴露 GET/DELETE 的只读服务。
///
/// 含真实写逻辑的仓库（子表路由、标量引用转换等）不在宏覆盖范围内——
/// 手写 `AliothRepository` impl（与 `isahl-db` 既有模式一致）。
///
/// # 路径约定
///
/// crud 内部类型一律 `$crate::` 引用；外部路径在展开点解析：
/// `::sqlx::PgPool`、`::async_trait::async_trait`、`common::AliothError`——
/// 调用方 MUST 直接依赖 `sqlx`、`async_trait`、`common`。
#[macro_export]
macro_rules! make_repository {
    // 变体 1：只读仓库（create/update → NotImplemented）
    ($name:ident, $entity:ty, $create:ty, $update:ty $(,)?) => {
        #[derive(Clone)]
        pub struct $name {
            generic: $crate::GenericRepository<$entity>,
        }
        impl $name {
            pub fn new(pool: ::sqlx::PgPool) -> Self {
                Self {
                    generic: $crate::GenericRepository::new(pool),
                }
            }
        }
        impl From<::sqlx::PgPool> for $name {
            fn from(pool: ::sqlx::PgPool) -> Self {
                Self::new(pool)
            }
        }
        #[::async_trait::async_trait]
        impl $crate::AliothRepository<$entity, $create, $update, common::AliothError> for $name {
            async fn list(
                &self,
                q: &$crate::ListQuery,
            ) -> Result<$crate::PaginatedResponse<$entity>, common::AliothError> {
                self.generic.list_refs(q).await
            }
            async fn get(&self, id: i64) -> Result<Option<$entity>, common::AliothError> {
                self.generic.get_refs(id, None).await
            }
            async fn create(
                &self,
                _req: $create,
                _user_id: i64,
            ) -> Result<$entity, common::AliothError> {
                Err(common::AliothError::NotImplemented(
                    "create not implemented".into(),
                ))
            }
            async fn update(
                &self,
                _id: i64,
                _req: $update,
                _user_id: i64,
            ) -> Result<Option<$entity>, common::AliothError> {
                Err(common::AliothError::NotImplemented(
                    "update not implemented".into(),
                ))
            }
            async fn delete(&self, id: i64, user_id: i64) -> Result<(), common::AliothError> {
                self.generic.delete(id, user_id).await
            }
        }
    };
}
