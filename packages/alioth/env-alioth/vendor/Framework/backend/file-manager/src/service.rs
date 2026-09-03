//! High-level file service combining storage backends + DB repository.
//!
//! 上传事务（无半完成态）：SHA-256 → 事务内写链（标量 qk_size → 文件行 →
//! info-url → stor-plc-url → file_rr_url）→ 字节落盘
//! （失败回滚，无孤儿 DB 行/磁盘残留）。下载经 URL 链解析存储位置，按
//! `info-url.scheme` 路由到对应后端（local/s3/oss）。
//!
//! checksum 闭环（零 DDL，isahl 冻结）：SHA-256 存 `info-url.notice`
//! （`sha256:{hex}` 前缀）；下载重算比对（ChecksumMismatch）；无前缀旧记录跳过。

use std::sync::Arc;

use sqlx::PgPool;

use crate::backend::StorageBackend;
use crate::error::FileError;
use crate::models::{DownloadResult, FileRecord, FileTableKind, UpdateRequest, UploadRequest};
use crate::repository::FileRepository;

/// scheme → 存储后端路由表（FileManager 构造，lib.rs）。
pub trait BackendResolver: Send + Sync {
    /// 按 scheme 取后端；未知 scheme 返回 Err（诚实失败）。
    fn backend_for_scheme(&self, scheme: &str) -> Result<Arc<dyn StorageBackend>, FileError>;
    /// 默认上传后端。
    fn default_backend(&self) -> Arc<dyn StorageBackend>;
    /// 默认上传 scheme（写入 info-url.scheme）。
    fn default_scheme(&self) -> &str;
}

/// High-level file service.
#[derive(Clone)]
pub struct FileService {
    storage: Arc<dyn StorageBackend>,
    repo: Arc<dyn FileRepository>,
    pool: PgPool,
    backends: Arc<dyn BackendResolver>,
}

impl FileService {
    pub fn new(
        storage: Arc<dyn StorageBackend>,
        repo: Box<dyn FileRepository>,
        pool: PgPool,
        backends: Arc<dyn BackendResolver>,
    ) -> Self {
        Self {
            storage,
            repo: Arc::from(repo),
            pool,
            backends,
        }
    }

    /// 上传：SHA-256 → 事务写链 → 字节落盘（失败整体回滚）。
    /// `namespace` 为存储空间分隔键（X-Namespace header，格式由调用方校验）。
    /// kind↔扩展名 + magic bytes 双校验（服务层兜底，HTTP 层另有 400 前置校验）。
    pub async fn upload(
        &self,
        namespace: &str,
        req: UploadRequest,
    ) -> Result<FileRecord, FileError> {
        let data = &req.data;
        if data.is_empty() {
            return Err(FileError::InvalidKey("empty file".into()));
        }

        // kind↔扩展名白名单 + 二进制 magic bytes 校验
        let ext = req.filename.rsplit('.').next().unwrap_or("").to_lowercase();
        if !req.table_kind.allowed_extensions().contains(&ext.as_str()) {
            return Err(FileError::InvalidKey(format!(
                "扩展名 .{ext} 不属于 kind {:?}",
                req.table_kind
            )));
        }
        if !validate_magic(&ext, data) {
            return Err(FileError::InvalidKey(format!(
                "内容与 .{ext} 类型不符（magic bytes 校验失败）"
            )));
        }

        // SHA-256 内容校验和（SECURITY_SPEC §6：禁止 MD5；sha2 0.11 用 hex 编码）
        use sha2::{Digest, Sha256};
        let checksum = hex::encode(Sha256::digest(data));

        // 文件 id 预生成（磁盘路径 + code 需要）
        let file_id: i64 = sqlx::query_scalar("SELECT isahl.gen_next_zuid()")
            .fetch_one(&self.pool)
            .await?;

        // 存储键：`{namespace}/{table_kind}/{file_id}/{filename}`（namespace 物理分隔）
        let rel_path = self.storage_key(namespace, req.table_kind, file_id, &req.filename);
        let scheme = self.backends.default_scheme().to_string();

        let mut tx = self.pool.begin().await?;

        // 1. 大小标量：qk_size → zc_id_scal-data.mark = 字节数（事务内创建，防孤儿标量行）
        let size_id = common::scalar::ScalarService::new(self.pool.clone())
            .find_or_create_mark_tx(
                &mut tx,
                sqlx::types::Decimal::from(data.len() as u64),
                r#"isahl."zc_id_scal-data""#,
                "size",
            )
            .await
            .map_err(|e| FileError::Config(format!("scalar: {e}")))?;

        // 2. 文件行（活表；encoding 用表 DEFAULT UTF-8；table 名来自编译期常量枚举）
        let code = req
            .code
            .clone()
            .unwrap_or_else(|| format!("FIL-{}", file_id));
        let insert_sql = format!(
            r#"INSERT INTO {table}
               (id, notice, code, qk_size, created_by_id,
                dk_scene, dk_factor, dk_function, ck_category,
                ak_benefit_user, ak_permit_user, ak_access_user)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"#,
            table = req.table_kind.table_name(),
        );
        sqlx::query(sqlx::AssertSqlSafe(insert_sql.as_str()))
            .bind(file_id)
            .bind(&req.filename)
            .bind(&code)
            .bind(size_id)
            .bind(req.created_by_id)
            .bind(req.dk_scene)
            .bind(req.dk_factor)
            .bind(req.dk_function)
            .bind(req.ck_category)
            .bind(&req.ak_benefit_user)
            .bind(&req.ak_permit_user)
            .bind(&req.ak_access_user)
            .execute(&mut *tx)
            .await?;

        // 3. URL 链（AVIC/WZ 实证）：info-url → stor-plc-url → file_rr_url
        //    info-url.notice 存 `sha256:{hex}`（checksum 零 DDL 落库位；scheme/path 列
        //    为存储定位权威，notice 不再冗余存 storage_key）
        let info_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_info-url"
               (id, notice, scheme, path, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4) RETURNING id"#,
        )
        .bind(format!("sha256:{checksum}"))
        .bind(&scheme)
        .bind(&rel_path)
        .bind(req.created_by_id)
        .fetch_one(&mut *tx)
        .await?;
        let stor_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_stor-plc-url"
               (id, notice, code, fk_address, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4) RETURNING id"#,
        )
        .bind(format!("FIL-{} 存储位置", file_id))
        .bind(&code)
        .bind(info_id)
        .bind(req.created_by_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_file_rr_url"
               (id, notice, ref_left, ref_right, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4)"#,
        )
        .bind(format!("file-{} → url", file_id))
        .bind(file_id)
        .bind(stor_id)
        .bind(req.created_by_id)
        .execute(&mut *tx)
        .await?;

        // 5. 字节落盘（事务内：失败回滚，无孤儿 DB 行/磁盘残留）
        self.storage
            .put(&rel_path, data.clone(), &req.content_type)
            .await?;

        tx.commit().await?;

        Ok(FileRecord {
            id: file_id,
            created_at: chrono::Utc::now(),
            created_by_id: req.created_by_id,
            filename: Some(req.filename),
            encoding: None,
            code: Some(code),
            size: Some(data.len() as i64),
            dk_scene: req.dk_scene,
            dk_factor: req.dk_factor,
            dk_function: req.dk_function,
            ck_category: req.ck_category,
            ak_benefit_user: req.ak_benefit_user,
            ak_permit_user: req.ak_permit_user,
            ak_access_user: req.ak_access_user,
            scheme: Some(scheme),
            storage_path: Some(rel_path),
            url: Some(format!("/api/files/{}", file_id)),
            checksum: Some(checksum),
        })
    }

    /// 下载：经 URL 链解析存储位置，按 scheme 路由后端读字节 + checksum 校验
    /// （旧记录无 checksum → 跳过校验）。
    pub async fn download(&self, file_id: i64) -> Result<DownloadResult, FileError> {
        let record = self
            .repo
            .find_by_id(file_id, None)
            .await?
            .ok_or(FileError::NotFound(file_id))?;

        let (scheme, path) = self
            .repo
            .resolve_storage_path(file_id)
            .await?
            .ok_or_else(|| FileError::NotFound(file_id))?;

        let backend = self.backends.backend_for_scheme(&scheme)?;
        let data = backend.get(&path).await?;

        if let Some(expected) = record.checksum.as_deref() {
            use sha2::{Digest, Sha256};
            let actual = hex::encode(Sha256::digest(&data));
            if actual != expected {
                return Err(FileError::ChecksumMismatch);
            }
        }

        Ok(DownloadResult {
            data,
            filename: record.filename.clone().unwrap_or_else(|| "file".into()),
            content_type: content_type_for(&record.filename.clone().unwrap_or_default())
                .to_string(),
            record,
        })
    }

    /// 下载字节区间 `[start, end]`（单区间；调用方负责 416/206 语义）。
    /// checksum 校验仅对整文件下载（download）执行——区间读取无法本地重算全文件。
    pub async fn download_range(
        &self,
        file_id: i64,
        start: u64,
        end: u64,
    ) -> Result<DownloadResult, FileError> {
        let record = self
            .repo
            .find_by_id(file_id, None)
            .await?
            .ok_or(FileError::NotFound(file_id))?;

        let (scheme, path) = self
            .repo
            .resolve_storage_path(file_id)
            .await?
            .ok_or_else(|| FileError::NotFound(file_id))?;

        let backend = self.backends.backend_for_scheme(&scheme)?;
        let data = backend.get_range(&path, start, end).await?;

        Ok(DownloadResult {
            data,
            filename: record.filename.clone().unwrap_or_else(|| "file".into()),
            content_type: content_type_for(&record.filename.clone().unwrap_or_default())
                .to_string(),
            record,
        })
    }

    /// 软删 + 磁盘清理（best effort：字节删除失败不阻断 DB 软删）。
    pub async fn delete(&self, file_id: i64, deleted_by_id: Option<i64>) -> Result<(), FileError> {
        let (scheme, path) = self
            .repo
            .resolve_storage_path(file_id)
            .await?
            .ok_or(FileError::NotFound(file_id))?;
        if let Ok(backend) = self.backends.backend_for_scheme(&scheme) {
            let _ = backend.delete(&path).await;
        }
        self.repo.soft_delete(file_id, deleted_by_id).await?;
        Ok(())
    }

    /// 更新（PUT）：可选替换字节 / 改名 / 更新 ak_* 授权列。
    /// - 改名 → 存储对象 move + 链 path 更新
    /// - 替换字节 → 重算 checksum（链 notice）+ size 标量
    ///
    /// 字节操作 best effort（与 delete 同口径：DB 失败回滚后字节可能已动）。
    pub async fn update(&self, file_id: i64, req: UpdateRequest) -> Result<FileRecord, FileError> {
        let chain = self
            .repo
            .resolve_chain(file_id)
            .await?
            .ok_or(FileError::NotFound(file_id))?;
        let record = self
            .repo
            .find_by_id(file_id, None)
            .await?
            .ok_or(FileError::NotFound(file_id))?;

        let old_filename = record.filename.clone().unwrap_or_default();
        let new_filename = req.filename.clone().unwrap_or_else(|| old_filename.clone());
        let filename_changed = new_filename != old_filename;
        let replacing = req.data.is_some();

        // 新字节/新文件名校验：kind 扩展名 + magic bytes
        if replacing || filename_changed {
            let ext = new_filename.rsplit('.').next().unwrap_or("").to_lowercase();
            if !chain.kind.allowed_extensions().contains(&ext.as_str()) {
                return Err(FileError::InvalidKey(format!(
                    "扩展名 .{ext} 不属于 kind {:?}",
                    chain.kind
                )));
            }
        }
        if let Some(data) = &req.data {
            let ext = new_filename.rsplit('.').next().unwrap_or("").to_lowercase();
            if !validate_magic(&ext, data) {
                return Err(FileError::InvalidKey(format!(
                    "内容与 .{ext} 类型不符（magic bytes 校验失败）"
                )));
            }
        }

        // 新存储键：替换旧 path 最后一段（文件名）
        let new_path = if filename_changed {
            let (prefix, _) = chain
                .path
                .rsplit_once('/')
                .ok_or_else(|| FileError::Config(format!("存储键缺少目录: {}", chain.path)))?;
            format!("{prefix}/{new_filename}")
        } else {
            chain.path.clone()
        };

        // 新 checksum（替换时计算）
        let new_checksum = if let Some(data) = &req.data {
            use sha2::{Digest, Sha256};
            Some(hex::encode(Sha256::digest(data)))
        } else {
            None
        };

        let mut tx = self.pool.begin().await?;

        // 字节操作（事务内：失败回滚）
        let backend = self.backends.backend_for_scheme(&chain.scheme)?;
        if filename_changed {
            backend.move_object(&chain.path, &new_path).await?;
        }
        if let Some(data) = &req.data {
            let content_type = content_type_for(&new_filename).to_string();
            backend.put(&new_path, data.clone(), &content_type).await?;
        }

        // 1. 文件行：notice + 可选 ak_* + 可选 qk_size
        let size_id: Option<i64> = if replacing {
            let data = req.data.as_ref().expect("replacing");
            Some(
                common::scalar::ScalarService::new(self.pool.clone())
                    .find_or_create_mark_tx(
                        &mut tx,
                        sqlx::types::Decimal::from(data.len() as u64),
                        r#"isahl."zc_id_scal-data""#,
                        "size",
                    )
                    .await
                    .map_err(|e| FileError::Config(format!("scalar: {e}")))?,
            )
        } else {
            None
        };

        let mut row_sql = format!(
            r#"UPDATE {table} SET notice = $2"#,
            table = chain.kind.table_name(),
        );
        let mut param = 2;
        if req.ak_access_user.is_some() {
            param += 1;
            row_sql.push_str(&format!(", ak_access_user = ${param}"));
        }
        if req.ak_permit_user.is_some() {
            param += 1;
            row_sql.push_str(&format!(", ak_permit_user = ${param}"));
        }
        if req.ak_benefit_user.is_some() {
            param += 1;
            row_sql.push_str(&format!(", ak_benefit_user = ${param}"));
        }
        if size_id.is_some() {
            param += 1;
            row_sql.push_str(&format!(", qk_size = ${param}"));
        }
        row_sql.push_str(" WHERE id = $1 AND deleted_at IS NULL");

        let mut row_q = sqlx::query(sqlx::AssertSqlSafe(row_sql.as_str()))
            .bind(file_id)
            .bind(&new_filename);
        if let Some(v) = &req.ak_access_user {
            row_q = row_q.bind(v);
        }
        if let Some(v) = &req.ak_permit_user {
            row_q = row_q.bind(v);
        }
        if let Some(v) = &req.ak_benefit_user {
            row_q = row_q.bind(v);
        }
        if let Some(v) = size_id {
            row_q = row_q.bind(v);
        }
        row_q.execute(&mut *tx).await?;

        // 2. URL 链：info-url path（改名）+ notice（替换时覆盖 checksum）
        let notice: Option<String> = new_checksum.as_ref().map(|cs| format!("sha256:{cs}"));
        sqlx::query(
            r#"UPDATE isahl."zc_id_info-url"
               SET path = $1, notice = COALESCE($2, notice), updated_at = NOW()
               WHERE id = $3 AND deleted_at IS NULL"#,
        )
        .bind(&new_path)
        .bind(notice)
        .bind(chain.info_url_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        self.repo
            .find_by_id(file_id, None)
            .await?
            .ok_or(FileError::NotFound(file_id))
    }

    /// 文件元数据（带行级授权过滤；`user=None` 内部调用不过滤）。
    pub async fn get_metadata(
        &self,
        file_id: i64,
        user: Option<i64>,
    ) -> Result<Option<FileRecord>, FileError> {
        self.repo.find_by_id(file_id, user).await
    }

    /// 列表（带行级授权过滤 + namespace 隔离 + 本体维度过滤），返回 `(records, total)`。
    #[allow(clippy::too_many_arguments)] // 列表过滤维度参数（场景/因子/功能/分页）
    pub async fn list_by_context(
        &self,
        table_kind: FileTableKind,
        user: Option<i64>,
        namespace: Option<&str>,
        dk_scene: Option<i64>,
        dk_factor: Option<i64>,
        dk_function: Option<i64>,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<FileRecord>, i64), FileError> {
        let limit = page_size.clamp(1, 100);
        let offset = (page.max(1) - 1) * limit;
        self.repo
            .list_by_context(
                table_kind,
                user,
                namespace,
                dk_scene,
                dk_factor,
                dk_function,
                limit,
                offset,
            )
            .await
    }

    /// presigned 直连下载 URL：local 后端无直连语义 → None（调用方回退代理下载）；
    /// S3/OSS → 后端签名 URL（含 CDN 短路）。
    pub async fn presigned_url(
        &self,
        file_id: i64,
        expires_in_secs: u64,
    ) -> Result<Option<String>, FileError> {
        let chain = self
            .repo
            .resolve_chain(file_id)
            .await?
            .ok_or(FileError::NotFound(file_id))?;
        if chain.scheme == crate::models::SCHEME_LOCAL {
            return Ok(None);
        }
        let backend = self.backends.backend_for_scheme(&chain.scheme)?;
        let url = backend.presigned_url(&chain.path, expires_in_secs).await?;
        Ok(Some(url))
    }

    /// 存储键：`{namespace}/{table_kind}/{file_id}/{filename}`。
    fn storage_key(
        &self,
        namespace: &str,
        kind: FileTableKind,
        file_id: i64,
        filename: &str,
    ) -> String {
        format!(
            "{}/{}/{}/{}",
            namespace,
            kind.path_segment(),
            file_id,
            filename
        )
    }
}

/// MIME 判定：扩展名静态 match（非结构化解析；与 WZ contract 同口径）。
pub fn content_type_for(filename: &str) -> &'static str {
    match filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "zip" => "application/zip",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// 二进制 magic bytes 校验（静态字节签名，非结构化格式解析——NO_REGEX 合规）。
/// txt 免检；pdf/png/jpg 固定头；doc/xls 为 OLE 复合文档；docx/xlsx/zip 为 ZIP 容器。
pub fn validate_magic(ext: &str, data: &[u8]) -> bool {
    match ext {
        "pdf" => data.starts_with(b"%PDF-"),
        "png" => data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        "jpg" | "jpeg" => data.starts_with(&[0xFF, 0xD8, 0xFF]),
        "zip" | "docx" | "xlsx" => data.starts_with(b"PK\x03\x04"),
        "doc" | "xls" => data.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]),
        "txt" => true,
        _ => true, // 白名单外扩展名由调用方（kind 白名单）拦截
    }
}
