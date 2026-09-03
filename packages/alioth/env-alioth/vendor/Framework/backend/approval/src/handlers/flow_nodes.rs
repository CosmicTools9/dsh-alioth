//! WZ approval 薄包装 —— 流程节点读端点
//!
//! `GET /service/approval/flows/{id}/nodes`
//!
//! Framework approval crate 的 `/flow-nodes` CRUD 仅支持分页/搜索，
//! 无「按流程过滤 + 排序」能力；且原型（block.tsx）契约字段为
//! `id/processId/name/nodeType/order`。遵循 REUSE_FIRST：复用 crate
//! `approval::models::FlowNode`（表 `isahl."zc_id_even-approve"`）做
//! 行解码，本模块只做查询编排 + DTO 映射，不动 Framework crate。
//!
//! 排序语义：表无显式 order 列，节点行按 `id`（ZUID 单调递增）升序
//! 返回，`order` 字段按行序派生（1-based），与原型「按 order 升序展示」一致。
//!
//! nodeType 语义：优先 `t_color_`（草稿节点直插时写入）；发布（publish）物化的
//! 节点类型存 `zc_id_operation.ck_cate-proc_op → zc_id_cate-proc_op.code`
//! （`approve`/`cc`/`condition`/`parallel`/`gate` 等；§4.4.1）——start/end
//! 无分类，end 经 rr_statement 桥判定，其余无分类回退 start（图端点）。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::error::AliothError;
use common::permissions::require_resource_access;
use common::ApiResponse;
use serde::Serialize;
use sqlx::PgPool;
/// 节点列表项（前端契约：id/processId/name/nodeType/order）
#[derive(Debug, Serialize)]
pub struct FlowNodeItem {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid::opt", rename = "processId")]
    pub process_id: Option<i64>,
    pub name: String,
    #[serde(rename = "nodeType")]
    pub node_type: Option<String>,
    pub order: i64,
}

/// 节点列表响应信封
#[derive(Debug, Serialize)]
pub struct FlowNodesResponse {
    pub items: Vec<FlowNodeItem>,
}

/// GET /flows/{id}/nodes — 按流程过滤、按 id 升序（派生 order）返回节点
pub async fn list_flow_nodes(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let user_id = require_auth(&req)?;
    let flow_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "flow-node", flow_id, "read").await?;

    // 流程不存在 → 404（与 version.rs 语义一致）
    let flow_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM isahl.zc_id_process WHERE id = $1 AND deleted_at IS NULL)",
    )
    .bind(flow_id)
    .fetch_one(pool.get_ref())
    .await
    .map_err(|e| AliothError::Database(e.to_string()))?;
    if !flow_exists {
        return Err(AliothError::NotFound(format!(
            "ApprovalFlow {} not found",
            flow_id
        )));
    }

    /// 行解码结构：节点列 + 类型派生列（rr.code 经桥链派生；fk_process 恒为路径 flow_id）
    #[derive(sqlx::FromRow)]
    struct NodeRow {
        id: i64,
        label: String,
        fk_process: Option<i64>,
        node_type: Option<String>,
    }

    // 节点列表三形态（2026-08-29 终端节点语义修正 + §4.4.1 类型承载）：
    // 1. event 载体节点（event 驱动 start/中间节点）：even-approve 行——类型经
    //    rr_event→op→ck_cate-proc_op→cate_proc_op.code 派生——桥成员即节点
    //    身份判据；类型取 COALESCE(t_color_, c.code, end/start 桥判定)；
    // 2. end 节点：statement 范例行经 rr_statement←op 反查（桥即 end 语义）；
    // 3. task 驱动 start：task 范例行经 rr_task←op 反查（桥即 start 语义）。
    let rows = sqlx::query_as::<_, NodeRow>(
        "SELECT n.id, n.notice AS label, $1::bigint AS fk_process, \
                COALESCE(n.t_color_, c.code, \
                         CASE WHEN EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_statement rs \
                                           WHERE rs.ref_left = o.id AND rs.deleted_at IS NULL) \
                              THEN 'end' ELSE 'start' END) AS node_type \
         FROM isahl.\"zc_id_even-approve\" n \
         JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = n.id AND oe.deleted_at IS NULL \
         JOIN isahl.zc_id_process_rr_operation rr ON rr.ref_right = oe.ref_left AND rr.ref_left = $1 AND rr.deleted_at IS NULL \
         JOIN isahl.zc_id_operation o ON o.id = rr.ref_right AND o.deleted_at IS NULL \
         LEFT JOIN isahl.\"zc_id_cate-proc_op\" c ON c.id = o.\"ck_cate-proc_op\" AND c.deleted_at IS NULL \
         WHERE n.deleted_at IS NULL \
         UNION ALL \
         SELECT s.id, s.notice AS label, $1::bigint, 'end' \
         FROM isahl.zc_id_statement s \
         JOIN isahl.zc_id_operation_rr_statement rs ON rs.ref_right = s.id AND rs.deleted_at IS NULL \
         JOIN isahl.\"zc_id_process_rr_operation\" rr2 ON rr2.ref_right = rs.ref_left AND rr2.ref_left = $1 AND rr2.deleted_at IS NULL \
         WHERE s.deleted_at IS NULL \
         UNION ALL \
         SELECT t.id, t.notice AS label, $1::bigint, 'start' \
         FROM isahl.zc_id_task t \
         JOIN isahl.zc_id_operation_rr_task rt ON rt.ref_right = t.id AND rt.deleted_at IS NULL \
         JOIN isahl.\"zc_id_process_rr_operation\" rr3 ON rr3.ref_right = rt.ref_left AND rr3.ref_left = $1 AND rr3.deleted_at IS NULL \
         WHERE t.deleted_at IS NULL \
         ORDER BY id ASC",
    )
    .bind(flow_id)
    .fetch_all(pool.get_ref())
    .await
    .map_err(|e| AliothError::Database(e.to_string()))?;

    let items: Vec<FlowNodeItem> = rows
        .into_iter()
        .enumerate()
        .map(|(idx, row)| FlowNodeItem {
            id: row.id,
            process_id: row.fk_process,
            name: row.label,
            node_type: row.node_type,
            order: idx as i64 + 1,
        })
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(FlowNodesResponse { items })))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route("/flows/{id}/nodes", web::get().to(list_flow_nodes));
}
