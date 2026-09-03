//! DTOs for the file manager: FileRecord, UploadRequest, DownloadResult.
//!
//! 对齐活库 schema（information_schema 实证，2026-08-13）：
//! `isahl.zc_id_file-{document,image,avatar,package,ver_ctrl}` 五张文件表，
//! 真实列：notice(文件名)/code(FIL-{id})/qk_size(→zc_id_scal-data 标量引用)
//! /encoding(enum)/ak_benefit_user|ak_permit_user|ak_access_user(行级授权列)
//! /dk_scene|dk_factor|dk_function(本体维度)/ck_category/created_by_id。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 文件存储后端标识（`zc_id_info-url.scheme` 取值）。
pub const SCHEME_LOCAL: &str = "local";
pub const SCHEME_S3: &str = "s3";
pub const SCHEME_OSS: &str = "oss";
/// 腾讯云 COS（S3 兼容端点）
pub const SCHEME_COS: &str = "cos";
/// 华为云 OBS（S3 兼容端点）
pub const SCHEME_OBS: &str = "obs";
/// MinIO / 自建 S3（path-style 惯例）
pub const SCHEME_MINIO: &str = "minio";

/// 上传后默认文件编码（`zc_id_prod_file_encoding_enum` 的 UTF-8 值，表 DEFAULT）。
pub const DEFAULT_ENCODING: &str = "UTF-8";

/// Which live `zc_id_file-*` table to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileTableKind {
    /// `isahl.zc_id_file-document` — 通用文档文件（默认）
    Document,
    /// `isahl.zc_id_file-image` — 图片文件
    Image,
    /// `isahl.zc_id_file-avatar` — 头像文件
    Avatar,
    /// `isahl.zc_id_file-package` — 包文件
    Package,
    /// `isahl.zc_id_file-ver_ctrl` — 版本控制文件（含 lock/hashcode）
    Versioned,
}

impl FileTableKind {
    /// 活库表名（information_schema 实证：仅这 5 张 + 无 zc_id_file 基表）。
    pub fn table_name(&self) -> &'static str {
        match self {
            Self::Document => r#"isahl."zc_id_file-document""#,
            Self::Image => r#"isahl."zc_id_file-image""#,
            Self::Avatar => r#"isahl."zc_id_file-avatar""#,
            Self::Package => r#"isahl."zc_id_file-package""#,
            Self::Versioned => r#"isahl."zc_id_file-ver_ctrl""#,
        }
    }

    /// 存储键路径段（`{ns}/{kind}/{file_id}/{filename}`）。
    pub fn path_segment(&self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Image => "image",
            Self::Avatar => "avatar",
            Self::Package => "package",
            Self::Versioned => "ver_ctrl",
        }
    }

    pub fn from_path_segment(seg: &str) -> Option<Self> {
        match seg {
            "document" => Some(Self::Document),
            "image" => Some(Self::Image),
            "avatar" => Some(Self::Avatar),
            "package" => Some(Self::Package),
            "ver_ctrl" => Some(Self::Versioned),
            _ => None,
        }
    }

    /// kind 允许的扩展名白名单（HTTP 层全局白名单之外的 kind 约束）。
    /// document 保留全量；image/avatar 仅图片；package/ver_ctrl 仅归档。
    pub fn allowed_extensions(&self) -> &'static [&'static str] {
        match self {
            Self::Document => &[
                "pdf", "png", "jpg", "jpeg", "doc", "docx", "xls", "xlsx", "txt",
            ],
            Self::Image | Self::Avatar => &["png", "jpg", "jpeg"],
            Self::Package | Self::Versioned => &["zip"],
        }
    }
}

/// DB record for a file (maps to live `zc_id_file-*` columns).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub created_at: DateTime<Utc>,
    #[serde(with = "common::serde_zuid::opt")]
    pub created_by_id: Option<i64>,
    /// 文件名（物理列 notice）
    pub filename: Option<String>,
    /// 文件编码（物理列 encoding，默认 UTF-8）
    pub encoding: Option<String>,
    /// 业务编码（物理列 code，`FIL-{id}`）
    pub code: Option<String>,
    /// 文件大小（解析 `qk_size → zc_id_scal-data.mark`，字节数）
    pub size: Option<i64>,
    /// 本体维度（dk_scene/dk_factor/dk_function）
    pub dk_scene: Option<i64>,
    pub dk_factor: Option<i64>,
    pub dk_function: Option<i64>,
    /// 文件分类（ck_category，可选）
    pub ck_category: Option<i64>,
    /// 行级授权列（bigint[]）
    pub ak_benefit_user: Option<Vec<i64>>,
    pub ak_permit_user: Option<Vec<i64>>,
    pub ak_access_user: Option<Vec<i64>>,
    /// 存储后端 scheme（经 URL 链解析 `zc_id_info-url.scheme`）
    pub scheme: Option<String>,
    /// 存储相对路径 / 对象键（经 URL 链解析 `zc_id_info-url.path`）
    pub storage_path: Option<String>,
    /// 下载 URL（`/api/files/{id}`）
    pub url: Option<String>,
    /// SHA-256 内容校验和（上传时计算）
    pub checksum: Option<String>,
}

/// Input for uploading a new file.
#[derive(Debug, Clone)]
pub struct UploadRequest {
    /// Original filename (e.g. "photo.png") — 服务层净化后写入 notice。
    pub filename: String,
    /// MIME type (e.g. "image/png")。
    pub content_type: String,
    /// Raw file bytes。
    pub data: Vec<u8>,
    /// Which live child table to insert into。
    pub table_kind: FileTableKind,
    pub notice: Option<String>,
    pub code: Option<String>,
    pub dk_scene: Option<i64>,
    pub dk_factor: Option<i64>,
    pub dk_function: Option<i64>,
    pub ck_category: Option<i64>,
    pub ak_benefit_user: Option<Vec<i64>>,
    pub ak_permit_user: Option<Vec<i64>>,
    pub ak_access_user: Option<Vec<i64>>,
    pub created_by_id: Option<i64>,
}

impl UploadRequest {
    pub fn new(
        filename: impl Into<String>,
        content_type: impl Into<String>,
        data: Vec<u8>,
    ) -> Self {
        Self {
            filename: filename.into(),
            content_type: content_type.into(),
            data,
            table_kind: FileTableKind::Document,
            notice: None,
            code: None,
            dk_scene: None,
            dk_factor: None,
            dk_function: None,
            ck_category: None,
            ak_benefit_user: None,
            ak_permit_user: None,
            ak_access_user: None,
            created_by_id: None,
        }
    }
}

/// 更新请求（`PUT /api/files/{id}`）：字段可选，None 表示不更新。
/// - `data`：替换字节（重算 checksum + size 标量）
/// - `filename`：改名（移动存储对象 + 更新 URL 链 path）
/// - `ak_*`：更新行级授权列
#[derive(Debug, Clone, Default)]
pub struct UpdateRequest {
    pub filename: Option<String>,
    pub data: Option<Vec<u8>>,
    pub ak_benefit_user: Option<Vec<i64>>,
    pub ak_permit_user: Option<Vec<i64>>,
    pub ak_access_user: Option<Vec<i64>>,
}

pub struct DownloadResult {
    pub data: Vec<u8>,
    pub filename: String,
    pub content_type: String,
    pub record: FileRecord,
}
