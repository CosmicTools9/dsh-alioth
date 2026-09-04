//! `/api/files` HTTP handlers。
//!
//! 安全（SECURITY_SPEC 对齐）：
//! - 10MB 上传上限、扩展名白名单、文件名净化（拒绝 `/`、`\`、`..`）
//! - kind↔扩展名 + 二进制 magic bytes 校验（HTTP 层 400；服务层兜底）
//! - 行级授权：`created_by_id = $user OR $user = ANY(ak_*)`（未授权 → 404，防枚举）
//! - namespace 隔离：`X-Namespace` 必填（`^[A-Z][a-zA-Z0-9-]*$`），下载/删除校验
//!   存储链 path 前缀与 header 一致（不一致 → 404），列表 `path LIKE '{ns}/%'` 过滤
//! - 上传限速 20 req/min/user（SECURITY_SPEC §4）：main.rs 挂
//!   `RateLimitMiddleware::per_user_any(["/api/files"], 20, 20/60)`（JWT `sub` 为 key，
//!   无 token 回退 IP）
//!
//! 下载协议扩展：
//! - ETag（`"sha256:{hex}"`）+ Last-Modified + If-None-Match/If-Modified-Since → 304
//! - 单区间 `Range: bytes=a-b` → 206 + Content-Range；不可满足 → 416（`bytes */{size}`）
//! - 多区间/畸形 Range → 忽略（RFC 7233 §3.1 允许）→ 200 全量；无 checksum 旧记录无 ETag

use actix_multipart::Multipart;
use actix_web::{web, HttpRequest, HttpResponse};
use common::error::AliothError;
use framework_file_manager::models::{FileTableKind, UpdateRequest, UploadRequest};
use framework_file_manager::service::{content_type_for, validate_magic};
use futures::StreamExt;
use serde::Deserialize;

use super::FilesState;

/// 上传大小上限（SECURITY_SPEC 对齐：10MB）
const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
/// 全局扩展名白名单（净化层；kind 级约束由 `FileTableKind::allowed_extensions` 判定）
const ALLOWED_EXT: [&str; 10] = [
    "pdf", "png", "jpg", "jpeg", "doc", "docx", "xls", "xlsx", "txt", "zip",
];

/// 提取并校验 `X-Namespace` header（必填，格式 `^[A-Z][a-zA-Z0-9-]*$`）。
fn extract_namespace(req: &HttpRequest) -> Result<String, AliothError> {
    // 批注 8df27a4f：下载（浏览器 <a> 直开）无 header——支持 ?namespace= 查询参数兜底
    let raw = req
        .headers()
        .get("X-Namespace")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .or_else(|| {
            req.query_string()
                .split('&')
                .find_map(|kv| kv.strip_prefix("namespace=").map(String::from))
        })
        .ok_or_else(|| AliothError::BadRequest("缺少 X-Namespace header".into()))?;
    let valid = !raw.is_empty()
        && raw
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false)
        && raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        return Err(AliothError::BadRequest(format!(
            "X-Namespace 格式非法（须 ^[A-Z][a-zA-Z0-9-]*$）: {raw}"
        )));
    }
    Ok(raw.to_string())
}

/// 文件名净化：拒绝路径分隔符与 `..` + 扩展名白名单。
fn sanitize_filename(name: &str) -> Result<String, AliothError> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.starts_with('.')
    {
        return Err(AliothError::BadRequest(
            "文件名非法（含路径分隔符或 ..）".into(),
        ));
    }
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    if !ALLOWED_EXT.contains(&ext.as_str()) {
        return Err(AliothError::BadRequest(format!("扩展名不在白名单: {ext}")));
    }
    Ok(name.to_string())
}

/// kind↔扩展名 + magic bytes 前置校验（HTTP 层 → 400；服务层另兜底）。
fn validate_kind_content(
    table_kind: FileTableKind,
    filename: &str,
    data: &[u8],
) -> Result<(), AliothError> {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    if !table_kind.allowed_extensions().contains(&ext.as_str()) {
        return Err(AliothError::BadRequest(format!(
            "扩展名 .{ext} 不属于 kind {:?}",
            table_kind
        )));
    }
    if !validate_magic(&ext, data) {
        return Err(AliothError::BadRequest(format!(
            "内容与 .{ext} 类型不符（magic bytes 校验失败）"
        )));
    }
    Ok(())
}

/// 列表查询参数（camelCase：tableKind/pageSize 与上传表单同风格）。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    page: Option<i64>,
    page_size: Option<i64>,
    table_kind: Option<String>,
    /// 本体维度过滤（dk_scene/dk_factor/dk_function）
    scene: Option<i64>,
    factor: Option<i64>,
    function: Option<i64>,
}

/// POST /api/files — 上传（multipart，字段 file + 可选 tableKind/ak*）
pub async fn upload_file(
    req: HttpRequest,
    state: web::Data<FilesState>,
    mut payload: Multipart,
) -> Result<HttpResponse, AliothError> {
    let user_id = common::context::extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("missing user context".into()))?;
    let namespace = extract_namespace(&req)?;

    // 表单字段提取
    let mut filename: Option<String> = None;
    let mut bytes: Vec<u8> = Vec::new();
    let mut table_kind = FileTableKind::Document;
    let mut dk_scene: Option<i64> = None;
    let mut ak_access: Option<Vec<i64>> = None;
    let mut ak_permit: Option<Vec<i64>> = None;
    let mut ak_benefit: Option<Vec<i64>> = None;

    while let Some(field_res) = payload.next().await {
        let mut field =
            field_res.map_err(|e| AliothError::BadRequest(format!("multipart 解析失败: {e}")))?;
        let cd = field.content_disposition();
        let field_name = cd.and_then(|c| c.get_name()).map(String::from);
        match field_name.as_deref() {
            Some("file") => {
                let fname = cd
                    .and_then(|c| c.get_filename())
                    .map(String::from)
                    .ok_or_else(|| AliothError::BadRequest("file 字段缺文件名".into()))?;
                filename = Some(sanitize_filename(&fname)?);
                while let Some(chunk) = field.next().await {
                    let data = chunk
                        .map_err(|e| AliothError::BadRequest(format!("读取上传流失败: {e}")))?;
                    if bytes.len() + data.len() > MAX_UPLOAD_BYTES {
                        return Err(AliothError::BadRequest("文件超过 10MB 上限".into()));
                    }
                    bytes.extend_from_slice(&data);
                }
            }
            Some("tableKind") => {
                let v = read_text_field(&mut field).await?;
                table_kind = FileTableKind::from_path_segment(&v)
                    .ok_or_else(|| AliothError::BadRequest(format!("tableKind 非法: {v}")))?;
            }
            Some("scene") => {
                let v = read_text_field(&mut field).await?;
                dk_scene = v.trim().parse::<i64>().ok().or_else(|| {
                    // 兼容逗号分隔单值（复用 parse_id_list 语义）
                    parse_id_list(&v).and_then(
                        |mut ids| {
                            if ids.len() == 1 {
                                ids.pop()
                            } else {
                                None
                            }
                        },
                    )
                });
            }
            Some("akAccessUsers") => ak_access = parse_id_list(&read_text_field(&mut field).await?),
            Some("akPermitUsers") => ak_permit = parse_id_list(&read_text_field(&mut field).await?),
            Some("akBenefitUsers") => {
                ak_benefit = parse_id_list(&read_text_field(&mut field).await?)
            }
            _ => {} // 忽略未知字段
        }
    }

    let filename = filename.ok_or_else(|| AliothError::BadRequest("缺少 file 字段".into()))?;
    if bytes.is_empty() {
        return Err(AliothError::BadRequest("空文件".into()));
    }
    // kind↔扩展名 + magic bytes → 400
    validate_kind_content(table_kind, &filename, &bytes)?;
    let content_type = content_type_for(&filename).to_string();

    let record = state
        .service
        .upload(
            &namespace,
            UploadRequest {
                filename,
                content_type,
                data: bytes,
                table_kind,
                notice: None,
                code: None,
                dk_scene,
                dk_factor: None,
                dk_function: None,
                ck_category: None,
                ak_benefit_user: ak_benefit,
                ak_permit_user: ak_permit,
                ak_access_user: ak_access,
                created_by_id: Some(user_id),
            },
        )
        .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "id": record.id.to_string(),
        "namespace": namespace,
        "fileName": record.filename,
        "fileSize": record.size,
        "checksum": record.checksum,
        "url": record.url,
    })))
}

/// GET /api/files/{id} — 下载（行级授权 + namespace 链校验 + 条件请求 + Range）
pub async fn download_file(
    req: HttpRequest,
    state: web::Data<FilesState>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let user_id = common::context::extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("missing user context".into()))?;
    let namespace = extract_namespace(&req)?;
    let file_id = path.into_inner();

    // 行级授权：未授权/不存在 → 404（防枚举）
    let meta = state
        .service
        .get_metadata(file_id, Some(user_id))
        .await?
        .ok_or_else(|| AliothError::NotFound(format!("文件不存在: {file_id}")))?;

    // namespace 链校验：存储键前缀必须与 header 一致
    verify_namespace(&namespace, meta.storage_path.as_deref())?;

    let etag = meta.checksum.as_ref().map(|c| format!("\"sha256:{c}\""));
    let last_modified = meta
        .created_at
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    let size = meta.size;

    // 条件请求（If-None-Match 优先于 If-Modified-Since）
    if let Some(etag) = &etag {
        if let Some(inm) = req
            .headers()
            .get(actix_web::http::header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
        {
            if inm.trim() == etag || inm.trim() == "*" {
                return Ok(HttpResponse::NotModified()
                    .insert_header((actix_web::http::header::ETAG, etag.clone()))
                    .insert_header((
                        actix_web::http::header::LAST_MODIFIED,
                        last_modified.clone(),
                    ))
                    .finish());
            }
        }
    }
    if let Some(ims) = req
        .headers()
        .get(actix_web::http::header::IF_MODIFIED_SINCE)
        .and_then(|v| v.to_str().ok())
    {
        if let Ok(since) = chrono::DateTime::parse_from_rfc2822(ims.trim()) {
            if meta.created_at.timestamp() <= since.timestamp() {
                return Ok(HttpResponse::NotModified()
                    .insert_header((
                        actix_web::http::header::LAST_MODIFIED,
                        last_modified.clone(),
                    ))
                    .finish());
            }
        }
    }

    // Range（单区间；畸形/多区间忽略 → 200；不可满足 → 416）
    let total = size.unwrap_or(0) as u64;
    let range_header = req
        .headers()
        .get(actix_web::http::header::RANGE)
        .and_then(|v| v.to_str().ok());
    let range = match (range_header, total) {
        (Some(h), sz) if sz > 0 => parse_range(h, sz),
        _ => RangeDecision::Ignore,
    };
    match range {
        RangeDecision::Valid(start, end) => {
            let result = state.service.download_range(file_id, start, end).await?;
            let mut resp = HttpResponse::PartialContent();
            resp.content_type(result.content_type);
            resp.insert_header((
                actix_web::http::header::CONTENT_RANGE,
                format!("bytes {start}-{end}/{total}"),
            ));
            resp.insert_header((
                actix_web::http::header::CONTENT_LENGTH,
                (end - start + 1).to_string(),
            ));
            resp.insert_header((
                actix_web::http::header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename*=UTF-8''{}",
                    rfc5987_encode(&result.filename)
                ),
            ));
            if let Some(e) = &etag {
                resp.insert_header((actix_web::http::header::ETAG, e.clone()));
            }
            if let Some(cs) = meta.checksum.as_deref() {
                resp.insert_header(("X-Checksum-Sha256", cs));
            }
            Ok(resp.body(result.data))
        }
        RangeDecision::Unsatisfiable => Ok(HttpResponse::RangeNotSatisfiable()
            .insert_header((
                actix_web::http::header::CONTENT_RANGE,
                format!("bytes */{total}"),
            ))
            .finish()),
        RangeDecision::Ignore => {
            let result = state.service.download(file_id).await?;
            let mut resp = HttpResponse::Ok();
            resp.content_type(result.content_type);
            resp.insert_header((
                actix_web::http::header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename*=UTF-8''{}",
                    rfc5987_encode(&result.filename)
                ),
            ));
            resp.insert_header((
                actix_web::http::header::LAST_MODIFIED,
                last_modified.clone(),
            ));
            if let Some(e) = &etag {
                resp.insert_header((actix_web::http::header::ETAG, e.clone()));
            }
            if let Some(cs) = meta.checksum.as_deref() {
                resp.insert_header(("X-Checksum-Sha256", cs));
            }
            Ok(resp.body(result.data))
        }
    }
}

/// GET /api/files/{id}/presigned — 直连下载 URL（S3/OSS）；local → 代理 URL
pub async fn presigned_file(
    req: HttpRequest,
    state: web::Data<FilesState>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let user_id = common::context::extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("missing user context".into()))?;
    let namespace = extract_namespace(&req)?;
    let file_id = path.into_inner();

    let meta = state
        .service
        .get_metadata(file_id, Some(user_id))
        .await?
        .ok_or_else(|| AliothError::NotFound(format!("文件不存在: {file_id}")))?;
    verify_namespace(&namespace, meta.storage_path.as_deref())?;

    const EXPIRES: u64 = 3600;
    match state.service.presigned_url(file_id, EXPIRES).await? {
        Some(url) => Ok(HttpResponse::Ok().json(serde_json::json!({
            "url": url,
            "expiresIn": EXPIRES,
            "proxy": false,
        }))),
        None => Ok(HttpResponse::Ok().json(serde_json::json!({
            "url": format!("/api/files/{file_id}"),
            "expiresIn": 0,
            "proxy": true,
        }))),
    }
}

/// PUT /api/files/{id} — 更新（multipart，字段全部可选）：
/// file（替换字节）/ fileName（改名）/ akAccessUsers|akPermitUsers|akBenefitUsers
pub async fn update_file(
    req: HttpRequest,
    state: web::Data<FilesState>,
    path: web::Path<i64>,
    mut payload: Multipart,
) -> Result<HttpResponse, AliothError> {
    let user_id = common::context::extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("missing user context".into()))?;
    let namespace = extract_namespace(&req)?;
    let file_id = path.into_inner();

    // 行级授权 + namespace 链校验（与 delete 同口径）
    let meta = state
        .service
        .get_metadata(file_id, Some(user_id))
        .await?
        .ok_or_else(|| AliothError::NotFound(format!("文件不存在: {file_id}")))?;
    verify_namespace(&namespace, meta.storage_path.as_deref())?;

    let mut filename: Option<String> = None;
    let mut bytes: Option<Vec<u8>> = None;
    let mut ak_access: Option<Vec<i64>> = None;
    let mut ak_permit: Option<Vec<i64>> = None;
    let mut ak_benefit: Option<Vec<i64>> = None;

    while let Some(field_res) = payload.next().await {
        let mut field =
            field_res.map_err(|e| AliothError::BadRequest(format!("multipart 解析失败: {e}")))?;
        let cd = field.content_disposition();
        let field_name = cd.and_then(|c| c.get_name()).map(String::from);
        match field_name.as_deref() {
            Some("file") => {
                let fname = cd
                    .and_then(|c| c.get_filename())
                    .map(String::from)
                    .ok_or_else(|| AliothError::BadRequest("file 字段缺文件名".into()))?;
                filename = Some(sanitize_filename(&fname)?);
                let mut buf: Vec<u8> = Vec::new();
                while let Some(chunk) = field.next().await {
                    let data = chunk
                        .map_err(|e| AliothError::BadRequest(format!("读取上传流失败: {e}")))?;
                    if buf.len() + data.len() > MAX_UPLOAD_BYTES {
                        return Err(AliothError::BadRequest("文件超过 10MB 上限".into()));
                    }
                    buf.extend_from_slice(&data);
                }
                bytes = Some(buf);
            }
            Some("fileName") => {
                let v = read_text_field(&mut field).await?;
                if !v.is_empty() {
                    filename = Some(sanitize_filename(&v)?);
                }
            }
            Some("akAccessUsers") => ak_access = parse_id_list(&read_text_field(&mut field).await?),
            Some("akPermitUsers") => ak_permit = parse_id_list(&read_text_field(&mut field).await?),
            Some("akBenefitUsers") => {
                ak_benefit = parse_id_list(&read_text_field(&mut field).await?)
            }
            _ => {} // 忽略未知字段
        }
    }

    let has_any = filename.is_some()
        || bytes.is_some()
        || ak_access.is_some()
        || ak_permit.is_some()
        || ak_benefit.is_some();
    if !has_any {
        return Err(AliothError::BadRequest(
            "无可更新字段（file/fileName/ak* 至少一项）".into(),
        ));
    }
    if let Some(data) = &bytes {
        if data.is_empty() {
            return Err(AliothError::BadRequest("空文件".into()));
        }
        // kind↔扩展名 + magic bytes（按最终文件名；fileName 未给时沿用现名）
        let final_name = filename
            .clone()
            .unwrap_or_else(|| meta.filename.clone().unwrap_or_default());
        let kind = FileTableKind::from_path_segment(
            meta.storage_path
                .as_deref()
                .and_then(|p| p.split('/').nth(1))
                .unwrap_or("document"),
        )
        .unwrap_or(FileTableKind::Document);
        validate_kind_content(kind, &final_name, data)?;
    }

    let record = state
        .service
        .update(
            file_id,
            UpdateRequest {
                filename,
                data: bytes,
                ak_benefit_user: ak_benefit,
                ak_permit_user: ak_permit,
                ak_access_user: ak_access,
            },
        )
        .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "id": record.id.to_string(),
        "fileName": record.filename,
        "fileSize": record.size,
        "checksum": record.checksum,
        "url": record.url,
    })))
}

/// GET /api/files — 列表（page/page_size/tableKind/scene/factor/function + namespace 隔离）
pub async fn list_files(
    req: HttpRequest,
    state: web::Data<FilesState>,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, AliothError> {
    let user_id = common::context::extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("missing user context".into()))?;
    let namespace = extract_namespace(&req)?;

    let table_kind = match query.table_kind.as_deref() {
        None | Some("document") => FileTableKind::Document,
        Some(k) => FileTableKind::from_path_segment(k)
            .ok_or_else(|| AliothError::BadRequest(format!("tableKind 非法: {k}")))?,
    };
    let page = query.page.unwrap_or(1);
    let page_size = query.page_size.unwrap_or(20);

    let (records, total) = state
        .service
        .list_by_context(
            table_kind,
            Some(user_id),
            Some(&namespace),
            query.scene,
            query.factor,
            query.function,
            page,
            page_size,
        )
        .await?;

    let items: Vec<serde_json::Value> = records
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id.to_string(),
                "fileName": r.filename,
                "fileSize": r.size,
                "code": r.code,
                "createdAt": r.created_at,
                "checksum": r.checksum,
                "url": r.url,
            })
        })
        .collect();

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "items": items,
        "page": page,
        "pageSize": page_size,
        "total": total,
    })))
}

/// DELETE /api/files/{id} — 软删 + 磁盘清理（best effort）
pub async fn delete_file(
    req: HttpRequest,
    state: web::Data<FilesState>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let user_id = common::context::extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("missing user context".into()))?;
    let namespace = extract_namespace(&req)?;
    let file_id = path.into_inner();

    let meta = state
        .service
        .get_metadata(file_id, Some(user_id))
        .await?
        .ok_or_else(|| AliothError::NotFound(format!("文件不存在: {file_id}")))?;
    verify_namespace(&namespace, meta.storage_path.as_deref())?;

    state.service.delete(file_id, Some(user_id)).await?;
    Ok(HttpResponse::Ok().json(serde_json::json!({ "deleted": true, "id": file_id.to_string() })))
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/files")
            .route("", web::post().to(upload_file))
            .route("", web::get().to(list_files))
            .route("/{id}", web::get().to(download_file))
            .route("/{id}", web::put().to(update_file))
            .route("/{id}", web::delete().to(delete_file))
            .route("/{id}/presigned", web::get().to(presigned_file)),
    );
}

/// namespace 一致性：存储键前缀必须为 `{ns}/`。
fn verify_namespace(namespace: &str, storage_path: Option<&str>) -> Result<(), AliothError> {
    let prefix = format!("{namespace}/");
    match storage_path {
        Some(p) if p.starts_with(&prefix) => Ok(()),
        // 链缺失或前缀不符 → 404（防跨 namespace 枚举）
        _ => Err(AliothError::NotFound("文件不存在".into())),
    }
}

/// 读取普通文本 form 字段值。
async fn read_text_field(field: &mut actix_multipart::Field) -> Result<String, AliothError> {
    let mut buf = String::new();
    while let Some(chunk) = field.next().await {
        let data = chunk.map_err(|e| AliothError::BadRequest(format!("读取表单字段失败: {e}")))?;
        buf.push_str(&String::from_utf8_lossy(&data));
    }
    Ok(buf.trim().to_string())
}

/// 逗号分隔 id 列表解析。
fn parse_id_list(raw: &str) -> Option<Vec<i64>> {
    if raw.is_empty() {
        return None;
    }
    let ids: Vec<i64> = raw
        .split(',')
        .filter_map(|s| s.trim().parse::<i64>().ok())
        .collect();
    if ids.is_empty() {
        None
    } else {
        Some(ids)
    }
}

/// RFC 5987 文件名编码（非 ASCII 与保留字百分号转义，供 Content-Disposition）。
fn rfc5987_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// 单区间 Range 解析结果。
enum RangeDecision {
    /// 无 Range / 畸形 / 多区间 → 忽略（200 全量）
    Ignore,
    /// 满足的区间（闭区间 [start, end]）
    Valid(u64, u64),
    /// 区间不可满足（start >= size 等）→ 416
    Unsatisfiable,
}

/// 解析单区间 `Range: bytes=a-b` / `bytes=a-` / `bytes=-n`。
/// 手写解析（NO_REGEX 合规）：多区间或格式非法 → Ignore（RFC 7233 §3.1 允许忽略）；
/// 仅当语义上不可满足（起点越界/空后缀）→ Unsatisfiable。
fn parse_range(header: &str, size: u64) -> RangeDecision {
    let Some(rest) = header.trim().strip_prefix("bytes=") else {
        return RangeDecision::Ignore;
    };
    if rest.contains(',') {
        return RangeDecision::Ignore; // 多区间不支持 → 忽略
    }
    let (a, b) = match rest.split_once('-') {
        Some(x) => x,
        None => return RangeDecision::Ignore,
    };
    let (a, b) = (a.trim(), b.trim());
    match (a.is_empty(), b.is_empty()) {
        // 后缀区间 bytes=-n：最后 n 字节
        (true, false) => match b.parse::<u64>() {
            Ok(0) => RangeDecision::Unsatisfiable,
            Ok(n) => {
                let start = size.saturating_sub(n);
                RangeDecision::Valid(start, size - 1)
            }
            Err(_) => RangeDecision::Ignore,
        },
        // bytes=a-：到文件尾
        (false, true) => match a.parse::<u64>() {
            Ok(s) if s < size => RangeDecision::Valid(s, size - 1),
            Ok(_) => RangeDecision::Unsatisfiable,
            Err(_) => RangeDecision::Ignore,
        },
        // bytes=a-b
        (false, false) => {
            let (Ok(s), Ok(e)) = (a.parse::<u64>(), b.parse::<u64>()) else {
                return RangeDecision::Ignore;
            };
            if s >= size || s > e {
                RangeDecision::Unsatisfiable
            } else {
                RangeDecision::Valid(s, e.min(size - 1))
            }
        }
        (true, true) => RangeDecision::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_path_traversal() {
        // 路径分隔符 / 与 \、..、点开头、空名、白名单外扩展名全部拒绝
        for bad in [
            "../../etc/passwd",
            "a/b.txt",
            "a\\b.txt",
            "..hidden.pdf",
            "script.exe",
            "",
        ] {
            assert!(sanitize_filename(bad).is_err(), "应拒绝: {bad}");
        }
        // 合法文件名放行（含 zip）
        assert!(sanitize_filename("合同.pdf").is_ok());
        assert!(sanitize_filename("photo.png").is_ok());
        assert!(sanitize_filename("bundle.zip").is_ok());
    }

    #[test]
    fn parse_id_list_handles_empty_and_invalid() {
        assert_eq!(parse_id_list(""), None);
        assert_eq!(parse_id_list("abc,,"), None);
        assert_eq!(parse_id_list("1,2,3"), Some(vec![1, 2, 3]));
        assert_eq!(parse_id_list("1, x, 3"), Some(vec![1, 3]));
    }

    #[test]
    fn rfc5987_encodes_non_ascii() {
        assert_eq!(rfc5987_encode("合同.pdf"), "%E5%90%88%E5%90%8C.pdf");
        assert_eq!(rfc5987_encode("plain-name.txt"), "plain-name.txt");
    }

    #[test]
    fn namespace_format_validation() {
        // 合法：大写字母开头，后续字母数字或 -
        for ok in ["Alioth", "WZ", "AVIC-CAASEC", "A1"] {
            let req = test_request_with_namespace(ok);
            assert_eq!(extract_namespace(&req).unwrap(), ok);
        }
        // 非法：空、小写开头、非法字符
        for bad in ["", "alioth", "WZ!", "1A", "A B"] {
            let req = test_request_with_namespace(bad);
            assert!(extract_namespace(&req).is_err(), "应拒绝: {bad}");
        }
    }

    fn test_request_with_namespace(ns: &str) -> actix_web::HttpRequest {
        use actix_web::test::TestRequest;
        let mut r = TestRequest::default();
        if !ns.is_empty() {
            r = r.insert_header(("X-Namespace", ns));
        }
        r.to_http_request()
    }

    // ── Range 解析 ────────────────────────────────────────────────────────

    #[test]
    fn range_parse_basic() {
        assert!(matches!(
            parse_range("bytes=0-3", 100),
            RangeDecision::Valid(0, 3)
        ));
        assert!(matches!(
            parse_range("bytes=10-19", 100),
            RangeDecision::Valid(10, 19)
        ));
        // 越界 end 裁剪到 size-1
        assert!(matches!(
            parse_range("bytes=90-200", 100),
            RangeDecision::Valid(90, 99)
        ));
    }

    #[test]
    fn range_parse_open_and_suffix() {
        assert!(matches!(
            parse_range("bytes=10-", 100),
            RangeDecision::Valid(10, 99)
        ));
        assert!(matches!(
            parse_range("bytes=-10", 100),
            RangeDecision::Valid(90, 99)
        ));
        // 后缀 n > size → 全量
        assert!(matches!(
            parse_range("bytes=-500", 100),
            RangeDecision::Valid(0, 99)
        ));
        // 后缀 n == 0 → 不可满足
        assert!(matches!(
            parse_range("bytes=-0", 100),
            RangeDecision::Unsatisfiable
        ));
    }

    #[test]
    fn range_parse_unsatisfiable() {
        assert!(matches!(
            parse_range("bytes=100-200", 100),
            RangeDecision::Unsatisfiable
        ));
        assert!(matches!(
            parse_range("bytes=50-10", 100),
            RangeDecision::Unsatisfiable
        ));
        assert!(matches!(
            parse_range("bytes=100-", 100),
            RangeDecision::Unsatisfiable
        ));
    }

    #[test]
    fn range_parse_ignored_when_malformed() {
        for bad in [
            "bytes=0-3,5-9", // 多区间
            "bytes=abc",
            "chunked=0-3", // 非 bytes 单位
            "bytes=",
            "bytes=-",
            "0-3", // 缺 bytes= 前缀
            "",
        ] {
            assert!(
                matches!(parse_range(bad, 100), RangeDecision::Ignore),
                "应忽略: {bad:?}"
            );
        }
        // 空文件（size=0）：任何区间不可满足（调用方在 size>0 时才解析）
        assert!(matches!(
            parse_range("bytes=0-3", 0),
            RangeDecision::Unsatisfiable
        ));
    }

    #[test]
    fn kind_content_validation() {
        // image 表：png/jpg 放行；docx 拒绝；png 内容为文本拒绝（magic）
        let png: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
        assert!(validate_kind_content(FileTableKind::Image, "a.png", png).is_ok());
        assert!(validate_kind_content(FileTableKind::Image, "a.docx", png).is_err());
        assert!(validate_kind_content(FileTableKind::Image, "a.png", b"plain text").is_err());
        // document 表：png 内容放行；package：zip 内容放行、png 拒绝
        assert!(validate_kind_content(FileTableKind::Document, "a.png", png).is_ok());
        let zip: &[u8] = b"PK\x03\x04rest";
        assert!(validate_kind_content(FileTableKind::Package, "a.zip", zip).is_ok());
        assert!(validate_kind_content(FileTableKind::Package, "a.zip", b"not zip").is_err());
    }
}
