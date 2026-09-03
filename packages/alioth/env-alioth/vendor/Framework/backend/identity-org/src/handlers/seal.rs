//! Seal 自定义 handler —— 批量创建铅封（add-wz-seal-batch-creation；
//! refactor-dispatch-seal-code-generation 重构：去字典 o_number 依赖，code 前缀自动续号）
//!
//! 端点（挂载在 /service/isahl-db/seals）：
//! - `POST /seals/batch` — `startCode` 缺省时按类型 code 前缀自动续号（count 1=单号、N=连号）；
//!   `startCode` 显式时从起始号等宽递增（事务原子 + code 查重 + NGAC 行注册）
//!
//! 注册顺序约束：本 register 必须先于 `crud_routes::<Seal, …>("/seals")` 注册
//! （actix 按注册顺序匹配，避免 `/{id}` 抢占 `/batch`）。

use crate::models::{CreateSealBatchRequest, Seal};
use crate::repository::SealRepository;
use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::AliothError as ApiError;
use sqlx::PgPool;

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/seals").route("/batch", web::post().to(seal_batch_create)));
}

/// POST /seals/batch — 批量创建连号铅封
pub async fn seal_batch_create(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<CreateSealBatchRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let repo = SealRepository::from(pool.get_ref().clone());
    let items = repo.batch_create(body.into_inner(), user_id).await?;
    // NGAC：与 crud_create 一致，每行注册 object_attribute + 创建者关联（best-effort）
    for item in &items {
        crud::register_created_resource_ngac::<Seal>(pool.get_ref(), item.id, user_id).await;
    }
    // 补 _refs（REFERENCE_RESOLVER_SPEC：读取结果经 _refs 自动解析关联值）
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match repo.get_refs(item.id).await {
            Ok(Some(full)) => out.push(full),
            _ => out.push(item),
        }
    }
    Ok(HttpResponse::Created().json(ApiResponse::success(out)))
}
