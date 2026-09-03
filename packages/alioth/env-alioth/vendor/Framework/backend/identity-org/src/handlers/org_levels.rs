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
use sqlx::{AssertSqlSafe, PgPool};

/// 组织等级行（id/code/notice/comments/lv_value/ref_count）
type LevelRow = (
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

async fn list_level_table(pool: &PgPool, table: &str) -> Result<HttpResponse, ApiError> {
    // 表名为调用方白名单字面量（仅 leve-org / leve-post-resp）
    let sql = format!(
        "SELECT id, code, notice, comments, lv_value::text, ref_count FROM \"isahl\".\"{}\" \
         WHERE deleted_at IS NULL ORDER BY id",
        table
    );
    let rows: Vec<LevelRow> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
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
