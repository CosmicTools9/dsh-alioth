//! 通用知识检索（chat 上下文增强 upgrade-chat-ai-context-coverage E3）
//!
//! 覆盖两类触发：
//! 1. **业务静态域**（`POST /api/knowledge/search`）：`scopes` 映射到叶表后按关键词
//!    ILIKE 检索——会计凭证（`zc_id_docu-accounting`，ledger-entry 服务写入）、
//!    文件文档（`zc_id_file-document`，transport-dispatch / OpenActivity 写入）。
//! 2. **关系驱动**（`POST /api/knowledge/relations`）：页面上下文携带实体标识时，
//!    经业务关系表取该实体**直接引用**的条文——合同 → `zc_id_contract_rr_law`
//!    （`ref_left` = 合同 id，`ref_right` = `zc_id_law` 继承链行；WZ contract 服务
//!    写入），MUST NOT 仅依赖关键词匹配。
//!
//! 表名只来自本文件常量注册表（不接受用户输入表名）；关键词/实体 id 一律绑定参数。

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::api::chat_sessions::adapters::db_llm_config::DbLlmConfigAdapter;
use crate::api::chat_sessions::ports::LlmConfigPort;
use crate::api::knowledge_expand;

/// 静态域检索 SQL（编译期固化：表名是字面量，正文单一来源）
macro_rules! knowledge_search_sql {
    ($t:literal) => {
        concat!(
            "SELECT id, code, notice, comments FROM isahl.\"",
            $t,
            "\" \
             WHERE deleted_at IS NULL \
               AND EXISTS (SELECT 1 FROM unnest($1::text[]) AS term \
                            WHERE notice ILIKE '%' || term || '%' \
                               OR code ILIKE '%' || term || '%' \
                               OR comments ILIKE '%' || term || '%') \
             LIMIT $2"
        )
    };
}

/// 静态域 scope → 叶表 → 检索 SQL（仅登记**有写入方**的域：`zc_id_document` 全仓无生产者，不登记）。
const STATIC_SCOPES: [(&str, &str, &str); 2] = [
    (
        "docu-accounting",
        "zc_id_docu-accounting",
        knowledge_search_sql!("zc_id_docu-accounting"),
    ),
    (
        "file-document",
        "zc_id_file-document",
        knowledge_search_sql!("zc_id_file-document"),
    ),
];

/// 关系驱动的条文引用检索 SQL（编译期固化）
macro_rules! relation_refs_sql {
    ($r:literal, $t:literal) => {
        concat!(
            "SELECT t.id, t.code, t.notice, t.comments FROM isahl.\"",
            $r,
            "\" r \
             JOIN isahl.\"",
            $t,
            "\" t ON t.id = r.ref_right AND t.deleted_at IS NULL \
             WHERE r.ref_left = $1 AND r.deleted_at IS NULL \
             ORDER BY t.id LIMIT $2"
        )
    };
}

/// 关系注册表：entity → (目标表, 关系名, 检索 SQL)。
/// `zc_id_contract_rr_standard` 全仓无写入方（仅继承图存在），故不登记以免空转。
struct RelationRoute {
    entity: &'static str,
    /// 起点顶点 origin（知识图 Node.origin；多跳入口）。
    start_origin: &'static str,
    target: &'static str,
    label: &'static str,
    sql: &'static str,
}

const RELATIONS: &[RelationRoute] = &[RelationRoute {
    entity: "contract",
    start_origin: "zc_id_contract",
    target: "zc_id_law",
    label: "law",
    sql: relation_refs_sql!("zc_id_contract_rr_law", "zc_id_law"),
}];

#[derive(Debug, Deserialize)]
pub struct KnowledgeSearchReq {
    pub keywords: Vec<String>,
    pub scopes: Option<Vec<String>>,
    pub max_results: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct RelationReq {
    /// 实体名（当前支持 `contract`）。
    pub entity: String,
    /// 实体 id（ID_JSON_PRECISION：字符串形态 zuid）。
    pub entity_id: String,
    pub limit: Option<i32>,
    /// 有界多跳深度（1..3，默认 1 = 既有单跳；change
    /// extend-knowledge-graph-cypher-backend B-1）。>1 时经知识图投影
    /// 取链式引用（Cypher 优先 / SQL 递归降级，输出同构附带 via_chain）。
    pub depth: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct KnowledgeHit {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub title: Option<String>,
    pub snippet: Option<String>,
    pub source_table: String,
    /// 关系驱动命中的关系名（静态域检索为 None）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<String>,
    /// 多跳路径（≥2 跳命中时的 origin 序列；单跳/静态域缺省——同构兼容）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_chain: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct KnowledgeResp {
    pub hits: Vec<KnowledgeHit>,
    pub total: usize,
}

/// scope 列表 → (叶表, 检索 SQL)（未知 scope 忽略；空列表 = 全部登记域）。
fn resolve_static_scope_sqls(scopes: &[String]) -> Vec<(&'static str, &'static str)> {
    if scopes.is_empty() {
        return STATIC_SCOPES.iter().map(|(_, t, sql)| (*t, *sql)).collect();
    }
    scopes
        .iter()
        .filter_map(|scope| {
            STATIC_SCOPES
                .iter()
                .find(|(name, _, _)| name == scope)
                .map(|(_, t, sql)| (*t, *sql))
        })
        .collect()
}

/// entity → 关系路由（关系表 / 目标表 / 关系名 / 编译期 SQL）。
fn resolve_relation(entity: &str) -> Option<&'static RelationRoute> {
    RELATIONS.iter().find(|r| r.entity == entity)
}

/// 请求 locale（扩展语言提示）：`Accept-Language` 含 en → "en"，否则 "zh-CN"。
fn request_locale(req: &HttpRequest) -> String {
    let header = req
        .headers()
        .get("Accept-Language")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if header.to_ascii_lowercase().contains("en") {
        "en".to_string()
    } else {
        "zh-CN".to_string()
    }
}

/// POST /api/knowledge/search — 业务静态域关键词检索（含 LLM 查询扩展，add-knowledge-query-expansion）。
pub async fn knowledge_search(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<KnowledgeSearchReq>,
) -> Result<HttpResponse, actix_web::Error> {
    if body.keywords.is_empty() {
        return Ok(
            HttpResponse::BadRequest().json(serde_json::json!({"error":"keywords required"}))
        );
    }
    let max_results = body.max_results.unwrap_or(5).clamp(1, 20) as i64;
    let scopes = body.scopes.as_deref().unwrap_or(&[]);
    let table_sqls = resolve_static_scope_sqls(scopes);

    // 查询扩展（阶段一）：LLM 每次请求加载一次；仅对**首个关键词**扩展（主查询词），
    // 其余关键词原样参与——把单请求 LLM 调用限制在 ≤1 次（延迟与成本有界）。
    // 不可用 ⇒ 空扩展（降级为原词检索）。
    let locale = request_locale(&req);
    let llm = match DbLlmConfigAdapter::new(pool.get_ref().clone())
        .load_service()
        .await
    {
        Ok(service) => Some(service),
        Err(e) => {
            common::telemetry::warn!("knowledge expand degraded: llm config unavailable: {}", e);
            None
        }
    };
    let primary = body.keywords.first().cloned().unwrap_or_default();
    let expansions = match &llm {
        Some(service) => knowledge_expand::expand_keywords(service, &primary, &locale).await,
        None => Vec::new(),
    };
    // 每个关键词的检索词表（有界）：主词 = 原词 ∪ 扩展词；其余词 = 自身。
    // 请求级预算（design D2）：所有词表累计 MUST ≤ QUERY_MAX_TERMS。
    let mut budget = knowledge_expand::QUERY_MAX_TERMS;
    let term_sets: Vec<Vec<String>> = body
        .keywords
        .iter()
        .map(|keyword| {
            let set = if keyword == &primary {
                knowledge_expand::merge_terms(std::slice::from_ref(keyword), &expansions)
            } else {
                knowledge_expand::merge_terms(std::slice::from_ref(keyword), &[])
            };
            let set: Vec<String> = set.into_iter().take(budget).collect();
            budget = budget.saturating_sub(set.len());
            set
        })
        .collect();

    let mut hits: Vec<KnowledgeHit> = Vec::new();
    'outer: for (table, sql) in &table_sqls {
        // 词表形态：原词 ∪ 扩展词（有界）；表名与 SQL 均来自编译期常量注册表，词一律参数绑定。
        for terms in &term_sets {
            let rows =
                sqlx::query_as::<_, (i64, Option<String>, Option<String>, Option<String>)>(*sql)
                    .bind(&terms)
                    .bind(max_results)
                    .fetch_all(pool.get_ref())
                    .await
                    .map_err(|e| {
                        actix_web::error::ErrorInternalServerError(format!("knowledge search: {e}"))
                    })?;
            for (id, code, notice, comments) in rows {
                if hits.iter().any(|hit| hit.id == id) {
                    continue;
                }
                hits.push(KnowledgeHit {
                    id,
                    code,
                    title: notice,
                    snippet: comments,
                    source_table: table.to_string(),
                    relation: None,
                    via_chain: None,
                });
                if hits.len() >= max_results as usize {
                    break 'outer;
                }
            }
        }
    }
    let total = hits.len();
    Ok(HttpResponse::Ok().json(KnowledgeResp { hits, total }))
}

/// POST /api/knowledge/relations — 关系驱动的实体引用条文检索（有界多跳）。
///
/// depth=1（默认）走既有编译期 SQL（行为逐字段兼容）；depth ∈ (1,3] 经知识图投影
/// （`isahl_knowledge` AGE 图）取链式引用——Cypher UNION 变长路径优先（AGE 1.8 无
/// `|` 边类型联合语法，按边类型 UNION），不可用降级 SQL 递归 CTE，输出同构。
pub async fn knowledge_relations(
    pool: web::Data<PgPool>,
    req: web::Json<RelationReq>,
) -> Result<HttpResponse, actix_web::Error> {
    let Some(route) = resolve_relation(&req.entity) else {
        return Ok(HttpResponse::BadRequest()
            .json(serde_json::json!({"error": format!("unsupported entity: {}", req.entity)})));
    };
    let Ok(entity_id) = req.entity_id.trim().parse::<i64>() else {
        return Ok(HttpResponse::BadRequest()
            .json(serde_json::json!({"error": format!("invalid entity_id: {}", req.entity_id)})));
    };
    let limit = req.limit.unwrap_or(5).clamp(1, 20) as i64;
    let depth = req.depth.unwrap_or(1);
    if !(1..=3).contains(&depth) {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": format!("depth out of range [1,3]: {depth}")
        })));
    }

    // 单跳：既有编译期 SQL（含全部既有语义——目标父表继承链 + 软删过滤 + limit）。
    if depth == 1 {
        let rows =
            sqlx::query_as::<_, (i64, Option<String>, Option<String>, Option<String>)>(route.sql)
                .bind(entity_id)
                .bind(limit)
                .fetch_all(pool.get_ref())
                .await
                .map_err(|e| {
                    actix_web::error::ErrorInternalServerError(format!("knowledge relations: {e}"))
                })?;
        let hits: Vec<KnowledgeHit> = rows
            .into_iter()
            .map(|(id, code, notice, comments)| KnowledgeHit {
                id,
                code,
                title: notice,
                snippet: comments,
                source_table: route.target.to_string(),
                relation: Some(route.label.to_string()),
                via_chain: None,
            })
            .collect();
        let total = hits.len();
        return Ok(HttpResponse::Ok().json(KnowledgeResp { hits, total }));
    }

    // 多跳：AGE 优先（common::age 三态准入 + 显式降级），SQL 递归 CTE 兜底。
    // 去重键 (id, via_chain)；via 为 origin 序列（含起点终点）。
    let mut dedup = std::collections::HashSet::new();
    let mut hits: Vec<KnowledgeHit> = Vec::new();
    let mut rows = multi_hop_age(pool.get_ref(), route.start_origin, entity_id, depth)
        .await
        .unwrap_or_else(|| Vec::new());
    if rows.is_empty() {
        rows = multi_hop_sql(pool.get_ref(), entity_id, depth)
            .await
            .map_err(|e| {
                actix_web::error::ErrorInternalServerError(format!(
                    "knowledge relations multi-hop: {e}"
                ))
            })?;
    }
    for (id, origin, code, name, via) in rows {
        if !dedup.insert((id, via.clone())) {
            continue;
        }
        hits.push(KnowledgeHit {
            id,
            code,
            title: name,
            snippet: None,
            source_table: origin,
            relation: Some(route.label.to_string()),
            via_chain: if via.len() >= 2 { Some(via) } else { None },
        });
        if hits.len() >= limit as usize {
            break;
        }
    }
    let total = hits.len();
    Ok(HttpResponse::Ok().json(KnowledgeResp { hits, total }))
}

/// 多跳 Cypher（AGE 可用时）：经 DB 侧投影函数 `isahl_knowledge.knowledge_multi_hop`
/// 执行四边类型 VLE（AGE 1.8 无 `|` 边类型联合语法，故库侧按边类型分支 UNION），
/// 返回 (id, origin, code, name, via)。None = AGE 不可用/异常（调用方降级 SQL）。
///
/// 参数通道：运行时值以 text/bigint/int 绑定传入，库内 cast 为 agtype 后交 `cypher()`
/// 第三参——**Rust 侧绑定 agtype 参数不可行**（sqlx 参数一律 binary 格式，AGE
/// `agtype_recv` 只接受其私有二进制布局），平台事实见 `common::age` 模块文档。
async fn multi_hop_age(
    pool: &PgPool,
    start_origin: &str,
    entity_id: i64,
    depth: i32,
) -> Option<Vec<(i64, String, Option<String>, Option<String>, Vec<String>)>> {
    // 边类型与起点 origin 为编译期常量（RelationRoute 派生），depth 已校验 ∈ [1,3]。
    // 跨 label 混走：AGE 不支持「变长 + label alternation」（1.8.0 实测语法拒绝）⇒ 无类型
    // 变长 + 关系类型白名单过滤，与 SQL 递归 CTE 同口径（contract→法条→条文 两跳含混走）。
    let query = format!(
        "MATCH p=(s:Node {{origin: $origin, id: $id}})-[*1..{depth}]->(t:Node) \
         WHERE size([x IN relationships(p) WHERE NOT (type(x) IN ['REL', 'BRIDGE_LAW', 'BRIDGE_REFERENCE', 'BRIDGE_FORMULA'])]) = 0 \
         RETURN {{id: t.id, origin: t.origin, code: t.code, name: t.name, \
                  via: [n IN nodes(p) | n.origin]}}"
    );
    let rows = common::age::try_cypher_json(
        pool,
        common::age::KNOWLEDGE_GRAPH,
        &query,
        Some(&serde_json::json!({"origin": start_origin, "id": entity_id})), // id-json-ok：agtype 数字参数（15 位 zuid < 2^53），非 JS 消费面
    )
    .await
    .ok()??;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let via = r
            .get("via")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        out.push((
            r.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
            r.get("origin")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            r.get("code").and_then(|v| v.as_str()).map(String::from),
            r.get("name").and_then(|v| v.as_str()).map(String::from),
            via,
        ));
    }
    Some(out)
}

/// 多跳 SQL 递归降级：边集 = contract_rr_law + 三桥（有向），深度 ≤3，路径数组聚合。
async fn multi_hop_sql(
    pool: &PgPool,
    entity_id: i64,
    depth: i32,
) -> Result<Vec<(i64, String, Option<String>, Option<String>, Vec<String>)>, sqlx::Error> {
    // 边集：contract REL + 三桥（有向；与图投影同口径）。节点视图回收终点属性
    //（23 白名单表 UNION ALL，FROM ONLY 层表互斥——与 AVIC 服务/图投影白名单对齐；
    //  contract 起点无表属性，仅占位 origin）。via 以 text[] 数组传播（无字符串 hack）。
    let rows = sqlx::query_as::<_, (i64, String, Option<String>, Option<String>, Vec<String>)>(
        r#"
        WITH RECURSIVE nodes AS (
            SELECT t.id, 'zc_id_stan-air-caac'::text AS origin, t.code, t.notice AS name
              FROM ONLY isahl."zc_id_stan-air-caac" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_stan-air-caac-article', t.code, t.notice FROM ONLY isahl."zc_id_stan-air-caac-article" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_stan-air-faa', t.code, t.notice FROM ONLY isahl."zc_id_stan-air-faa" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_stan-air-faa-article', t.code, t.notice FROM ONLY isahl."zc_id_stan-air-faa-article" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_stan-air-easa', t.code, t.notice FROM ONLY isahl."zc_id_stan-air-easa" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_stan-air-easa-article', t.code, t.notice FROM ONLY isahl."zc_id_stan-air-easa-article" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_stan-air-icao', t.code, t.notice FROM ONLY isahl."zc_id_stan-air-icao" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_stan-air-icao-article', t.code, t.notice FROM ONLY isahl."zc_id_stan-air-icao-article" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_stan-fin-cas', t.code, t.notice FROM ONLY isahl."zc_id_stan-fin-cas" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_stan-fin-cas-article', t.code, t.notice FROM ONLY isahl."zc_id_stan-fin-cas-article" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_stan-fin-ifrs', t.code, t.notice FROM ONLY isahl."zc_id_stan-fin-ifrs" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_stan-fin-ifrs-article', t.code, t.notice FROM ONLY isahl."zc_id_stan-fin-ifrs-article" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_law-civil-code', t.code, t.notice FROM ONLY isahl."zc_id_law-civil-code" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_law-civil-book', t.code, t.notice FROM ONLY isahl."zc_id_law-civil-book" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_law-civil-chapter', t.code, t.notice FROM ONLY isahl."zc_id_law-civil-chapter" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_law-civil-section', t.code, t.notice FROM ONLY isahl."zc_id_law-civil-section" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_law-civil-article', t.code, t.notice FROM ONLY isahl."zc_id_law-civil-article" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_law-common-statute', t.code, t.notice FROM ONLY isahl."zc_id_law-common-statute" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_law-common-title', t.code, t.notice FROM ONLY isahl."zc_id_law-common-title" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_law-common-chapter', t.code, t.notice FROM ONLY isahl."zc_id_law-common-chapter" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_law-common-section', t.code, t.notice FROM ONLY isahl."zc_id_law-common-section" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_law-common-case', t.code, t.notice FROM ONLY isahl."zc_id_law-common-case" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT t.id, 'zc_id_law-common-holding', t.code, t.notice FROM ONLY isahl."zc_id_law-common-holding" t WHERE t.deleted_at IS NULL
            UNION ALL SELECT r.ref_left, 'zc_id_contract', NULL, NULL
              FROM isahl."zc_id_contract_rr_law" r WHERE r.deleted_at IS NULL
        ),
        edges AS (
            SELECT r.ref_left AS l, r.ref_right AS rr
              FROM isahl."zc_id_contract_rr_law" r WHERE r.deleted_at IS NULL
            UNION ALL
            SELECT b.ref_left, b.ref_right FROM isahl."zc_id_standard_rr_law" b WHERE b.deleted_at IS NULL
            UNION ALL
            SELECT b.ref_left, b.ref_right FROM isahl."zc_id_standard_rr_reference" b WHERE b.deleted_at IS NULL
            UNION ALL
            SELECT b.ref_left, b.ref_right FROM isahl."zc_id_standard_r_formula" b WHERE b.deleted_at IS NULL
        ),
        walk(id, via, depth) AS (
            SELECT $1::bigint, ARRAY['zc_id_contract'::text], 0
            UNION
            SELECT e.rr, w.via || n.origin, w.depth + 1
              FROM walk w
              JOIN edges e ON e.l = w.id
              JOIN nodes n ON n.id = e.rr
             WHERE w.depth < $2
        )
        SELECT n.id, n.origin, n.code, n.name, w.via
          FROM walk w JOIN nodes n ON n.id = w.id
         WHERE w.depth > 0
        "#,
    )
    .bind(entity_id)
    .bind(depth)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/knowledge")
            .route("/search", web::post().to(knowledge_search))
            .route("/relations", web::post().to(knowledge_relations)),
    );
}

#[cfg(test)]
mod db_tests {
    //! 双路径等价资产（C-3）：AGE 优先路径与 SQL 递归降级路径必须对同一起点给出同一集合。
    //!
    //! 私有实现直连（同模块）：`multi_hop_age` / `multi_hop_sql`。AGE 不可用时断言**显式降级**
    //! （`Ok(None)`），AGE 可用时断言两路径集合逐项相等——即守门项「Cypher 优先 / SQL 递归降级
    //! 同构」的可复跑形式（此前仅一次性手工验证，无资产）。

    use super::*;
    use common::age::{age_status, AgeStatus, KNOWLEDGE_GRAPH};
    use std::collections::BTreeSet;

    /// 多跳行走行形态：(id, origin, code, name, via)
    type HopRow = (i64, String, Option<String>, Option<String>, Vec<String>);

    const T_CODE_REL: &str = "KM-T-MH-SRC-REL";
    const T_CODE_REF: &str = "KM-T-MH-SRC-REF";
    const T_CONTRACT: i64 = 999_000_000_000_002;

    async fn seed_chain(pool: &PgPool) -> (i64, i64) {
        let a: i64 = sqlx::query_scalar(
            r#"SELECT id FROM isahl."zc_id_law-common-section" WHERE deleted_at IS NULL ORDER BY id LIMIT 1"#,
        )
        .fetch_one(pool)
        .await
        .expect("common-section 种子行缺失");
        let b: i64 = sqlx::query_scalar(
            r#"SELECT id FROM isahl."zc_id_stan-air-caac-article" WHERE deleted_at IS NULL ORDER BY id LIMIT 1"#,
        )
        .fetch_one(pool)
        .await
        .expect("caac 条文种子行缺失");
        sqlx::query(r#"DELETE FROM isahl."zc_id_contract_rr_law" WHERE code = $1"#)
            .bind(T_CODE_REL)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(r#"DELETE FROM isahl."zc_id_standard_rr_reference" WHERE code = $1"#)
            .bind(T_CODE_REF)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_contract_rr_law" (code, ref_left, ref_right) VALUES ($1, $2, $3)"#,
        )
        .bind(T_CODE_REL)
        .bind(T_CONTRACT)
        .bind(a)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_standard_rr_reference" (code, ref_left, ref_right) VALUES ($1, $2, $3)"#,
        )
        .bind(T_CODE_REF)
        .bind(a)
        .bind(b)
        .execute(pool)
        .await
        .unwrap();
        (a, b)
    }

    async fn cleanup(pool: &PgPool) {
        let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_contract_rr_law" WHERE code = $1"#)
            .bind(T_CODE_REL)
            .execute(pool)
            .await;
        let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_standard_rr_reference" WHERE code = $1"#)
            .bind(T_CODE_REF)
            .execute(pool)
            .await;
    }

    fn key(rows: &[HopRow]) -> BTreeSet<(i64, Vec<String>)> {
        rows.iter().map(|r| (r.0, r.4.clone())).collect()
    }

    #[tokio::test]
    async fn age_and_sql_paths_agree_when_age_usable_or_degrade_explicitly() {
        let pool = ::common::testing::connect_test_db().await;
        cleanup(&pool).await;
        let (a, b) = seed_chain(&pool).await;

        let sql_rows = multi_hop_sql(&pool, T_CONTRACT, 2)
            .await
            .expect("SQL 降级路径查询失败");
        let sql_ids: BTreeSet<i64> = sql_rows.iter().map(|r| r.0).collect();
        assert!(
            sql_ids.contains(&a) && sql_ids.contains(&b),
            "SQL 递归路径必须覆盖两跳链（合同→法条→适航条文）：{sql_rows:?}"
        );
        let two_hop = sql_rows.iter().find(|r| r.0 == b).expect("两跳终点");
        assert_eq!(two_hop.4.len(), 3, "两跳命中 via 应为 [合同, 中继, 终点]");

        // AGE 侧读的是迁移期投影快照：本次插入的测试桥行不在快照内 ⇒ 先幂等重建投影再比对。
        // 重建失败（AGE 运行时缺函数/权限）视同 AGE 不可用——走显式降级断言，
        // 绝不把「等价断言未执行」掩盖为通过。
        let projection_rebuilt =
            sqlx::query("SELECT isahl_knowledge.age_rebuild_knowledge_graph()")
                .execute(&pool)
                .await
                .is_ok();

        // AGE 参数通道探针：AGE 解析期要求 cypher() 第三参为**裸 Param** 节点
        // （`src/backend/parser/cypher_analyze.c`：`IsA(arg3, Param)`）且声明类型恰为
        // `ag_catalog.agtype`；`common::age` 以 TEXT 绑定 ⇒ PG 解析不到该重载或必须插入
        // Coercion 节点，两者均被 AGE 拒绝 ⇒ 参数化 Cypher 一律降级为 SQL。绑定修好后
        // 本分支自动升级为真实等价断言。
        let age_param_channel_ok = projection_rebuilt
            && common::age::try_cypher_json(
                &pool,
                KNOWLEDGE_GRAPH,
                "MATCH (s:Node {origin: $origin, id: $id}) RETURN {n: count(s)}",
                Some(&serde_json::json!({"origin": "zc_id_contract", "id": T_CONTRACT})),
            )
            .await
            .ok()
            .flatten()
            .is_some();

        match (
            age_status(&pool, KNOWLEDGE_GRAPH).await,
            projection_rebuilt,
            age_param_channel_ok,
        ) {
            (AgeStatus::Usable, true, true) => {
                let age_rows = multi_hop_age(&pool, "zc_id_contract", T_CONTRACT, 2)
                    .await
                    .expect(
                        "AGE 可用、投影已重建且参数通道可用时不得降级（Ok(None) 仅表示不可用）",
                    );
                assert_eq!(
                    key(&age_rows),
                    key(&sql_rows),
                    "AGE 与 SQL 两路径多跳命中集合必须逐项一致（via 同口径：焦点起步）"
                );
            }
            (status, rebuilt, channel) => {
                assert!(
                    multi_hop_age(&pool, "zc_id_contract", T_CONTRACT, 2)
                        .await
                        .is_none(),
                    "AGE 状态 {status:?}（投影重建成功={rebuilt}，参数通道可用={channel}）时必须显式降级（None），不得静默空结果"
                );
            }
        }

        cleanup(&pool).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scoped_tables(scopes: &[&str]) -> Vec<&'static str> {
        let owned: Vec<String> = scopes.iter().map(|s| (*s).to_string()).collect();
        resolve_static_scope_sqls(&owned)
            .into_iter()
            .map(|(table, _)| table)
            .collect()
    }

    #[test]
    fn static_scopes_map_to_registered_leaf_tables_only() {
        assert_eq!(
            scoped_tables(&["docu-accounting"]),
            vec!["zc_id_docu-accounting"]
        );
        assert_eq!(
            scoped_tables(&["file-document"]),
            vec!["zc_id_file-document"]
        );
        // 空 scopes = 全部登记域
        assert_eq!(scoped_tables(&[]).len(), STATIC_SCOPES.len());
        // 未登记域（含无生产者的 zc_id_document）→ 不返回任何表（不跨域串结果）
        assert!(scoped_tables(&["document"]).is_empty());
    }

    #[test]
    fn relation_registry_covers_contract_law_only() {
        let route = resolve_relation("contract").expect("contract 已登记");
        assert_eq!((route.target, route.label), ("zc_id_law", "law"));
        // 无写入方的关系（rr_standard）与未知实体 → None（空转防护）
        assert!(resolve_relation("contract-standard").is_none());
        assert!(resolve_relation("unknown").is_none());
    }

    /// 集成（双路径不变量）：AGE 可用时多跳 MUST 走 Cypher
    /// （`isahl_knowledge.knowledge_multi_hop`）而非静默降级，且 Cypher 命中 MUST 为 SQL
    /// 递归路径命中的子集——AGE 1.8 无 `|` 边类型联合 VLE，混合类型链仅 SQL 可达，
    /// 故 MUST NOT 主张集合全等；`via` 首位 MUST 为起点 origin。
    #[tokio::test]
    async fn multi_hop_age_is_cypher_backed_and_sql_superset() {
        use std::collections::HashSet;

        let pool = common::testing::connect_test_db().await;
        let route = resolve_relation("contract").expect("contract 已登记");
        let start_origin = route.start_origin;
        let starts: Vec<i64> = sqlx::query_scalar(
            "SELECT (ag_catalog.agtype_to_jsonb(n.properties)->>'id')::bigint \
               FROM isahl_knowledge.\"Node\" n \
              WHERE ag_catalog.agtype_to_jsonb(n.properties)->>'origin' = $1",
        )
        .bind(start_origin)
        .fetch_all(&pool)
        .await
        .expect("起点查询不应抛错");
        assert!(
            !starts.is_empty(),
            "test 库无 {start_origin} 顶点——知识图投影缺失"
        );

        let (mut sql_total, mut cypher_total) = (0usize, 0usize);
        for id in starts {
            for depth in [2, 3] {
                let sql_hits = multi_hop_sql(&pool, id, depth)
                    .await
                    .expect("SQL 兜底路径不应抛错");
                if sql_hits.is_empty() {
                    continue;
                }
                let age_hits = multi_hop_age(&pool, start_origin, id, depth)
                    .await
                    .expect("AGE 可用时 MUST 走 Cypher 路径（MUST NOT 静默降级 None）");
                let sql_ids: HashSet<i64> = sql_hits.iter().map(|r| r.0).collect();
                let age_ids: HashSet<i64> = age_hits.iter().map(|r| r.0).collect();
                assert!(
                    age_ids.is_subset(&sql_ids),
                    "Cypher 命中越出 SQL 递归面（起点 {id} depth {depth}）：{age_ids:?} ⊄ {sql_ids:?}"
                );
                for (_, _, _, _, via) in &age_hits {
                    assert_eq!(
                        via.first().map(String::as_str),
                        Some(start_origin),
                        "via 首位 MUST 为起点 origin"
                    );
                }
                sql_total += sql_hits.len();
                cypher_total += age_hits.len();
            }
        }
        assert!(
            sql_total > 0,
            "contract 起点无可达命中——种子/投影缺失使该守门空转"
        );
        assert!(cypher_total > 0, "Cypher 路径零命中（AGE 读面未生效）");
    }
}
