//! 上下文对象列表端点 — `GET /approval-flows/context-objects?q=&table=&kind=&limit=`
//!
//! 两类消费方（fix-flow-designer-chain-breaks §D3）：
//! - `kind=definition`：在册 scope-definition 行——流程绑定 fk_context 的对象选择器；
//! - `kind=business`：业务实体行（剔除范畴定义行）——图库「发起」modal 的实体选择器
//!   （entity_id 数据源，配合 initiate 端点 entity_table=所选行落位叶表）。
//!
//! 缺省 kind 返回全部。
//!
//! 返回 `zc_id_proc-context` 族（含 PG 继承子叶）行的 {id, notice, leaf}。
//!
//! - 基表查询经 PG 继承自动并入子叶行（零动态表名，对齐 repositories.rs 范式）；
//! - `table` 参数可选，用于按落位叶表过滤（tableoid::regclass 派生值精确匹配）；
//! - 纯只读。

use actix_web::{web, HttpResponse};
use common::error::AliothError;
use common::ApiResponse;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ContextObjectsQuery {
    /// notice 模糊过滤（可选）
    pub q: Option<String>,
    /// 落位叶表精确过滤（可选，如 zc_id_even-approve）
    pub table: Option<String>,
    /// 上限（默认 200，≤500）
    pub limit: Option<i64>,
    /// definition=仅范畴定义行；business=仅业务实体行；缺省=全部
    pub kind: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct ContextObject {
    pub id: String,
    pub notice: Option<String>,
    pub leaf: String,
}

pub async fn context_objects(
    query: web::Query<ContextObjectsQuery>,
    pool: web::Data<sqlx::PgPool>,
) -> Result<HttpResponse, AliothError> {
    let limit = query.limit.unwrap_or(200).clamp(1, 500);
    let pattern = format!(
        "%{}%",
        query.q.as_deref().unwrap_or("").trim().replace('%', "")
    );
    let table_filter = query.table.as_deref().unwrap_or("").trim().to_string();

    let kind = query.kind.as_deref().unwrap_or("").trim().to_string();
    let rows: Vec<(String, Option<String>, String)> = sqlx::query_as(
        r#"SELECT id::text, notice, tableoid::regclass::text AS leaf
           FROM isahl."zc_id_proc-context"
           WHERE deleted_at IS NULL
             AND ($1 = '' OR notice ILIKE $1)
             AND ($3 = ''
                  OR ($3 = 'definition' AND _t_ = 'scope-definition')
                  OR ($3 = 'business' AND _t_ IS DISTINCT FROM 'scope-definition'
                      AND _t_ IS DISTINCT FROM 'flow-context'))
           ORDER BY id DESC
           LIMIT $2"#,
    )
    .bind(&pattern)
    .bind(limit)
    .bind(&kind)
    .fetch_all(pool.get_ref())
    .await
    .map_err(AliothError::from)?;

    let items: Vec<ContextObject> = rows
        .into_iter()
        .map(|(id, notice, leaf)| ContextObject { id, notice, leaf })
        .filter(|o| table_filter.is_empty() || o.leaf.ends_with(&table_filter))
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/approval-flows/context-objects",
        web::get().to(context_objects),
    );
}
