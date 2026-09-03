//! 字段值域候选端点 — `GET /approval-flows/context-field-domain?table=<leaf>&column=<col>`
//!
//! 可视化值编辑器的数据源（fix-flow-designer-runtime-chain：zuid→主体徽章、
//! 状态/等级/类目彩色徽章）。候选 SQL 静态分发于 `context_meta::DOMAIN_SQL`
//! （生成期从 meta reference_config 派生；subject join auth_users 取姓名）。
//!
//! table MUST 命中三域叶表白名单且 (table, column) 存在值域查询——否则 400。
//! 纯只读。

use actix_web::{web, HttpResponse};
use common::error::AliothError;
use common::ApiResponse;
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::context_meta::{context_fields_of, domain_sql};

#[derive(Debug, Deserialize)]
pub struct DomainQuery {
    pub table: String,
    pub column: String,
}

#[derive(Debug, Serialize)]
pub struct DomainItem {
    /// 候选行 zuid（字符串化）
    pub id: String,
    /// 展示名（subject = 用户姓名 COALESCE notice；lookup = 字典 notice）
    pub label: String,
    /// 徽章色（t_color_，可空）
    pub color: Option<String>,
}

pub async fn context_field_domain(
    pool: web::Data<sqlx::PgPool>,
    query: web::Query<DomainQuery>,
) -> Result<HttpResponse, AliothError> {
    let DomainQuery { table, column } = query.into_inner();
    let table = table.trim();

    // 白名单双校验：叶表在册 + 字段在册且有值域查询
    if context_fields_of(table).is_none() {
        return Err(AliothError::Validation {
            field: "table".into(),
            message: format!("table 不在上下文三域叶表白名单内: {table}"),
        });
    }
    let Some(sql_text) = domain_sql(table, &column) else {
        return Err(AliothError::Validation {
            field: "column".into(),
            message: format!("字段 {table}.{column} 无值域（标量/文本不提供候选）"),
        });
    };

    let rows = sqlx::query(sql_text)
        .fetch_all(pool.get_ref())
        .await
        .map_err(|e| AliothError::Database(e.to_string()))?;

    let items: Vec<DomainItem> = rows
        .iter()
        .map(|r| DomainItem {
            id: r.get::<i64, _>("id").to_string(),
            label: r
                .try_get::<Option<String>, _>("label")
                .unwrap_or(None)
                .unwrap_or_default(),
            color: r.try_get::<Option<String>, _>("color").unwrap_or(None),
        })
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/approval-flows/context-field-domain",
        web::get().to(context_field_domain),
    );
}
