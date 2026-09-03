//! SQL repository for the file manager — live `zc_id_file-*` tables.
//!
//! 读取路径的 kind 路由：文件行存放于 5 张 kind 表之一
//! （document/image/avatar/package/ver_ctrl），存储链 `file_rr_url → stor-plc-url →
//! info-url` 的 `path` 第二段即 kind（存储键 `{ns}/{kind}/{file_id}/{filename}`）。
//! 所有按 id 的读取/删除先经链解析 kind 再路由对应表——禁止硬编码 document 表。

use async_trait::async_trait;
use sqlx::PgPool;

use crate::error::FileError;
use crate::models::{FileRecord, FileTableKind};

/// 经 URL 链解析的完整存储信息（更新/删除/下载路由用）。
#[derive(Debug, Clone)]
pub struct ResolvedChain {
    pub scheme: String,
    pub path: String,
    pub kind: FileTableKind,
    /// `info-url.notice` 中的 `sha256:{hex}`；无前缀 → None（旧记录无校验和）。
    pub checksum: Option<String>,
    pub info_url_id: i64,
    pub stor_url_id: i64,
}

/// File repository trait for live `zc_id_file-*` DB operations.
#[async_trait]
pub trait FileRepository: Send + Sync {
    /// Find file by ID with optional row-level auth filter.
    /// `user = None` → 内部调用，不过滤。
    async fn find_by_id(&self, id: i64, user: Option<i64>)
        -> Result<Option<FileRecord>, FileError>;

    /// Soft-delete a file row (kind 由存储链解析)。
    async fn soft_delete(&self, id: i64, deleted_by_id: Option<i64>) -> Result<bool, FileError>;

    /// List files by context (dk_* dimensions + namespace 过滤) with row-level auth filter.
    /// 返回 `(records, total)`——total 为同过滤条件的计数（分页末页判定）。
    /// `namespace = Some(ns)` → 仅返回存储链 path 以 `{ns}/` 开头的文件（namespace 隔离）。
    #[allow(clippy::too_many_arguments)] // 列表过滤维度参数（场景/因子/功能/分页）
    async fn list_by_context(
        &self,
        table_kind: FileTableKind,
        user: Option<i64>,
        namespace: Option<&str>,
        dk_scene: Option<i64>,
        dk_factor: Option<i64>,
        dk_function: Option<i64>,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<FileRecord>, i64), FileError>;

    /// Resolve storage location via URL chain: `(scheme, path)`.
    async fn resolve_storage_path(&self, id: i64) -> Result<Option<(String, String)>, FileError>;

    /// 解析完整存储链（kind/checksum/链行 id），更新与删除路由用。
    async fn resolve_chain(&self, id: i64) -> Result<Option<ResolvedChain>, FileError>;
}

/// Sqlx-based file repository implementation.
#[derive(Clone)]
pub struct SqlxFileRepository {
    pool: PgPool,
}

impl SqlxFileRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 单文件行 SELECT 投影（行级授权列 + 本体维度）。
    /// encoding 为 PG 枚举（zc_id_prod_file_encoding_enum）→ ::text cast 供 sqlx 解码。
    const FILE_FIELDS: &'static str = r#"
        f.id, f.created_at, f.created_by_id, f.notice, f.code, f.encoding::text AS encoding,
        f.dk_scene, f.dk_factor, f.dk_function, f.ck_category,
        f.ak_benefit_user, f.ak_permit_user, f.ak_access_user
    "#;

    /// 行级授权谓词片段（绑定 $user）。
    const ROW_AUTH: &'static str = r#"
        AND (f.created_by_id = $USER
             OR $USER = ANY(f.ak_access_user)
             OR $USER = ANY(f.ak_permit_user)
             OR $USER = ANY(f.ak_benefit_user))
    "#;

    /// 经存储链解析文件 kind（path 第二段）；链缺失 → None。
    async fn resolve_kind(&self, id: i64) -> Result<Option<FileTableKind>, FileError> {
        let path: Option<String> = sqlx::query_scalar(
            r#"SELECT u.path
               FROM isahl."zc_id_file_rr_url" rr
               JOIN isahl."zc_id_stor-plc-url" s ON s.id = rr.ref_right AND s.deleted_at IS NULL
               JOIN isahl."zc_id_info-url" u ON u.id = s.fk_address AND u.deleted_at IS NULL
               WHERE rr.ref_left = $1 AND rr.deleted_at IS NULL
               ORDER BY rr.id DESC LIMIT 1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        match path {
            Some(p) => p
                .split('/')
                .nth(1)
                .and_then(FileTableKind::from_path_segment)
                .map(Some)
                .ok_or_else(|| FileError::Config(format!("无法从存储链 path 解析 kind: {p}"))),
            None => Ok(None),
        }
    }

    /// 文件行查询（可带行级授权过滤；kind 经存储链解析路由）。
    async fn fetch_file(
        &self,
        id: i64,
        user: Option<i64>,
    ) -> Result<Option<FileRecord>, FileError> {
        let Some(kind) = self.resolve_kind(id).await? else {
            return Ok(None);
        };
        let base = format!(
            r#"SELECT {fields} FROM {table} f WHERE f.id = $1 AND f.deleted_at IS NULL"#,
            fields = Self::FILE_FIELDS,
            table = kind.table_name(),
        );

        let row = if let Some(uid) = user {
            // 行级授权谓词（$USER 从 $2 起）
            let sql = format!("{base}{}", Self::ROW_AUTH.replace("$USER", "$2"));
            sqlx::query_as::<_, FileRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(id)
                .bind(uid)
                .fetch_optional(&self.pool)
                .await?
        } else {
            sqlx::query_as::<_, FileRow>(sqlx::AssertSqlSafe(base.as_str()))
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
        };
        self.decorate(row, kind).await
    }

    /// 补充派生字段：size（qk_size → zc_id_scal-data.mark）、storage_path/scheme（URL 链）、
    /// checksum（info-url.notice `sha256:` 前缀）。
    async fn decorate(
        &self,
        row: Option<FileRow>,
        kind: FileTableKind,
    ) -> Result<Option<FileRecord>, FileError> {
        let Some(row) = row else { return Ok(None) };
        let (scheme, storage_path, size, checksum) = self.fetch_derived(row.id, kind).await?;
        Ok(Some(FileRecord {
            id: row.id,
            created_at: row.created_at,
            created_by_id: row.created_by_id,
            filename: row.notice,
            encoding: row.encoding,
            code: row.code,
            size,
            dk_scene: row.dk_scene,
            dk_factor: row.dk_factor,
            dk_function: row.dk_function,
            ck_category: row.ck_category,
            ak_benefit_user: row.ak_benefit_user,
            ak_permit_user: row.ak_permit_user,
            ak_access_user: row.ak_access_user,
            scheme,
            storage_path,
            url: Some(format!("/api/files/{}", row.id)),
            checksum,
        }))
    }

    /// URL 链（scheme/path/checksum）+ 标量大小一次解析（kind 表路由）。
    async fn fetch_derived(
        &self,
        file_id: i64,
        kind: FileTableKind,
    ) -> Result<(Option<String>, Option<String>, Option<i64>, Option<String>), FileError> {
        let (scheme, path, notice): (Option<String>, Option<String>, Option<String>) =
            sqlx::query_as(
                r#"SELECT u.scheme, u.path, u.notice
               FROM isahl."zc_id_file_rr_url" rr
               JOIN isahl."zc_id_stor-plc-url" s ON s.id = rr.ref_right AND s.deleted_at IS NULL
               JOIN isahl."zc_id_info-url" u ON u.id = s.fk_address AND u.deleted_at IS NULL
               WHERE rr.ref_left = $1 AND rr.deleted_at IS NULL
               ORDER BY rr.id DESC LIMIT 1"#,
            )
            .bind(file_id)
            .fetch_optional(&self.pool)
            .await?
            .unwrap_or((None, None, None));

        let checksum = notice.and_then(|n| n.strip_prefix("sha256:").map(str::to_string));

        // qk_size → zc_id_scal-data.mark（标量真值；缺失返回 NULL，不把 ID 当值）
        let size_sql = format!(
            r#"SELECT sd.mark::bigint
               FROM {table} f
               JOIN isahl."zc_id_scal-data" sd ON sd.id = f.qk_size
               WHERE f.id = $1 AND f.deleted_at IS NULL"#,
            table = kind.table_name(),
        );
        let size: Option<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(size_sql.as_str()))
            .bind(file_id)
            .fetch_optional(&self.pool)
            .await?;

        Ok((scheme, path, size, checksum))
    }

    /// 列表过滤谓词（行级授权 + namespace 隔离 + dk_* 维度），返回 (WHERE 片段, 绑定参数个数)。
    /// 参数绑定顺序：user → namespace → dk_scene → dk_factor → dk_function。
    fn list_where_clause(
        user: Option<i64>,
        namespace: Option<&str>,
        dk_scene: Option<i64>,
        dk_factor: Option<i64>,
        dk_function: Option<i64>,
    ) -> (String, usize) {
        let mut sql = String::new();
        let mut param: usize = 0;
        if user.is_some() {
            param += 1;
            sql.push_str(&Self::ROW_AUTH.replace("$USER", &format!("${param}")));
        }
        if namespace.is_some() {
            param += 1;
            // namespace 隔离：存储链 info-url.path 以 `{ns}/` 开头
            sql.push_str(&format!(
                " AND EXISTS (SELECT 1 FROM isahl.\"zc_id_file_rr_url\" rr \
                 JOIN isahl.\"zc_id_stor-plc-url\" s ON s.id = rr.ref_right AND s.deleted_at IS NULL \
                 JOIN isahl.\"zc_id_info-url\" u ON u.id = s.fk_address AND u.deleted_at IS NULL \
                 WHERE rr.ref_left = f.id AND rr.deleted_at IS NULL AND u.path LIKE ${param})"
            ));
        }
        for (col, opt) in [
            ("f.dk_scene", dk_scene),
            ("f.dk_factor", dk_factor),
            ("f.dk_function", dk_function),
        ] {
            if opt.is_some() {
                param += 1;
                sql.push_str(&format!(" AND {col} = ${param}"));
            }
        }
        (sql, param)
    }
}

/// Raw DB row mapping — live `zc_id_file-*` common columns.
#[derive(Debug, sqlx::FromRow)]
struct FileRow {
    pub id: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub created_by_id: Option<i64>,
    pub notice: Option<String>,
    pub code: Option<String>,
    pub encoding: Option<String>,
    pub dk_scene: Option<i64>,
    pub dk_factor: Option<i64>,
    pub dk_function: Option<i64>,
    pub ck_category: Option<i64>,
    pub ak_benefit_user: Option<Vec<i64>>,
    pub ak_permit_user: Option<Vec<i64>>,
    pub ak_access_user: Option<Vec<i64>>,
}

#[async_trait]
impl FileRepository for SqlxFileRepository {
    async fn find_by_id(
        &self,
        id: i64,
        user: Option<i64>,
    ) -> Result<Option<FileRecord>, FileError> {
        self.fetch_file(id, user).await
    }

    async fn soft_delete(&self, id: i64, deleted_by_id: Option<i64>) -> Result<bool, FileError> {
        let Some(kind) = self.resolve_kind(id).await? else {
            return Ok(false);
        };
        let now = chrono::Utc::now();
        let update_sql = format!(
            r#"UPDATE {table}
               SET deleted_at = $1, deleted_by_id = $2
               WHERE id = $3 AND deleted_at IS NULL"#,
            table = kind.table_name(),
        );
        let result = sqlx::query(sqlx::AssertSqlSafe(update_sql.as_str()))
            .bind(now)
            .bind(deleted_by_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_by_context(
        &self,
        table_kind: FileTableKind,
        user: Option<i64>,
        namespace: Option<&str>,
        dk_scene: Option<i64>,
        dk_factor: Option<i64>,
        dk_function: Option<i64>,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<FileRecord>, i64), FileError> {
        // 显式 $N 占位符（sqlx 0.9 QueryBuilder 惰性 `?` 与 postgres $N append 冲突；
        // 与 fetch_file 同款模式：$USER 重复引用同一参数编号）
        let table = table_kind.table_name();
        let (where_sql, param_count) =
            Self::list_where_clause(user, namespace, dk_scene, dk_factor, dk_function);

        // total：同过滤条件计数（无 ORDER/LIMIT）
        let count_sql =
            format!("SELECT COUNT(*) FROM {table} f WHERE f.deleted_at IS NULL{where_sql}");
        let mut count_q = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.as_str()));
        if let Some(uid) = user {
            count_q = count_q.bind(uid);
        }
        if let Some(ns) = namespace {
            count_q = count_q.bind(format!("{ns}/%"));
        }
        for v in [dk_scene, dk_factor, dk_function].into_iter().flatten() {
            count_q = count_q.bind(v);
        }
        let total = count_q.fetch_one(&self.pool).await?;

        // 分页查询
        let (limit_idx, offset_idx) = (param_count + 1, param_count + 2);
        let list_sql = format!(
            "SELECT {fields} FROM {table} f WHERE f.deleted_at IS NULL{where_sql} \
             ORDER BY f.created_at DESC LIMIT ${limit_idx} OFFSET ${offset_idx}",
            fields = Self::FILE_FIELDS,
        );
        let mut q = sqlx::query_as::<_, FileRow>(sqlx::AssertSqlSafe(list_sql.as_str()));
        if let Some(uid) = user {
            q = q.bind(uid);
        }
        if let Some(ns) = namespace {
            q = q.bind(format!("{ns}/%"));
        }
        for v in [dk_scene, dk_factor, dk_function].into_iter().flatten() {
            q = q.bind(v);
        }
        q = q.bind(limit).bind(offset);

        let rows = q.fetch_all(&self.pool).await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            if let Some(rec) = self.decorate(Some(row), table_kind).await? {
                out.push(rec);
            }
        }
        Ok((out, total))
    }

    async fn resolve_storage_path(&self, id: i64) -> Result<Option<(String, String)>, FileError> {
        let row: Option<(String, String)> = sqlx::query_as(
            r#"SELECT COALESCE(u.scheme, 'local'), u.path
               FROM isahl."zc_id_file_rr_url" rr
               JOIN isahl."zc_id_stor-plc-url" s ON s.id = rr.ref_right AND s.deleted_at IS NULL
               JOIN isahl."zc_id_info-url" u ON u.id = s.fk_address AND u.deleted_at IS NULL
               WHERE rr.ref_left = $1 AND rr.deleted_at IS NULL
               ORDER BY rr.id DESC LIMIT 1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn resolve_chain(&self, id: i64) -> Result<Option<ResolvedChain>, FileError> {
        let row: Option<(String, String, Option<String>, i64, i64)> = sqlx::query_as(
            r#"SELECT u.scheme, u.path, u.notice, u.id, s.id
               FROM isahl."zc_id_file_rr_url" rr
               JOIN isahl."zc_id_stor-plc-url" s ON s.id = rr.ref_right AND s.deleted_at IS NULL
               JOIN isahl."zc_id_info-url" u ON u.id = s.fk_address AND u.deleted_at IS NULL
               WHERE rr.ref_left = $1 AND rr.deleted_at IS NULL
               ORDER BY rr.id DESC LIMIT 1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        let Some((scheme, path, notice, info_url_id, stor_url_id)) = row else {
            return Ok(None);
        };
        let kind = path
            .split('/')
            .nth(1)
            .and_then(FileTableKind::from_path_segment)
            .ok_or_else(|| FileError::Config(format!("无法从存储链 path 解析 kind: {path}")))?;
        let checksum = notice.and_then(|n| n.strip_prefix("sha256:").map(str::to_string));
        Ok(Some(ResolvedChain {
            scheme,
            path,
            kind,
            checksum,
            info_url_id,
            stor_url_id,
        }))
    }
}
