//! 法律本体检索 — 通用 EmpAgent 上下文增强
//!
//! POST /api/legal/search
//!   keywords: ["承运", "赔偿"]
//!   scopes:   ["civil", "common", "common-holding", "intl"]  (可选,空=全部)
//!   max_results: 5 (可选)
//!
//! 检索/排序/分页/转义单源 = `common::scoped_search`（本文件只剩注册表 + 适配映射）；
//! 关键词匹配 notice/comments（含扩展词 OR 并集），`jurisdiction` 由附加列投影回填。

use actix_web::{web, HttpResponse};
use common::scoped_search::{search_scoped, ScopeTable, ScopedRequest};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Debug, Deserialize)]
pub struct LegalSearchReq {
    pub keywords: Vec<String>,
    pub max_results: Option<i32>,
    pub scopes: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct LegalArticleHit {
    #[serde(with = "common::serde_zuid")]
    pub article_id: i64,
    pub article_code: Option<String>,
    pub article_title: Option<String>,
    pub article_body: Option<String>,
    pub source_table: String,
    pub jurisdiction: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LegalSearchResp {
    pub hits: Vec<LegalArticleHit>,
    pub total: usize,
}

/// scope → 叶表注册表（表名/列名均为编译期字面量）。
/// `fk_jurisdiction` 为附加列（本端点无 level 过滤语义，仅投影回 `jurisdiction`）。
const SCOPES: &[ScopeTable] = &[
    ScopeTable {
        scope: "civil",
        table: "zc_id_law-civil-article",
        columns: &["notice", "comments"],
        level_column: Some("fk_jurisdiction"),
    },
    ScopeTable {
        scope: "common",
        table: "zc_id_law-common-section",
        columns: &["notice", "comments"],
        level_column: Some("fk_jurisdiction"),
    },
    ScopeTable {
        scope: "common-holding",
        table: "zc_id_law-common-holding",
        columns: &["notice", "comments"],
        level_column: Some("fk_jurisdiction"),
    },
    ScopeTable {
        scope: "intl",
        table: "zc_id_law-intl-article",
        columns: &["notice", "comments"],
        level_column: Some("fk_jurisdiction"),
    },
];

pub async fn legal_search(
    pool: web::Data<PgPool>,
    req: web::Json<LegalSearchReq>,
) -> Result<HttpResponse, actix_web::Error> {
    if req.keywords.is_empty() {
        return Ok(
            HttpResponse::BadRequest().json(serde_json::json!({"error":"keywords required"}))
        );
    }
    let max_results = req.max_results.unwrap_or(5).clamp(1, 20) as i64;
    let scopes = req.scopes.as_deref().unwrap_or(&[]);
    let page = search_scoped(
        pool.get_ref(),
        SCOPES,
        &ScopedRequest {
            keywords: req.keywords.as_slice(),
            scopes,
            level: None,
            limit: max_results,
            offset: 0,
        },
    )
    .await
    .map_err(|e| actix_web::error::ErrorInternalServerError(format!("legal search: {}", e)))?;

    let hits: Vec<LegalArticleHit> = page
        .hits
        .into_iter()
        .map(|hit| LegalArticleHit {
            article_id: hit.id,
            article_code: hit.code,
            article_title: hit.title,
            article_body: hit.body,
            source_table: hit.source_table.to_string(),
            jurisdiction: hit.level,
        })
        .collect();
    Ok(HttpResponse::Ok().json(LegalSearchResp {
        hits,
        total: page.total as usize,
    }))
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/legal").route("/search", web::post().to(legal_search)));
}
