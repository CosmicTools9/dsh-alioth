//! 流程发起端点 — `POST /approval-flows/{id}/initiate`
//!
//! fix-flow-designer-runtime-chain D4：以业务实体（task/event 域叶表行）发起
//! 已发布流程。校验链（全 400 fail-closed）：已发布 → 已绑定输入范畴 →
//! entity_table == 范畴叶表 → 三域白名单 → 实体行存在。随后从 start 节点按
//! 边条件求值推进创建首链审批实例，实体引用经 operation_rr 桥逐实例绑定
//! （D3），实例 comments 为可读摘要文本（remove-comments-json-embedding 合规）。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context;
use common::error::AliothError;
use common::ApiResponse;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::advance::initiate_flow;
use crate::context_meta::context_fields_of;
use ::common::event_bus::DomainEventBus;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct InitiateRequest {
    /// 业务实体叶表（MUST 等于流程绑定范畴的叶表）
    pub entity_table: String,
    /// 业务实体行 id（zuid；JSON 传输字符串化，防 2^53 精度截断）
    #[serde(with = "common::serde_zuid")]
    pub entity_id: i64,
}

#[derive(Debug, Serialize)]
pub struct InitiateResponse {
    #[serde(with = "common::serde_zuid::seq")]
    pub instance_ids: Vec<i64>,
    /// 本次执行的 实现·实例 行 id（flow-lifecycle-split 链根）
    #[serde(with = "common::serde_zuid")]
    pub execution_id: i64,
}

/// 流程绑定上下文（fk_context 行）：
/// - 旧形态（scope-definition 落业务叶表）：叶表直接匹配 entity_table
/// - 新形态（flow-context 范例行，落域父表）：entity_table 同域判定
async fn bound_leaf_table(
    pool: &PgPool,
    flow_id: i64,
) -> Result<Option<(String, String)>, AliothError> {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        r#"SELECT tableoid::regclass::text, _t_
           FROM isahl."zc_id_proc-context"
           WHERE id = (SELECT fk_context FROM isahl.zc_id_process WHERE id = $1)
             AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AliothError::Database(e.to_string()))?;
    let Some((leaf, _t)) = row else {
        return Ok(None);
    };
    let leaf = leaf.trim_matches('"').to_string();
    let domain = crate::context_domain::domain_of_leaf(&leaf).unwrap_or("");
    Ok(Some((leaf, domain.to_string())))
}

pub async fn initiate(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<InitiateRequest>,
    bus: Option<web::Data<Arc<dyn DomainEventBus>>>,
) -> Result<HttpResponse, AliothError> {
    let flow_id = path.into_inner();
    let user_id = context::require_auth(&req)?;
    let req = body.into_inner();
    let pool_ref = pool.get_ref();

    // 1. 已发布（当前有活跃节点批次）
    let published: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
             SELECT 1 FROM isahl.zc_id_process_rr_operation rro
             WHERE rro.ref_left = $1 AND rro.deleted_at IS NULL
           )"#,
    )
    .bind(flow_id)
    .fetch_one(pool_ref)
    .await
    .map_err(|e| AliothError::Database(e.to_string()))?;
    if !published {
        return Err(AliothError::Validation {
            field: "flow".into(),
            message: format!("flow {flow_id} 未发布（无活跃节点批次），不可发起"),
        });
    }

    // 2. 流程已绑定输入范畴，且 entity_table 与绑定上下文一致（域判定）
    let bound = bound_leaf_table(pool_ref, flow_id).await?;
    let Some((bound_leaf, bound_domain)) = bound else {
        return Err(AliothError::Validation {
            field: "entity_table".into(),
            message: format!("flow {flow_id} 未绑定输入范畴（fk_context），不可携带实体发起"),
        });
    };
    let req_domain = crate::context_domain::domain_of_leaf(&req.entity_table).unwrap_or("");
    if bound_domain.is_empty() {
        // 旧形态（scope-definition 落业务叶表）：直接匹配叶表
        if req.entity_table != bound_leaf {
            return Err(AliothError::Validation {
                field: "entity_table".into(),
                message: format!(
                    "entity_table '{}' 与流程绑定范畴叶表 '{bound_leaf}' 不一致",
                    req.entity_table
                ),
            });
        }
    } else if req_domain != bound_domain {
        // 新形态（flow-context 范例行落域父表）：entity_table 必须同域（后代叶表）
        return Err(AliothError::Validation {
            field: "entity_table".into(),
            message: format!(
                "entity_table '{}' 与流程绑定上下文域 '{bound_domain}' 不一致（须为 {bound_leaf} 族叶表）",
                req.entity_table
            ),
        });
    }

    // 3. 三域叶表白名单（context_meta 静态索引）
    if context_fields_of(&req.entity_table).is_none() {
        return Err(AliothError::Validation {
            field: "entity_table".into(),
            message: format!("entity_table '{}' 不在三域叶表白名单内", req.entity_table),
        });
    }

    // 4. 实体行存在（静态 SQL 分发，防悬空桥——行缺失 → fetch_optional None）
    let Some(sql) = crate::context_meta::entity_row_sql(&req.entity_table) else {
        return Err(AliothError::Validation {
            field: "entity_table".into(),
            message: format!("entity_table '{}' 无实体加载通道", req.entity_table),
        });
    };
    let entity_row: Option<serde_json::Value> = sqlx::query_scalar(sql)
        .bind(req.entity_id)
        .fetch_optional(pool_ref)
        .await
        .map_err(|e| AliothError::Database(e.to_string()))?;
    if entity_row.is_none() {
        return Err(AliothError::Validation {
            field: "entity_id".into(),
            message: format!("实体行不存在：{}(#{})", req.entity_table, req.entity_id),
        });
    }

    // 5. 物化执行行（实现·实例，tpl_id → 范例）并以之为链根发起
    let summary = format!("流程发起：实体 {}#{}", req.entity_table, req.entity_id);
    let execution_id =
        crate::advance::materialize_flow_execution(pool_ref, flow_id, user_id, &summary).await?;
    let instance_ids = initiate_flow(
        pool_ref,
        flow_id,
        user_id,
        &req.entity_table,
        req.entity_id,
        Some(execution_id),
        bus.as_ref().map(|d| d.get_ref()),
    )
    .await?;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(InitiateResponse {
            instance_ids,
            execution_id,
        })),
    )
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route("/approval-flows/{id}/initiate", web::post().to(initiate));
}
