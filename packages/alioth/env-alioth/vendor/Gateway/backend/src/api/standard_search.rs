//! 技术标准检索 — 通用 EmpAgent 上下文增强
//!
//! POST /api/standard/search
//!   keywords:  ["适航", "审定"]
//!   scopes:    ["air-caac", "air-faa", "air-easa", "air-icao", "fin-cas", "fin-ifrs", "fin-gaap",
//!              "operation", "quality"]  (可选,空=全部)
//!   level:     "national"|"industry"|"enterprise"|"intl"  (可选)
//!   max_results: 5 (可选)
//!
//! 检索/排序/分页/转义单源 = `common::scoped_search`（本文件只剩注册表 + 适配映射）；
//! 关键词匹配 notice/comments，`level` 过滤经注册表声明的 `_t_` 分级列。

use actix_web::{web, HttpResponse};
use common::scoped_search::{search_scoped, ScopeTable, ScopedRequest};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Debug, Deserialize)]
pub struct StandardSearchReq {
    pub keywords: Vec<String>,
    pub max_results: Option<i32>,
    pub scopes: Option<Vec<String>>,
    pub level: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StandardHit {
    #[serde(with = "common::serde_zuid")]
    pub article_id: i64,
    pub article_code: Option<String>,
    pub article_title: Option<String>,
    pub article_body: Option<String>,
    pub source_table: String,
    pub level: Option<String>,
    pub issuer: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StandardSearchResp {
    pub hits: Vec<StandardHit>,
    pub total: usize,
}

/// scope → 叶表注册表（表名/列名均为编译期字面量；`_t_` 为分级列——`level` 过滤位）。
const SCOPES: &[ScopeTable] = &[
    ScopeTable {
        scope: "air-caac",
        table: "zc_id_stan-air-caac-article",
        columns: &["notice", "comments"],
        level_column: Some("_t_"),
    },
    ScopeTable {
        scope: "air-faa",
        table: "zc_id_stan-air-faa-article",
        columns: &["notice", "comments"],
        level_column: Some("_t_"),
    },
    ScopeTable {
        scope: "air-easa",
        table: "zc_id_stan-air-easa-article",
        columns: &["notice", "comments"],
        level_column: Some("_t_"),
    },
    ScopeTable {
        scope: "air-icao",
        table: "zc_id_stan-air-icao-article",
        columns: &["notice", "comments"],
        level_column: Some("_t_"),
    },
    ScopeTable {
        scope: "fin-cas",
        table: "zc_id_stan-fin-cas-article",
        columns: &["notice", "comments"],
        level_column: Some("_t_"),
    },
    ScopeTable {
        scope: "fin-ifrs",
        table: "zc_id_stan-fin-ifrs-article",
        columns: &["notice", "comments"],
        level_column: Some("_t_"),
    },
    ScopeTable {
        scope: "fin-gaap",
        table: "zc_id_stan-fin-gaap-article",
        columns: &["notice", "comments"],
        level_column: Some("_t_"),
    },
    ScopeTable {
        scope: "operation",
        table: "zc_id_stan-operation",
        columns: &["notice", "comments"],
        level_column: Some("_t_"),
    },
    ScopeTable {
        scope: "quality",
        table: "zc_id_stan-prod_quality",
        columns: &["notice", "comments"],
        level_column: Some("_t_"),
    },
];

pub async fn standard_search(
    pool: web::Data<PgPool>,
    req: web::Json<StandardSearchReq>,
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
            level: req.level.as_deref(),
            limit: max_results,
            offset: 0,
        },
    )
    .await
    .map_err(|e| actix_web::error::ErrorInternalServerError(format!("standard search: {}", e)))?;

    let hits: Vec<StandardHit> = page
        .hits
        .into_iter()
        .map(|hit| StandardHit {
            article_id: hit.id,
            article_code: hit.code,
            article_title: hit.title,
            article_body: hit.body,
            source_table: hit.source_table.to_string(),
            level: hit.level,
            // `issuer`（`_f_` 形态列）不在统一投影内；形态列由 dk_function 派生，当前库内为空
            issuer: None,
        })
        .collect();
    Ok(HttpResponse::Ok().json(StandardSearchResp {
        hits,
        total: page.total as usize,
    }))
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/standard").route("/search", web::post().to(standard_search)));
}
