//! 技术标准检索 — 通用 EmpAgent 上下文增强
//!
//! POST /api/standard/search
//!   keywords:  ["适航", "审定"]
//!   scopes:    ["air", "fin", "operation", "quality"]  (可选,空=全部)
//!   level:     "national"|"industry"|"enterprise"|"intl"  (可选)
//!   max_results: 5 (可选)
//!
//! 按 scope → 叶表, level → _t_ 过滤, keyword 匹配 notice/comments。

use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};

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

const ALL_LEAF_TABLES: [&str; 9] = [
    "zc_id_stan-air-caac-article",
    "zc_id_stan-air-faa-article",
    "zc_id_stan-air-easa-article",
    "zc_id_stan-air-icao-article",
    "zc_id_stan-fin-cas-article",
    "zc_id_stan-fin-ifrs-article",
    "zc_id_stan-fin-gaap-article",
    "zc_id_stan-operation",
    "zc_id_stan-prod_quality",
];

fn resolve_tables(scopes: &[String]) -> Vec<&'static str> {
    if scopes.is_empty() {
        return ALL_LEAF_TABLES.to_vec();
    }
    scopes
        .iter()
        .filter_map(|s| match s.as_str() {
            "air" | "air-caac" => Some("zc_id_stan-air-caac-article"),
            "air-faa" => Some("zc_id_stan-air-faa-article"),
            "air-easa" => Some("zc_id_stan-air-easa-article"),
            "air-icao" => Some("zc_id_stan-air-icao-article"),
            "fin" | "fin-cas" => Some("zc_id_stan-fin-cas-article"),
            "fin-ifrs" => Some("zc_id_stan-fin-ifrs-article"),
            "fin-gaap" => Some("zc_id_stan-fin-gaap-article"),
            "operation" => Some("zc_id_stan-operation"),
            "quality" => Some("zc_id_stan-prod_quality"),
            _ => None,
        })
        .collect()
}

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
    let tables = resolve_tables(scopes);
    let level_filter = req.level.as_deref();

    let mut hits: Vec<StandardHit> = Vec::new();
    'outer: for table in &tables {
        let sql = format!(
            "SELECT id, code, notice, comments, _t_, _f_ FROM isahl.\"{}\" \
             WHERE deleted_at IS NULL AND (notice ILIKE '%' || $1 || '%' OR comments ILIKE '%' || $1 || '%') \
             {} \
             LIMIT $2",
            table,
            if level_filter.is_some() { "AND _t_ = $3" } else { "" },
        );
        for kw in &req.keywords {
            let mut q = sqlx::query_as::<
                _,
                (
                    i64,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                ),
            >(AssertSqlSafe(sql.as_str()))
            .bind(kw)
            .bind(max_results);
            if let Some(level) = level_filter {
                q = q.bind(level);
            }
            let rows = q.fetch_all(pool.get_ref()).await.map_err(|e| {
                actix_web::error::ErrorInternalServerError(format!("standard search: {}", e))
            })?;

            for (id, code, notice, comments, level, issuer) in rows {
                if hits.iter().any(|h| h.article_id == id) {
                    continue;
                }
                hits.push(StandardHit {
                    article_id: id,
                    article_code: code,
                    article_title: notice,
                    article_body: comments,
                    source_table: table.to_string(),
                    level,
                    issuer,
                });
                if hits.len() >= max_results as usize {
                    break 'outer;
                }
            }
        }
    }
    let total = hits.len();
    Ok(HttpResponse::Ok().json(StandardSearchResp { hits, total }))
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/standard").route("/search", web::post().to(standard_search)));
}
