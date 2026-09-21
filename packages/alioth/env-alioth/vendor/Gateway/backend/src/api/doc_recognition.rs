//! 文档识别服务端点（doc-recognition，Framework 基础能力的 Gateway 接入面）。
//!
//! 用户裁决（2026-09-16）：**OA 等外部门户不接大模型**——识别/转换/类型提取能力
//! 嵌合于 Framework（`doc-recognition` crate），LLM 接入点复用 Gateway：
//! 能力逻辑在 crate（收注入的 `LlmService`，配置源无关——Meta 多模态可同源消费），
//! 本模块只做 HTTP 组合：`X-Service-Key` 服务身份（fail-closed，先例
//! `transport-operations::carrier_portal::verify_service_key`）→ base64 文件 →
//! 转换（PDF 文本层）→ 类别白名单（`zc_id_cate-certification` 活动行——企业类目
//! 落库后自动纳入）→ LLM 结构化（`DbLlmConfigAdapter` 解析 `LlmService`：DB 优先 env 兜底）。
//!
//! 消费方：OA 门户证照制式分析（`POST /api/external/subject/mine/certificates/analyze`
//! 转调本端点）。

use actix_web::{web, HttpRequest, HttpResponse};
use base64::Engine;
use log::{error, warn};
use serde::Deserialize;
use sqlx::PgPool;

use crate::api::chat_sessions::adapters::db_llm_config::DbLlmConfigAdapter;
use crate::api::chat_sessions::ports::LlmConfigPort;

/// base64 载荷上限：解后 10MB → 编码后约 13.4MB，取 16MB（同 OA documents 上限先例量级）。
const MAX_B64_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct AnalyzeRequest {
    /// 证书 PDF 文件内容（base64 标准编码）
    pub file_base64: String,
}

/// 共享密钥校验（fail-closed，照 `transport-operations::carrier_portal` 同形）。
fn verify_service_key(req: &HttpRequest) -> Result<(), HttpResponse> {
    let expected = std::env::var("PLATFORM_SERVICE_KEY")
        .ok()
        .filter(|k| !k.is_empty());
    let Some(expected) = expected else {
        return Err(HttpResponse::Unauthorized().json(serde_json::json!({
            "code": "SERVICE_KEY_UNCONFIGURED",
            "message": "PLATFORM_SERVICE_KEY 未配置——识别服务拒绝（fail-closed）",
        })));
    };
    let provided = req
        .headers()
        .get("x-service-key")
        .and_then(|v| v.to_str().ok());
    match provided {
        Some(k) if k == expected => Ok(()),
        _ => Err(HttpResponse::Unauthorized().json(
            serde_json::json!({ "code": "INVALID_SERVICE_KEY", "message": "Invalid service key" }),
        )),
    }
}

/// POST /api/service/doc-recognition/analyze — 证照制式分析（服务间；不落库）。
async fn analyze(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<AnalyzeRequest>,
) -> HttpResponse {
    if let Err(resp) = verify_service_key(&req) {
        return resp;
    }
    if body.file_base64.len() > MAX_B64_BYTES {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "code": "TOO_LARGE", "message": "文件超过 10MB 上限" }));
    }
    let bytes = match base64::engine::general_purpose::STANDARD.decode(body.file_base64.trim()) {
        Ok(b) => b,
        Err(e) => return HttpResponse::BadRequest().json(
            serde_json::json!({ "code": "BAD_BASE64", "message": format!("base64 解码失败: {e}") }),
        ),
    };
    if bytes.is_empty() {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "code": "EMPTY_FILE", "message": "文件内容为空" }));
    }

    // 转换：PDF → 文本层（扫描件无文本层显式 400）
    let (text, truncated) = match doc_recognition::extract_pdf_text(&bytes) {
        Ok(out) => out,
        Err(e) => {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({ "code": "NO_TEXT_LAYER", "message": e.to_string() }))
        }
    };

    // 类别白名单（活动行；企业类目 A3② 落库后自动纳入）
    let whitelist: Vec<(String, String)> = match sqlx::query_as(
        r#"SELECT code, notice FROM isahl."zc_id_cate-certification"
           WHERE deleted_at IS NULL AND code IS NOT NULL ORDER BY code"#,
    )
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            error!("识别白名单查询失败: {e}");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "code": "DB_ERROR" }));
        }
    };

    // LLM 接入点复用 Gateway：DB 优先 env 兜底解析 LlmService
    let adapter = DbLlmConfigAdapter::new(pool.get_ref().clone());
    let service = match adapter.load_service().await {
        Ok(s) => s,
        Err(msg) => {
            warn!("识别服务 LLM 配置不可用: {msg}");
            return HttpResponse::ServiceUnavailable().json(serde_json::json!({
                "code": "LLM_NOT_CONFIGURED",
                "message": format!("LLM 配置不可用：{msg}"),
            }));
        }
    };

    match doc_recognition::structure_certificate(&service, &text, &whitelist).await {
        Ok(draft) => HttpResponse::Ok().json(serde_json::json!({
            "draft": draft,
            "warning": truncated.then(|| "证书文本过长已截断，请核对识别结果".to_string()),
        })),
        Err(e) => {
            error!("证书制式分析失败: {e}");
            HttpResponse::BadGateway().json(serde_json::json!({
                "code": "LLM_FAILED",
                "message": e.to_string(),
            }))
        }
    }
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/service/doc-recognition")
            // base64 载荷放宽（actix 默认 2MB 会拒收常见扫描件；仅此子域生效）
            .app_data(web::JsonConfig::default().limit(MAX_B64_BYTES))
            .service(web::resource("/analyze").route(web::post().to(analyze))),
    );
}
