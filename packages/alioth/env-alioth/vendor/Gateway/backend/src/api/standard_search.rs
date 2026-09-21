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

/// 叶表检索 SQL（编译期固化：表名内嵌为字面量；`$pred` 只承载 `_t_` 谓词文本，不是表名）。
macro_rules! leaf_sql {
    ($t:literal, $pred:literal) => {
        concat!(
            "SELECT id, code, notice, comments, _t_, _f_ FROM isahl.\"",
            $t,
            "\" \
             WHERE deleted_at IS NULL \
               AND (notice ILIKE '%' || $1 || '%' OR comments ILIKE '%' || $1 || '%') ",
            $pred,
            " \
             LIMIT $2"
        )
    };
}

/// 叶表静态规格：表名与两种 `level` 形态的 SQL 全部编译期固化。
/// `sql` 不绑 `$3`；`sql_with_level` 多一个 `_t_ = $3` 谓词（`$1` keyword、`$2` limit 序号不变）。
struct LeafSql {
    table: &'static str,
    sql: &'static str,
    sql_with_level: &'static str,
}

macro_rules! leaf {
    ($t:literal) => {
        LeafSql {
            table: $t,
            sql: leaf_sql!($t, ""),
            sql_with_level: leaf_sql!($t, "AND _t_ = $3"),
        }
    };
}

static LEAF_AIR_CAAC: LeafSql = leaf!("zc_id_stan-air-caac-article");
static LEAF_AIR_FAA: LeafSql = leaf!("zc_id_stan-air-faa-article");
static LEAF_AIR_EASA: LeafSql = leaf!("zc_id_stan-air-easa-article");
static LEAF_AIR_ICAO: LeafSql = leaf!("zc_id_stan-air-icao-article");
static LEAF_FIN_CAS: LeafSql = leaf!("zc_id_stan-fin-cas-article");
static LEAF_FIN_IFRS: LeafSql = leaf!("zc_id_stan-fin-ifrs-article");
static LEAF_FIN_GAAP: LeafSql = leaf!("zc_id_stan-fin-gaap-article");
static LEAF_OPERATION: LeafSql = leaf!("zc_id_stan-operation");
static LEAF_QUALITY: LeafSql = leaf!("zc_id_stan-prod_quality");

/// 全部叶表（空 scopes = 全检；顺序与原常量数组一致）。
static ALL_LEAF_TABLES: [&LeafSql; 9] = [
    &LEAF_AIR_CAAC,
    &LEAF_AIR_FAA,
    &LEAF_AIR_EASA,
    &LEAF_AIR_ICAO,
    &LEAF_FIN_CAS,
    &LEAF_FIN_IFRS,
    &LEAF_FIN_GAAP,
    &LEAF_OPERATION,
    &LEAF_QUALITY,
];

/// scope → 叶表静态规格（未知 scope 忽略；空列表 = 全部登记叶表）。
fn resolve_tables(scopes: &[String]) -> Vec<&'static LeafSql> {
    if scopes.is_empty() {
        return ALL_LEAF_TABLES.to_vec();
    }
    scopes
        .iter()
        .filter_map(|s| match s.as_str() {
            "air" | "air-caac" => Some(&LEAF_AIR_CAAC),
            "air-faa" => Some(&LEAF_AIR_FAA),
            "air-easa" => Some(&LEAF_AIR_EASA),
            "air-icao" => Some(&LEAF_AIR_ICAO),
            "fin" | "fin-cas" => Some(&LEAF_FIN_CAS),
            "fin-ifrs" => Some(&LEAF_FIN_IFRS),
            "fin-gaap" => Some(&LEAF_FIN_GAAP),
            "operation" => Some(&LEAF_OPERATION),
            "quality" => Some(&LEAF_QUALITY),
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
    'outer: for leaf in &tables {
        // 表名与 SQL 均来自编译期静态注册表；keyword / level / limit 一律参数绑定。
        let sql = if level_filter.is_some() {
            leaf.sql_with_level
        } else {
            leaf.sql
        };
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
            >(sql)
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
                    source_table: leaf.table.to_string(),
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
