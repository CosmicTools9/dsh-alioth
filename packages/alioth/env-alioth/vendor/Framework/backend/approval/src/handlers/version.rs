//! 流程版本管理 Handler
//!
//! `GET /approval-flows/{id}/versions`
//! 返回流程的所有发布版本及其节点计数。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context;
use common::error::AliothError;
use common::ApiResponse;
use serde::Serialize;
use serde_json::Value;
use sqlx::{FromRow, PgPool};

use super::publish::materialize_graph;

#[derive(Debug, Serialize, FromRow)]
struct VersionInfo {
    // 版本号是发布批次计数器（1、2、3…），非 ZUID——数字序列化；
    // 历史实现误用 serde_zuid（字符串化）导致前端按数字解析失败。
    pub version: i64,
    pub published_at: String,
    pub node_count: i64,
}

/// GET /approval-flows/{id}/versions
pub async fn list_versions(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let flow_id = path.into_inner();

    // Read flow info
    let flow = sqlx::query_as::<_, (String, Option<i64>)>(
        r#"SELECT notice, tk_version FROM isahl.zc_id_process WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_optional(&**pool)
    .await
    .map_err(|e| AliothError::Database(e.to_string()))?
    .ok_or_else(|| AliothError::NotFound(format!("ApprovalFlow {} not found", flow_id)))?;

    let (name, current_version) = flow;

    // 版本批次（fix-approval-engine-semantics P1-6；2026-08-29 终端节点语义修正后
    // 重写分组载体）：批次键 = rr_operation 同事务时间戳（发布事务内所有节点行
    // 共享 NOW()——覆盖 event/task 终端与中间节点全形态）；even-approve 仅提供
    // publish_batch 版本号（含软删历史行——历史版本必须可见，不带 deleted_at
    // 过滤）；无标记的 legacy 批回退 created_at 分钟聚类（旧启发式）。
    let rows: Vec<(Option<String>, String, i64)> = sqlx::query_as(
        r#"
        SELECT
            (SELECT ea.timeline->>'publish_batch'
             FROM isahl."zc_id_operation_rr_event" oe
             JOIN isahl."zc_id_even-approve" ea
               ON ea.id = oe.ref_right
             JOIN isahl.zc_id_process_rr_operation rro2
               ON rro2.ref_right = oe.ref_left
             WHERE rro2.ref_left = rro.ref_left AND rro2.created_at = rro.created_at
             LIMIT 1) AS batch,
            TO_CHAR(rro.created_at, 'YYYY-MM-DD HH24:MI:SS') AS published_at,
            COUNT(*) AS node_count
        FROM isahl.zc_id_process_rr_operation rro
        WHERE rro.ref_left = $1
        GROUP BY rro.ref_left, rro.created_at, date_trunc('minute', rro.created_at)
        ORDER BY rro.created_at DESC
        "#,
    )
    .bind(flow_id)
    .fetch_all(&**pool)
    .await
    .map_err(|e| AliothError::Database(e.to_string()))?;

    let versions: Vec<VersionInfo> = rows
        .into_iter()
        .map(|(batch, published_at, node_count)| VersionInfo {
            // 有批次标记 → 真实版本号；legacy 批（无 rr 时间戳对齐标记）→ 0（未知版本）
            version: batch.and_then(|b| b.parse::<i64>().ok()).unwrap_or(0),
            published_at,
            node_count,
        })
        .collect();

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "id": flow_id.to_string(),
            "name": name,
            "current_version": current_version.unwrap_or(0),
            "versions": versions,
        }))),
    )
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/approval-flows/{id}/versions",
        web::get().to(list_versions),
    );
    cfg.route(
        "/approval-flows/{id}/versions/{version}/restore",
        web::post().to(restore_version),
    );
}

/// 版本恢复（fix-flow-designer-runtime-chain D6）：以历史版本图源快照
/// 重新物化为新发布批次（复用 materialize_graph）。语义：恢复 = 重新发布
/// 为最新批次，历史不可变；无快照的 legacy 版本 → 400 诚实报错。
pub async fn restore_version(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, AliothError> {
    let (flow_id, version) = path.into_inner();
    let user_id = context::require_auth(&req)?;
    let pool_ref = pool.get_ref();

    // 流程存在
    let flow: Option<(String,)> = sqlx::query_as(
        r#"SELECT notice FROM isahl.zc_id_process WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_optional(pool_ref)
    .await
    .map_err(|e| AliothError::Database(e.to_string()))?;
    let Some((flow_name,)) = flow else {
        return Err(AliothError::NotFound(format!(
            "ApprovalFlow {flow_id} not found"
        )));
    };

    // 版本图源快照（发布批次 start 节点行 timeline.graph；旧批已软删——桥与
    // 载体均不带 deleted_at 过滤，历史批次快照必须可恢复）
    let snapshot: Option<Value> = sqlx::query_scalar(
        r#"SELECT ea.timeline->'graph' FROM isahl."zc_id_even-approve" ea
           WHERE ea.timeline->>'publish_batch' = $2
             AND EXISTS (
                 -- 载体归属：同批 rro 的 created_at 与载体创建事务时间对齐
                 -- （publish 同事务 NOW() 一致；多对多 code 匹配用时间戳收敛到单批）
                 SELECT 1 FROM isahl.zc_id_process_rr_operation rro
                 WHERE rro.ref_left = $1
                   AND rro.code = ea.code
                   AND rro.created_at = ea.created_at
             )
           ORDER BY ea.id LIMIT 1"#,
    )
    .bind(flow_id)
    .bind(version.to_string())
    .fetch_optional(pool_ref)
    .await
    .map_err(|e| AliothError::Database(e.to_string()))?
    .flatten()
    .flatten();
    let Some(snapshot) = snapshot else {
        return Err(AliothError::Validation {
            field: "version".into(),
            message: format!("版本 {version} 无图源快照（legacy 批次），不可恢复"),
        });
    };

    let payload = materialize_graph(pool_ref, flow_id, user_id, &flow_name, snapshot).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(payload)))
}
