//! 组织维度只读列表 Handler（strengthen-identity-org）
//!
//! - `GET /org-levels`             — `zc_id_leve-org`（组织级别，leve-structure 叶）
//! - `GET /post-responsibilities`  — `zc_id_leve-post-resp`（岗位责任，leve-structure 叶）
//!
//! 维度数据写路径归模型发布通道，本 handler 仅只读。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::AliothError as ApiError;
use sqlx::PgPool;

/// 组织等级行（id/code/notice/comments/lv_value/ref_count）
type LevelRow = (
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

/// 层级叶表静态 SQL（编译期固化：表名以宏字面量出现，正文单一来源）
macro_rules! level_table_sql {
    ($table:literal) => {
        concat!(
            "SELECT id, code, notice, comments, lv_value::text, ref_count FROM \"isahl\".\"",
            $table,
            "\" WHERE deleted_at IS NULL ORDER BY id"
        )
    };
}

/// 允许的表名 → 静态 SQL（闭式白名单：仅 leve-org / leve-post-resp；未知表 fail-visible）
const LEVEL_TABLE_SQL: &[(&str, &str)] = &[
    ("zc_id_leve-org", level_table_sql!("zc_id_leve-org")),
    (
        "zc_id_leve-post-resp",
        level_table_sql!("zc_id_leve-post-resp"),
    ),
];

async fn list_level_table(pool: &PgPool, table: &str) -> Result<HttpResponse, ApiError> {
    let Some(sql) = LEVEL_TABLE_SQL
        .iter()
        .find(|(t, _)| *t == table)
        .map(|(_, s)| *s)
    else {
        return Err(ApiError::BadRequest(format!("未知层级表: {table}")));
    };
    let rows: Vec<LevelRow> = sqlx::query_as(sql)
        .fetch_all(pool)
        .await
        .map_err(ApiError::from_sqlx)?;
    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, code, notice, comments, lv_value, ref_count)| {
            serde_json::json!({
                "id": id.to_string(),
                "code": code,
                "notice": notice,
                "comments": comments,
                "lv_value": lv_value,
                "ref_count": ref_count,
            })
        })
        .collect();
    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

/// GET /org-levels
pub async fn list_org_levels(
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    require_auth(&req)?;
    list_level_table(pool.get_ref(), "zc_id_leve-org").await
}

/// GET /post-responsibilities
pub async fn list_post_responsibilities(
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    require_auth(&req)?;
    list_level_table(pool.get_ref(), "zc_id_leve-post-resp").await
}

/// 注册组织维度只读路由（strengthen-identity-org）
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/org-levels").route(web::get().to(list_org_levels)))
        .service(
            web::resource("/post-responsibilities")
                .route(web::get().to(list_post_responsibilities)),
        );
}
