//! System Config Repository Trait
//!
//! 提供系统配置的数据库访问抽象，不绑定具体表结构。
//! 使用方（Gateway / Meta）需自行实现该 trait，并映射到各自的物理表。
//!
//! 如需接入标准 CRUD v2 路由（`crud_routes`），实现者可额外实现 `AliothRepository`。

use async_trait::async_trait;
use sqlx::Error;

use crate::models::{CreateSystemConfigRequest, SystemConfig, UpdateSystemConfigRequest};

/// 系统配置 Repository trait
///
/// 包含基础 CRUD 与扩展查询能力。使用方需实现全部方法。
#[async_trait]
pub trait SystemConfigRepository: Clone + Send + Sync + 'static {
    /// 根据配置编码查找
    async fn find_by_code(&self, code: &str) -> Result<Option<SystemConfig>, Error>;

    // 基础 CRUD（原由 CrudRepository 提供，现显式声明）

    /// 创建配置
    async fn insert(&self, req: &CreateSystemConfigRequest) -> Result<SystemConfig, Error>;

    /// 根据 ID 获取配置
    async fn find_by_id(&self, id: i64) -> Result<Option<SystemConfig>, Error>;

    /// 列出配置（分页）
    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<SystemConfig>, Error>;

    /// 更新配置
    async fn update(
        &self,
        id: i64,
        req: &UpdateSystemConfigRequest,
    ) -> Result<Option<SystemConfig>, Error>;

    /// 软删除配置
    async fn soft_delete(&self, id: i64) -> Result<u64, Error>;
}
