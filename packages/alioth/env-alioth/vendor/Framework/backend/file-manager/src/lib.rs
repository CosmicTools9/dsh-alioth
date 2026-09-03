//! # Framework File Manager
//!
//! 共享文件存储基础设施（落地活库 schema，2026-08-13 修复）：
//! - `StorageBackend` trait + `LocalBackend`（磁盘）+ `S3Backend`（S3 兼容：AWS/OSS/MinIO，feature `s3`）
//! - `FileService`：上传/下载/删除/列表，对齐 `isahl.zc_id_file-{document,image,avatar,package,ver_ctrl}`
//!   + URL 存储链（file_rr_url → stor-plc-url → info-url）+ `qk_size → zc_id_scal-data` 标量
//! - `FileManager`：多后端容器（scheme 路由）——下载按链上 `info-url.scheme` 动态选后端
//! - 存储空间按 namespace 物理分隔：存储键 `{ns}/{table_kind}/{file_id}/{filename}`
//!
//! 配置单一真相源为 `isahl.zc_id_prot-oss_config`（settings jsonb，含 provider/
//! endpoint/region/bucket/force_path_style 等），装配入口在 Gateway
//! `FilesState::from_live_db`——本 crate 不提供环境变量配置路径。
//! 无任何配置行时 Gateway 兜底 local（`./data/local-files`）。

use std::sync::Arc;

use crate::backend::StorageBackend;

pub mod backend;
pub mod error;
pub mod models;
pub mod repository;
pub mod service;

pub use error::FileError;
pub use models::{DownloadResult, FileRecord, FileTableKind, UpdateRequest, UploadRequest};
pub use repository::{FileRepository, SqlxFileRepository};
pub use service::{BackendResolver, FileService};

/// 多后端容器：`(scheme, backend)` 映射 + 默认上传后端。
pub struct FileManager {
    backends: Vec<(String, Arc<dyn StorageBackend>)>,
    default_scheme: String,
}

impl FileManager {
    /// 仅本地后端（默认）。
    pub fn local_only(
        base_path: impl Into<std::path::PathBuf>,
        base_url: impl Into<String>,
    ) -> Self {
        Self::new(
            models::SCHEME_LOCAL.to_string(),
            vec![(
                models::SCHEME_LOCAL.to_string(),
                Arc::new(backend::LocalBackend::new(base_path, base_url)),
            )],
        )
    }

    /// 显式构造：`default_scheme` 必须存在于 `backends`。
    pub fn new(default_scheme: String, backends: Vec<(String, Arc<dyn StorageBackend>)>) -> Self {
        debug_assert!(
            backends.iter().any(|(s, _)| s == &default_scheme),
            "default_scheme {default_scheme} not registered"
        );
        Self {
            backends,
            default_scheme,
        }
    }

    /// 按 scheme 取后端；未知 scheme → 诚实错误（配置缺失）。
    pub fn backend_for_scheme(&self, scheme: &str) -> Result<Arc<dyn StorageBackend>, FileError> {
        self.backends
            .iter()
            .find(|(s, _)| s == scheme)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| {
                FileError::Config(format!(
                    "storage backend '{scheme}' not configured (scheme 路由失败，检查 zc_id_prot-oss_config storage 配置)"
                ))
            })
    }

    pub fn default_backend(&self) -> Arc<dyn StorageBackend> {
        self.backend_for_scheme(&self.default_scheme)
            .expect("default scheme registered at construction")
    }

    pub fn default_scheme(&self) -> &str {
        &self.default_scheme
    }
}

impl BackendResolver for FileManager {
    fn backend_for_scheme(&self, scheme: &str) -> Result<Arc<dyn StorageBackend>, FileError> {
        FileManager::backend_for_scheme(self, scheme)
    }

    fn default_backend(&self) -> Arc<dyn StorageBackend> {
        FileManager::default_backend(self)
    }

    fn default_scheme(&self) -> &str {
        FileManager::default_scheme(self)
    }
}
