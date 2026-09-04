//! 法律本体检索 — 通用 EmpAgent 上下文增强
//!
//! POST /api/legal/search
//!   keywords: ["承运", "赔偿"]
//!   scopes:   ["civil", "common", "intl"]  (可选,空=全部)
//!   max_results: 5 (可选)
//!
//! 按 scope 过滤 pg_inherits 叶表，keyword 匹配 notice/comments。

use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};

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

const ALL_LEAF_TABLES: [&str; 4] = [
    "zc_id_law-civil-article",
    "zc_id_law-common-section",
    "zc_id_law-common-holding",
    "zc_id_law-intl-article",
];

fn resolve_tables(scopes: &[String]) -> Vec<&'static str> {
    if scopes.is_empty() {
        return ALL_LEAF_TABLES.to_vec();
    }
    scopes
        .iter()
        .filter_map(|s| match s.as_str() {
            "civil" => Some("zc_id_law-civil-article"),
            "common" => Some("zc_id_law-common-section"),
            "common-holding" => Some("zc_id_law-common-holding"),
            "intl" => Some("zc_id_law-intl-article"),
            _ => None,
        })
        .collect()
}

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
    let tables = resolve_tables(scopes);

    let mut hits: Vec<LegalArticleHit> = Vec::new();
    'outer: for table in &tables {
        let sql = format!(
            "SELECT id, code, notice, comments, fk_jurisdiction FROM isahl.\"{}\" \
             WHERE deleted_at IS NULL AND (notice ILIKE '%' || $1 || '%' OR comments ILIKE '%' || $1 || '%') \
             LIMIT $2",
            table
        );
        for kw in &req.keywords {
            let rows = sqlx::query_as::<
                _,
                (
                    i64,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<i64>,
                ),
            >(AssertSqlSafe(sql.as_str()))
            .bind(kw)
            .bind(max_results)
            .fetch_all(pool.get_ref())
            .await
            .map_err(|e| {
                actix_web::error::ErrorInternalServerError(format!("legal search: {}", e))
            })?;

            for (id, code, notice, comments, jurisdiction) in rows {
                if hits.iter().any(|h| h.article_id == id) {
                    continue;
                }
                hits.push(LegalArticleHit {
                    article_id: id,
                    article_code: code,
                    article_title: notice,
                    article_body: comments,
                    source_table: table.to_string(),
                    jurisdiction: jurisdiction.map(|j| j.to_string()),
                });
                if hits.len() >= max_results as usize {
                    break 'outer;
                }
            }
        }
    }
    let total = hits.len();
    Ok(HttpResponse::Ok().json(LegalSearchResp { hits, total }))
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/legal").route("/search", web::post().to(legal_search)));
}
