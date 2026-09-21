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

/// 叶表检索 SQL（编译期固化：表名内嵌为字面量）。
macro_rules! leaf_sql {
    ($t:literal) => {
        concat!(
            "SELECT id, code, notice, comments, fk_jurisdiction FROM isahl.\"",
            $t,
            "\" \
             WHERE deleted_at IS NULL \
               AND (notice ILIKE '%' || $1 || '%' OR comments ILIKE '%' || $1 || '%') \
             LIMIT $2"
        )
    };
}

/// scope → (叶表, 检索 SQL) 静态注册表（表名与 SQL 均为编译期字面量）。
const SCOPES: [(&str, &str, &str); 4] = [
    (
        "civil",
        "zc_id_law-civil-article",
        leaf_sql!("zc_id_law-civil-article"),
    ),
    (
        "common",
        "zc_id_law-common-section",
        leaf_sql!("zc_id_law-common-section"),
    ),
    (
        "common-holding",
        "zc_id_law-common-holding",
        leaf_sql!("zc_id_law-common-holding"),
    ),
    (
        "intl",
        "zc_id_law-intl-article",
        leaf_sql!("zc_id_law-intl-article"),
    ),
];

/// scope 列表 → (叶表, 检索 SQL)（未知 scope 忽略；空列表 = 全部登记叶表）。
fn resolve_tables(scopes: &[String]) -> Vec<(&'static str, &'static str)> {
    if scopes.is_empty() {
        return SCOPES.iter().map(|(_, t, sql)| (*t, *sql)).collect();
    }
    scopes
        .iter()
        .filter_map(|s| {
            SCOPES
                .iter()
                .find(|(name, _, _)| name == s)
                .map(|(_, t, sql)| (*t, *sql))
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
    'outer: for (table, sql) in &tables {
        // 表名与 SQL 均来自编译期静态注册表；keyword / limit 一律参数绑定。
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
            >(*sql)
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
