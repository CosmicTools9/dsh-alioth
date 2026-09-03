//! 实体版本共享内核（entity 面）——consolidate-version-services
//!
//! 从 WZ/Alioth version 生产实现提取的唯一实现来源：
//! - models：VersionRecord（`isahl.zc_id_version` 全字段）+ Create/Update 请求
//! - repository：GenericRepository 包装（create 依赖列默认 `gen_next_zuid()`——
//!   `zc_id_version` 表 id 默认即 gen_next_zuid，版本 id 的 ZUID 全局唯一语义正确）+ 动态 SET update
//! - service：CRUD 委托 + 版本链回溯（fk_previous）+ 回滚（rollback_to_version）
//!
//! 保留既有 git 语义面（VersionRecord/VersionDiff/VersionService trait，AVIC 使用）不变。

pub mod models;
pub mod repository;
pub mod service;

pub use models::{CreateVersionRequest, UpdateVersionRequest, VersionRecord};
pub use repository::VersionRepository;
pub use service::VersionService;
