//! 批量审批通过/驳回 Handler
//!
//! 注册 `POST /approval-instances/batch/approve` 和 `POST /approval-instances/batch/reject`。
//! 一次性处理多个审批实例，单个实例失败不阻塞其余实例。
//!
//! fix-approval-action-chain P0-2：批量与单条共用同一全链（approve_inner/reject_inner：
//! 意见 + lifecycle 桥 + advance（仅通过）+ ApprovalCompleted 事件）。此前 process_one
//! 只写意见——不推进流程、不搭桥、不发事件，UI 显示已批但业务永久停滞（合同不激活、
//! 入职不完结），属链断裂陷阱。
//!
//! ## 路由
//! - `POST /approval-instances/batch/approve` — 批量通过
//! - `POST /approval-instances/batch/reject`  — 批量驳回

use super::approve_reject::{approve_inner, reject_inner};
use actix_web::{web, HttpRequest, HttpResponse};
use common::context;
use common::error::AliothError as ApiError;
use common::event_bus::DomainEventBus;
use common::permissions::require_resource_access;
use common::ApiResponse;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct BatchBody {
    #[serde(with = "common::serde_zuid::seq")]
    pub ids: Vec<i64>,
    pub opinion: Option<String>,
}

#[derive(Debug, Serialize)]
struct BatchResult {
    pub processed: usize,
    pub failed: Vec<BatchFailure>,
}

#[derive(Debug, Serialize)]
struct BatchFailure {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub error: String,
}

/// 批量公共循环：逐实例 NGAC 校验 + 执行动作全链；失败隔离（记 failed 继续）。
async fn process_batch(
    pool: &PgPool,
    bus: &Arc<dyn DomainEventBus>,
    ids: &[i64],
    user_id: i64,
    opinion: &str,
    action: &str,
    approve: bool,
) -> BatchResult {
    let mut result = BatchResult {
        processed: 0,
        failed: Vec::new(),
    };

    for &instance_id in ids {
        if let Err(e) =
            require_resource_access(pool, user_id, "approval-instances", instance_id, action).await
        {
            result.failed.push(BatchFailure {
                id: instance_id,
                error: format!("permission denied: {}", e),
            });
            continue;
        }

        let outcome = if approve {
            approve_inner(pool, bus, instance_id, user_id, opinion).await
        } else {
            reject_inner(pool, bus, instance_id, user_id, opinion).await
        };
        match outcome {
            Ok(_) => result.processed += 1,
            Err(e) => result.failed.push(BatchFailure {
                id: instance_id,
                error: e.to_string(),
            }),
        }
    }

    result
}

/// POST /approval-instances/batch/approve
pub async fn batch_approve(
    pool: web::Data<PgPool>,
    bus: web::Data<Arc<dyn DomainEventBus>>,
    req: HttpRequest,
    body: web::Json<BatchBody>,
) -> Result<HttpResponse, ApiError> {
    let user_id = context::require_auth(&req)?;
    let opinion = body.opinion.as_deref().unwrap_or("");
    let result = process_batch(
        pool.get_ref(),
        bus.get_ref(),
        &body.ids,
        user_id,
        opinion,
        "approve",
        true,
    )
    .await;
    Ok(HttpResponse::Ok().json(ApiResponse::success(result)))
}

/// POST /approval-instances/batch/reject
pub async fn batch_reject(
    pool: web::Data<PgPool>,
    bus: web::Data<Arc<dyn DomainEventBus>>,
    req: HttpRequest,
    body: web::Json<BatchBody>,
) -> Result<HttpResponse, ApiError> {
    let user_id = context::require_auth(&req)?;
    let opinion = body.opinion.as_deref().unwrap_or("");
    // 权限 action 对齐单条 reject（用 "approve" 决策权——种子权限词汇无独立
    // "reject" right，原批量驳回用 "reject" 对非 admin 恒拒，是潜在死端）。
    let result = process_batch(
        pool.get_ref(),
        bus.get_ref(),
        &body.ids,
        user_id,
        opinion,
        "approve",
        false,
    )
    .await;
    Ok(HttpResponse::Ok().json(ApiResponse::success(result)))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/approval-instances/batch/approve",
        web::post().to(batch_approve),
    )
    .route(
        "/approval-instances/batch/reject",
        web::post().to(batch_reject),
    );
}
