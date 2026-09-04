//! 审批流程发布 Handler
//!
//! `POST /approval-flows/{id}/publish`
//!
//! 从 ApprovalFlow.comments JSON 反序列化流程图节点和边，创建模板实例。
//! 节点行存储 next-ops 记录 DAG 边。

use crate::context_meta;
use actix_web::{web, HttpRequest, HttpResponse};
use common::context;
use common::error::AliothError as ApiError;
use common::ApiResponse;
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;

/// 审批流程发布 Handler
///
/// 从 ApprovalFlow.meta JSON 反序列化流程图节点和边，创建模板实例。
/// 节点行存储 next-ops 记录 DAG 边。
pub async fn publish_flow(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let flow_id = path.into_inner();
    let user_id = context::require_auth(&req)?;

    // 1. 读取流程 + 设计图 JSON（meta jsonb——migrate-flow-design-storage-to-meta-mermaid；
    //    结构源唯一，不回退解析 comments 惰性文本）
    let row = sqlx::query_as::<_, (String, Option<Value>)>(
        r#"SELECT notice, meta FROM isahl.zc_id_process
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_optional(&**pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .ok_or_else(|| ApiError::NotFound(format!("ApprovalFlow {} not found", flow_id)))?;

    let (flow_name, meta_json) = row;
    // 2. 设计图（jsonb 直取，无序列化解析）
    let parsed = meta_json.ok_or_else(|| ApiError::Validation {
        field: "meta".into(),
        message: "设计图缺失——存量流程请执行迁移文稿（meta ← comments 惰性 JSON）或重新保存".into(),
    })?;

    // 3. 物化（版本恢复与发布共用同一物化路径）
    let payload = materialize_graph(pool.get_ref(), flow_id, user_id, &flow_name, parsed).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(payload)))
}

/// 图物化（发布 + 版本恢复共用；fix-flow-designer-runtime-chain D6）：
/// 图校验（validate_graph）→ 版本号推进 → 旧批退役 → 节点/边物化 → 标记已发布。
/// 流程生命周期主状态桥写入（_r_status 体系；对齐实例 update_lifecycle_status 模式）：
/// find-or-create `zc_id_stus-process` 字典行 → UPSERT
/// `zc_id_lifecycle_r_primary-status` 桥（ref_left=流程行）→ 状态变更审计。
/// 事务版（发布路径，与图物化同批提交）。
pub(crate) async fn update_flow_lifecycle_status_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    flow_id: i64,
    status_code: &str,
    status_notice: &str,
    user_id: i64,
) -> Result<(), ApiError> {
    let status_id: i64 = match sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT id FROM isahl."zc_id_stus-process" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(status_code)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten()
    {
        Some(id) => id,
        None => {
            sqlx::query_scalar::<_, i64>(
                r#"INSERT INTO isahl."zc_id_stus-process" (id, code, notice)
                   VALUES (isahl.gen_next_zuid(), $1, $2) RETURNING id"#,
            )
            .bind(status_code)
            .bind(status_notice)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?
        }
    };
    let row = crud::audit_outbox::fetch_primary_status_row_tx(tx, flow_id)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
    let old_status = row.filter(|(_, active)| *active).map(|(s, _)| s);
    match row {
        Some((_, true)) => {
            sqlx::query(
                r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
                   SET ref_right = $1, updated_at = NOW()
                   WHERE ref_left = $2 AND deleted_at IS NULL"#,
            )
            .bind(status_id)
            .bind(flow_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?;
        }
        Some((_, false)) => {
            sqlx::query(
                r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
                   SET ref_right = $1, deleted_at = NULL, updated_at = NOW()
                   WHERE ref_left = $2 AND deleted_at IS NOT NULL"#,
            )
            .bind(status_id)
            .bind(flow_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?;
        }
        None => {
            sqlx::query(
                r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (id, ref_left, ref_right)
                   VALUES (isahl.gen_next_zuid(), $1, $2)"#,
            )
            .bind(flow_id)
            .bind(status_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?;
        }
    }
    // 审计为尽力而为：SAVEPOINT 隔离——失败回滚到保存点恢复事务，
    // 不得因 audit 写入失败 abort 主事务（实测 audit_outbox id 缺默认致
    // INSERT 报错 → 事务 aborted → commit 静默回滚，物化数据丢失）。
    sqlx::query(r#"SAVEPOINT approval_flow_status_audit"#)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
    let audit_res = crud::audit_outbox::audit_primary_status_tx(
        tx,
        flow_id,
        old_status,
        status_id,
        Some(user_id),
    )
    .await;
    match audit_res {
        Ok(_) => {
            sqlx::query(r#"RELEASE SAVEPOINT approval_flow_status_audit"#)
                .execute(&mut **tx)
                .await
                .map_err(|e| ApiError::Database(e.to_string()))?;
        }
        Err(e) => {
            sqlx::query(r#"ROLLBACK TO SAVEPOINT approval_flow_status_audit"#)
                .execute(&mut **tx)
                .await
                .map_err(|e2| ApiError::Database(e2.to_string()))?;
            common::telemetry::warn!(
                "audit_primary_status enqueue failed (approval flow {}): {}",
                flow_id,
                e
            );
        }
    }
    Ok(())
}

/// 流程生命周期主状态桥写入（pool 版，停用路径——无既有事务）。
pub(crate) async fn update_flow_lifecycle_status(
    pool: &PgPool,
    flow_id: i64,
    status_code: &str,
    status_notice: &str,
    user_id: i64,
) -> Result<(), ApiError> {
    let status_id: i64 = match sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT id FROM isahl."zc_id_stus-process" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(status_code)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten()
    {
        Some(id) => id,
        None => {
            sqlx::query_scalar::<_, i64>(
                r#"INSERT INTO isahl."zc_id_stus-process" (id, code, notice)
                   VALUES (isahl.gen_next_zuid(), $1, $2) RETURNING id"#,
            )
            .bind(status_code)
            .bind(status_notice)
            .fetch_one(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?
        }
    };
    let row = crud::audit_outbox::fetch_primary_status_row(pool, flow_id)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
    let old_status = row.filter(|(_, active)| *active).map(|(s, _)| s);
    match row {
        Some((_, true)) => {
            sqlx::query(
                r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
                   SET ref_right = $1, updated_at = NOW()
                   WHERE ref_left = $2 AND deleted_at IS NULL"#,
            )
            .bind(status_id)
            .bind(flow_id)
            .execute(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?;
        }
        Some((_, false)) => {
            sqlx::query(
                r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
                   SET ref_right = $1, deleted_at = NULL, updated_at = NOW()
                   WHERE ref_left = $2 AND deleted_at IS NOT NULL"#,
            )
            .bind(status_id)
            .bind(flow_id)
            .execute(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?;
        }
        None => {
            sqlx::query(
                r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (id, ref_left, ref_right)
                   VALUES (isahl.gen_next_zuid(), $1, $2)"#,
            )
            .bind(flow_id)
            .bind(status_id)
            .execute(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?;
        }
    }
    if let Err(e) = crud::audit_outbox::audit_primary_status(
        pool,
        flow_id,
        old_status,
        status_id,
        Some(user_id),
    )
    .await
    {
        common::telemetry::warn!(
            "audit_primary_status enqueue failed (approval flow {}): {}",
            flow_id,
            e
        );
    }
    Ok(())
}

pub(crate) async fn materialize_graph(
    pool: &PgPool,
    flow_id: i64,
    user_id: i64,
    flow_name: &str,
    parsed: Value,
) -> Result<Value, ApiError> {
    let (nodes, edges_opt) = validate_graph(&parsed)?;
    let edges = edges_opt.unwrap_or(&[]);

    // 图级节点键：优先非空 `id`，缺失/空串回退节点下标。键派生只有这一处——
    // 边推导与节点物化必须用同一规则，否则无 id 节点键不一致（"" 碰撞）导致
    // next-ops 解析丢失目标边。
    let node_key = |node: &Value, idx: usize| -> String {
        node.get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| idx.to_string())
    };

    // 边目标：图级键 + 可选 cond/label（P1-5 条件选边的物化载体）
    #[derive(Clone)]
    struct EdgeTarget {
        key: String,
        cond: Option<String>,
        label: Option<String>,
    }

    // 构建图级 ID → 下一图级 ID 列表
    let mut edge_map: HashMap<String, Vec<EdgeTarget>> = HashMap::new();
    for edge in edges {
        let source = edge.get("source").and_then(|v| v.as_str());
        let target = edge.get("target").and_then(|v| v.as_str());
        if let (Some(s), Some(t)) = (source, target) {
            if s != t {
                edge_map.entry(s.to_string()).or_default().push(EdgeTarget {
                    key: t.to_string(),
                    cond: edge
                        .get("cond")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    label: edge
                        .get("label")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                });
            }
        }
    }
    if edge_map.is_empty() {
        for (idx, node) in nodes.iter().enumerate() {
            let source = node_key(node, idx);
            if let Some(next_arr) = node.get("next").and_then(|v| v.as_array()) {
                for nxt in next_arr {
                    let to_idx = nxt.get("to").and_then(|v| v.as_i64());
                    if let Some(t) = to_idx {
                        if t >= 0 && (t as usize) < nodes.len() && t as usize != idx {
                            let target = node_key(&nodes[t as usize], t as usize);
                            edge_map
                                .entry(source.clone())
                                .or_default()
                                .push(EdgeTarget {
                                    key: target,
                                    cond: nxt
                                        .get("cond")
                                        .and_then(|v| v.as_str())
                                        .map(str::to_string),
                                    label: nxt
                                        .get("label")
                                        .and_then(|v| v.as_str())
                                        .map(str::to_string),
                                });
                        }
                    }
                }
            }
        }
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;

    // 3.0 版本批次与旧 DAG 退役（fix-approval-engine-semantics P1-6）
    // 先锁定流程行并计算新版本号（本批节点的 timeline.publish_batch 标记）。
    let current_version: Option<i64> = sqlx::query_scalar(
        r#"SELECT tk_version FROM isahl.zc_id_process WHERE id = $1 FOR UPDATE"#,
    )
    .bind(flow_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten();
    let new_version = current_version.unwrap_or(0) + 1;

    // 在途守卫：旧 DAG 节点被非终态实例引用时拒绝发布。2026-08-31 契约后
    // 节点语义实体多样（event 驱动 start → 事件真叶表、end → statement），
    // 实例经 rr_event 挂各自载体——守卫按「实例桥 → rro 桥链反查流程」判定，
    // 不依赖载体表类型（even-approve join 已移除，覆盖事件叶表/statement 全形态）。
    let inflight: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           WHERE EXISTS (
                 SELECT 1 FROM isahl.zc_id_operation_rr_event oe
                 JOIN isahl.zc_id_operation_rr_event oe2
                   ON oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL
                 JOIN isahl.zc_id_process_rr_operation rro2
                   ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL
                 WHERE oe.ref_left = oa.id AND oe.deleted_at IS NULL
                   AND rro2.ref_left = $1
             )
             AND oa.deleted_at IS NULL
             AND oa.tpl_id IS NOT NULL
             AND NOT EXISTS (
                 SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                 JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
                 WHERE ls.ref_left = oa.id AND ls.deleted_at IS NULL
                   AND s.code IN ('approved','rejected','withdrawn','cancelled','abstained')
             )"#,
    )
    .bind(flow_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    if inflight > 0 {
        return Err(ApiError::Validation {
            field: "flow".into(),
            message: format!(
                "flow {} has {} in-flight instance(s) on the current DAG — settle or withdraw them before re-publishing",
                flow_id, inflight
            ),
        });
    }

    // 旧批退役：软删既有节点行与关联行（历史实例经 rr_event 桥引用，
    // 读路径经 deleted_at 过滤不再回放旧 DAG）。
    sqlx::query(
        r#"UPDATE isahl."zc_id_even-approve"
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE id IN (
                 SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
                 JOIN isahl.zc_id_process_rr_operation rro
                   ON rro.ref_right = oe.ref_left AND rro.deleted_at IS NULL
                 WHERE oe.deleted_at IS NULL AND rro.ref_left = $1
             ) AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old nodes: {}", e)))?;
    // 旧批退役：终端节点语义实体（2026-08-29 裁决）——end 的 statement 范例行与
    // rr_statement 桥、task 驱动 start 的 task 范例行与 rr_task 桥。定位链：
    // 本流程在册 rr_operation → ref_right=节点 op 行 → 桥 → 范例行（类型无关，
    // 旧批整体退役；节点类型不再由 rr_operation.code 承载，见 §4.4.1）。
    // 实例侧桥行 ref_left=实例 id（gate/审批实例），不在本定位集，不受影响。
    // 范例行先退役（经在册桥定位），桥行后退役。
    sqlx::query(
        r#"UPDATE isahl.zc_id_statement
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE id IN (SELECT rs.ref_right FROM isahl.zc_id_operation_rr_statement rs
                        WHERE rs.ref_left IN (SELECT ref_right FROM isahl.zc_id_process_rr_operation
                                              WHERE ref_left = $1
                                                AND deleted_at IS NULL)
                          AND rs.deleted_at IS NULL)
             AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old end statements: {}", e)))?;
    sqlx::query(
        r#"UPDATE isahl.zc_id_operation_rr_statement
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE ref_left IN (SELECT ref_right FROM isahl.zc_id_process_rr_operation
                              WHERE ref_left = $1 AND deleted_at IS NULL)
             AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old end statement bridges: {}", e)))?;
    sqlx::query(
        r#"UPDATE isahl.zc_id_task
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE id IN (SELECT rt.ref_right FROM isahl.zc_id_operation_rr_task rt
                        WHERE rt.ref_left IN (SELECT ref_right FROM isahl.zc_id_process_rr_operation
                                              WHERE ref_left = $1
                                                AND deleted_at IS NULL)
                          AND rt.deleted_at IS NULL)
             AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old start tasks: {}", e)))?;
    sqlx::query(
        r#"UPDATE isahl.zc_id_operation_rr_task
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE ref_left IN (SELECT ref_right FROM isahl.zc_id_process_rr_operation
                              WHERE ref_left = $1 AND deleted_at IS NULL)
             AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old start task bridges: {}", e)))?;
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old edges: {}", e)))?;
    // 旧批退役：操作叶行 + 桥（fix-avic-approval-node-model 新模型产物）。
    // 定位改经 rr_operation（本流程全部历史 DAG op 行，软删幂等——已退役行重复
    // UPDATE 无害）；原 even-approve 定位器在 even-approve 退役后恒空（死定位器，
    // 旧批 op 行/岗位桥从未真正退役），且不覆盖 end/task-start 等无事件节点。
    // 实例行（gate/审批实例）不入 rr_operation.ref_right，不受本定位影响。
    sqlx::query(
        r#"UPDATE isahl.zc_id_operation_rr_approve
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE ref_left IN (SELECT ref_right FROM isahl.zc_id_process_rr_operation
                              WHERE ref_left = $1)
             AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old node approvers: {}", e)))?;
    sqlx::query(
        r#"UPDATE isahl.zc_id_operation_rr_review
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE ref_left IN (SELECT ref_right FROM isahl.zc_id_process_rr_operation
                              WHERE ref_left = $1)
             AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old node reviewers: {}", e)))?;
    sqlx::query(
        r#"UPDATE isahl.zc_id_operation_rr_post
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE ref_left IN (SELECT ref_right FROM isahl.zc_id_process_rr_operation
                              WHERE ref_left = $1)
             AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old node posters: {}", e)))?;
    sqlx::query(
        r#"UPDATE isahl.zc_id_operation_rr_event
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE ref_left IN (SELECT ref_right FROM isahl.zc_id_process_rr_operation
                              WHERE ref_left = $1)
             AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old node event bridges: {}", e)))?;
    sqlx::query(
        r#"UPDATE isahl.zc_id_operation
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE id IN (SELECT ref_right FROM isahl.zc_id_process_rr_operation
                        WHERE ref_left = $1)
             AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old node operations: {}", e)))?;
    // 旧批退役：loop 公式链（refactor-flow-loop-formula-model）——standard
    // 实现·实例 + operation_rr_standard + standard_r_formula + formula 行
    // （经 rr_operation 定位本流程历史 op 行 → rr_standard 桥 → standard 实例；
    // 实例经 r_formula 桥 → formula）。standard 范例（tpl_id IS NULL）保留
    // （find-or-create 幂等复用），仅软删实例与桥。
    sqlx::query(
        r#"UPDATE isahl.zc_id_standard_r_formula
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE ref_left IN (SELECT rs.ref_right FROM isahl.zc_id_operation_rr_standard rs
                              WHERE rs.ref_left IN (SELECT ref_right FROM isahl.zc_id_process_rr_operation
                                                    WHERE ref_left = $1)
                                AND rs.deleted_at IS NULL)
             AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old std r_formula: {}", e)))?;
    sqlx::query(
        r#"UPDATE isahl.zc_id_operation_rr_standard
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE ref_left IN (SELECT ref_right FROM isahl.zc_id_process_rr_operation
                              WHERE ref_left = $1)
             AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old node rr_standard: {}", e)))?;
    sqlx::query(
        r#"UPDATE isahl.zc_id_standard
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE id IN (SELECT rs.ref_right FROM isahl.zc_id_operation_rr_standard rs
                        WHERE rs.ref_left IN (SELECT ref_right FROM isahl.zc_id_process_rr_operation
                                              WHERE ref_left = $1)
                          AND rs.deleted_at IS NULL)
             AND deleted_at IS NULL AND tpl_id IS NOT NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old std instances: {}", e)))?;
    sqlx::query(
        r#"UPDATE isahl.zc_id_formula
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE id IN (SELECT rf.ref_right FROM isahl.zc_id_standard_r_formula rf
                        WHERE rf.ref_left IN (SELECT rs.ref_right FROM isahl.zc_id_operation_rr_standard rs
                                              WHERE rs.ref_left IN (SELECT ref_right FROM isahl.zc_id_process_rr_operation
                                                                    WHERE ref_left = $1)
                                                AND rs.deleted_at IS NULL)
                          AND rf.deleted_at IS NULL)
             AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(format!("retire old loop formulas: {}", e)))?;
    // 3. 遍历图节点，创建 even-approve 行
    // 3. 遍历图节点，创建 even-approve 行
    let mut graph_to_db: HashMap<String, i64> = HashMap::new();
    let mut node_map: Vec<Value> = Vec::with_capacity(nodes.len());
    // 版本图快照载体（D6）：挂本批首个 even-approve 节点（终端节点语义修正后
    // task-start/end 无事件行；全图无载体 → 发布诚实报错）
    let mut snapshot_written = false;
    let mut graph_snapshot_done = false;

    for (idx, node) in nodes.iter().enumerate() {
        let node_type = node
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let label = node.get("label").and_then(|v| v.as_str()).unwrap_or("");
        let graph_id = node_key(node, idx);

        let sla_hours = node.get("sla").and_then(|v| v.as_i64());
        let sla_id: Option<i64> = match sla_hours {
            Some(h) => {
                // SLA 小时数存 zc_id_scal-duration.mark（o_number 为触发器自动生成的业务编号，
                // 不承载数值；与全仓 ScalarService/license 模式一致）。find-or-create 语义：
                // 按 mark 查 → 命中复用；未命中创建后返回新 id。
                let mark = rust_decimal::Decimal::from(h);
                if let Some(id) = sqlx::query_scalar(
                    r#"SELECT id FROM isahl."zc_id_scal-duration"
                       WHERE mark = $1 AND deleted_at IS NULL LIMIT 1"#,
                )
                .bind(mark)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("lookup sla_dur[{}]: {}", idx, e)))?
                {
                    Some(id)
                } else {
                    Some(
                        sqlx::query_scalar(
                            r#"INSERT INTO isahl."zc_id_scal-duration" (notice, mark, created_by_id)
                               VALUES ($1, $2, 1) RETURNING id"#,
                        )
                        .bind(format!("sla: {}h", h))
                        .bind(mark)
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(|e| {
                            ApiError::Database(format!("create sla_dur[{}]: {}", idx, e))
                        })?,
                    )
                }
            }
            None => None,
        };

        // 节点语义实体判定（2026-08-29 裁决）：start=event/task、end=statement；
        // 中间节点与 event 驱动 start 保持 even-approve 事件模板（timeline/SLA 载体）。
        // 终端实体叶表在设计器显式配置（白名单 fail-closed，INSERT 落叶表铁律）。
        let drive = node
            .get("drive")
            .and_then(|v: &Value| v.as_str())
            .unwrap_or("event");
        let is_task_start = node_type == "start" && drive == "task";
        let is_event_start = node_type == "start" && drive != "task";
        let is_end = node_type == "end";

        let mut timeline = serde_json::json!({ "publish_batch": new_version });
        if node_type == "cc" {
            // A6：结构化收件人优先（recipientRefs 数组）；recipients 文本兼容保留
            let refs = node
                .get("recipientRefs")
                .and_then(|v: &Value| v.as_array())
                .cloned();
            if let Some(refs) = refs.filter(|r| !r.is_empty()) {
                for item in &refs {
                    let kind = item.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                    let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    if !matches!(kind, "role" | "employee" | "engineer") || id.is_empty() {
                        return Err(ApiError::Validation {
                            field: "nodes".into(),
                            message: format!(
                                "node[{}] '{}' (cc) recipientRefs 项非法——kind 须 role/employee（engineer 兼容）且 id 非空",
                                idx, label
                            ),
                        });
                    }
                }
                timeline
                    .as_object_mut()
                    .expect("timeline json object")
                    .insert("recipients".into(), serde_json::Value::Array(refs));
            } else if let Some(recipients) = node.get("recipients").and_then(|v: &Value| v.as_str())
            {
                timeline
                    .as_object_mut()
                    .expect("timeline json object")
                    .insert("recipients".into(), serde_json::json!(recipients));
            }
        }
        // 2026-09-01 能力补齐：branch 汇聚规则物化（all/any）
        if node_type == "branch" {
            if let Some(join_rule) = node.get("joinRule").and_then(|v: &Value| v.as_str()) {
                timeline
                    .as_object_mut()
                    .expect("timeline json object")
                    .insert("joinRule".into(), serde_json::json!(join_rule));
            }
        }
        // 2026-09-01 能力补齐：loop 循环条件与最大次数物化
        if node_type == "loop" {
            let obj = timeline.as_object_mut().expect("timeline json object");
            if let Some(expr) = node.get("loopExpr").and_then(|v: &Value| v.as_str()) {
                obj.insert("loopExpr".into(), serde_json::json!(expr));
            }
            if let Some(iter) = node.get("maxIter").and_then(|v: &Value| v.as_i64()) {
                obj.insert("loopMaxIter".into(), serde_json::json!(iter));
            }
        }
        // 2026-09-01 能力补齐：subflow target 物化（运行时 advance 触发被引用流程）
        if node_type == "subflow" {
            if let Some(target) = node.get("target").and_then(|v: &Value| v.as_str()) {
                timeline
                    .as_object_mut()
                    .expect("timeline json object")
                    .insert("target".into(), serde_json::json!(target));
            }
        }
        // 2026-09-02 A4：subflow wait=true（同步等待子流程终局；end 物化回调续链）
        if node_type == "subflow"
            && node.get("wait").and_then(|v: &Value| v.as_bool()) == Some(true)
        {
            timeline
                .as_object_mut()
                .expect("timeline json object")
                .insert("wait".into(), serde_json::json!(true));
        }
        // 2026-09-02 能力补齐：vote quorum 物化（0 = 全部投票人；运行时 SignMode::Vote
        // 门控经 advance vote_quorum 读取；缺省/非法按全员解析）
        if node_type == "vote" {
            if let Some(q) = node.get("quorum").and_then(|v: &Value| v.as_i64()) {
                timeline
                    .as_object_mut()
                    .expect("timeline json object")
                    .insert("quorum".into(), serde_json::json!(q));
            }
        }
        // 2026-09-02 A5：驳回路由物化（rejectAction=stop/back；backTo=目标节点下标，
        // 运行时经 process.meta.nodes[backTo].id 解析目标 op——免双段解析）
        if matches!(
            node_type,
            "approval" | "approve" | "oper-approve" | "review" | "action" | "vote"
        ) {
            if let Some(ra) = node.get("rejectAction").and_then(|v: &Value| v.as_str()) {
                if !matches!(ra, "stop" | "back") {
                    return Err(ApiError::Validation {
                        field: "nodes".into(),
                        message: format!(
                            "node[{}] '{}' rejectAction '{}' 非法——须为 stop/back",
                            idx, label, ra
                        ),
                    });
                }
                timeline
                    .as_object_mut()
                    .expect("timeline json object")
                    .insert("rejectAction".into(), serde_json::json!(ra));
                if ra == "back" {
                    if let Some(bt) = node.get("backTo").and_then(|v: &Value| v.as_i64()) {
                        timeline
                            .as_object_mut()
                            .expect("timeline json object")
                            .insert("backTo".into(), serde_json::json!(bt));
                    }
                }
            }
        }
        // 2026-09-03 升级岗位物化：roleEscalate（position id，新语义）优先，
        // 旧 escalateTo（岗位名）读取兼容——timeline.escalateTo 以岗位名承载，
        // SLA 超时升级（sla_timeout）经其投递。
        if matches!(
            node_type,
            "approval" | "approve" | "oper-approve" | "review" | "action" | "vote"
        ) {
            let esc_name: Option<String> = node
                .get("roleEscalate")
                .and_then(|v| v.as_str())
                .filter(|v| !v.trim().is_empty())
                .map(|v| v.trim().to_string());
            let esc_name = if let Some(id) = esc_name {
                sqlx::query_scalar::<_, String>(
                    r#"SELECT notice FROM isahl."zc_id_subj-position"
                       WHERE id = $1::bigint AND deleted_at IS NULL LIMIT 1"#,
                )
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("escalate position name: {}", e)))?
            } else {
                node.get("escalateTo")
                    .and_then(|v: &Value| v.as_str())
                    .map(|v| v.trim().to_string())
            };
            if let Some(name) = esc_name {
                if !name.is_empty() {
                    timeline
                        .as_object_mut()
                        .expect("timeline json object")
                        .insert("escalateTo".into(), serde_json::json!(name));
                }
            }
            // 备选触发阈值物化（2026-09-03）：节点 backupThreshold（缺省 10）随载体落库，
            // 运行时（resolve_node_assign）以 operation.meta/载体值为准——见 node_meta.rs。
            if let Some(bt) = node.get("backupThreshold").and_then(|v: &Value| v.as_i64()) {
                timeline
                    .as_object_mut()
                    .expect("timeline json object")
                    .insert("backupThreshold".into(), serde_json::json!(bt));
            }
        }

        // 2026-09-02 加权（岗位制多源，不引入项目/群组岗位模型）：源=现有主体
        // （UA role / 岗位人员 employee），逐源解析用户；权重表物化 timeline。
        if node_type == "vote" {
            if let Some(srcs) = node.get("voteSources").and_then(|v: &Value| v.as_array()) {
                if srcs.is_empty() {
                    return Err(ApiError::Validation {
                        field: "nodes".into(),
                        message: format!(
                            "node[{}] '{}' (vote) voteSources 不能为空数组",
                            idx, label
                        ),
                    });
                }
                let mut resolved: Vec<serde_json::Value> = Vec::new();
                for item in srcs {
                    let kind = item.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                    let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let weight = item.get("weight").and_then(|v| v.as_i64()).unwrap_or(1);
                    if !matches!(kind, "role" | "employee" | "engineer") || id.is_empty() {
                        return Err(ApiError::Validation {
                            field: "nodes".into(),
                            message: format!(
                                "node[{}] '{}' (vote) voteSources 项非法——kind 须 role/employee（engineer 兼容）且 id 非空",
                                idx, label
                            ),
                        });
                    }
                    if weight < 1 {
                        return Err(ApiError::Validation {
                            field: "nodes".into(),
                            message: format!("node[{}] '{}' (vote) 源权重须 ≥1", idx, label),
                        });
                    }
                    let users: Vec<i64> = if kind != "role" {
                        sqlx::query_scalar(
                            r#"SELECT u.id FROM isahl_auth.auth_users u
                               WHERE u.is_active = TRUE AND (u.id IN (SELECT e.fk_user FROM isahl."zc_id_subj-employee" e WHERE e.deleted_at IS NULL AND (e.id::text = $1 OR e.notice = $1 OR e.code = $1)) OR u.username = $1 OR u.name = $1)
                               LIMIT 50"#,
                        )
                        .bind(id)
                        .fetch_all(&mut *tx)
                        .await
                        .map_err(|e| ApiError::Database(format!("resolve vote engineer[{}]: {}", idx, e)))?
                                            } else {
                            // 岗位类别成员经 common::ngac_org 收敛解析（指派 UA ∪ 岗位持有者）
                            common::ngac_org::resolve_member_user_ids(&mut *tx, id, 200).await
                        };
                    for uid in users {
                        resolved.push(serde_json::json!({ "uid": uid, "weight": weight }));
                    }
                }
                timeline
                    .as_object_mut()
                    .expect("timeline json object")
                    .insert("resolvedWeights".into(), serde_json::Value::Array(resolved));
            }
        }

        // 2026-09-02 A3：quorumPct 百分位阈值（1..=100；与 quorum 互斥，validate 层拦）
        if node_type == "vote" {
            if let Some(pct) = node.get("quorumPct").and_then(|v: &Value| v.as_i64()) {
                if !(1..=100).contains(&pct) {
                    return Err(ApiError::Validation {
                        field: "nodes".into(),
                        message: format!(
                            "node[{}] '{}' (vote) quorumPct '{}' 非法——须 1..=100",
                            idx, label, pct
                        ),
                    });
                }
                timeline
                    .as_object_mut()
                    .expect("timeline json object")
                    .insert("quorumPct".into(), serde_json::json!(pct));
            }
        }
        // 2026-09-02 fix-flow-gateway-semantics A2：condition routing 物化
        // （exclusive=排他首中 / inclusive=全命中扇出；缺省 = 存量 inclusive）
        if node_type == "condition" {
            if let Some(r) = node.get("routing").and_then(|v: &Value| v.as_str()) {
                if !matches!(r, "exclusive" | "inclusive") {
                    return Err(ApiError::Validation {
                        field: "nodes".into(),
                        message: format!(
                            "node[{}] '{}' (condition) routing '{}' 非法——须为 exclusive/inclusive",
                            idx, label, r
                        ),
                    });
                }
                timeline
                    .as_object_mut()
                    .expect("timeline json object")
                    .insert("routing".into(), serde_json::json!(r));
            }
        }
        // 2026-09-01 能力补齐：subflow 引用校验——target 流程 code 必须存在且
        // 主状态桥为 published；自引用（target = 本流程 code）拒绝（无限递归）。
        if node_type == "subflow" {
            let target = node
                .get("target")
                .and_then(|v: &Value| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| ApiError::Validation {
                    field: "nodes".into(),
                    message: format!(
                        "node[{}] '{}' (subflow) 缺 target 配置——子流程节点须引用已发布流程的 code",
                        idx, label
                    ),
                })?;
            let self_code: Option<String> = sqlx::query_scalar(
                r#"SELECT code FROM isahl.zc_id_process WHERE id = $1 AND deleted_at IS NULL"#,
            )
            .bind(flow_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| ApiError::Database(format!("subflow self code[{}]: {}", idx, e)))?
            .flatten();
            if self_code.as_deref().map(str::trim) == Some(target.trim()) {
                return Err(ApiError::Validation {
                    field: "nodes".into(),
                    message: format!(
                        "node[{}] '{}' (subflow) target 不能引用本流程自身（code '{}' 递归）",
                        idx, label, target
                    ),
                });
            }
            let target_ok: Option<(i64, bool)> = sqlx::query_as(
                r#"SELECT p.id,
                          EXISTS (SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                                  JOIN isahl."zc_id_stus-process" s ON s.id = ls.ref_right
                                  WHERE ls.ref_left = p.id AND ls.deleted_at IS NULL
                                    AND s.code = 'published') AS published
                   FROM isahl.zc_id_process p
                   WHERE p.code = $1 AND p.deleted_at IS NULL
                   ORDER BY CASE WHEN p._f_ = '实现'
                                    AND (p._t_ = '范例' OR p._t_ IS NULL)
                                THEN 0 ELSE 1 END,
                            p.updated_at DESC
                   LIMIT 1"#,
            )
            .bind(target.trim())
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| ApiError::Database(format!("subflow target lookup[{}]: {}", idx, e)))?;
            match target_ok {
                Some((_, true)) => {
                    // A4：wait=true 要求子流程可达终局（图含 end 节点）——否则父流程无限等待
                    if node.get("wait").and_then(|v: &Value| v.as_bool()) == Some(true) {
                        let has_end: bool = sqlx::query_scalar::<_, bool>(
                            r#"SELECT EXISTS (
                                SELECT 1 FROM isahl.zc_id_process p2
                                CROSS JOIN jsonb_array_elements(p2.meta->'nodes') n
                                WHERE p2.code = $1 AND p2.deleted_at IS NULL
                                  AND n->>'type' = 'end'
                            )"#,
                        )
                        .bind(target.trim())
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(|e| ApiError::Database(format!("subflow wait end check: {}", e)))?
                        .unwrap_or(false);
                        if !has_end {
                            return Err(ApiError::Validation {
                                field: "nodes".into(),
                                message: format!(
                                    "node[{}] '{}' (subflow) wait=true 要求 target '{}' 含 end 终局节点",
                                    idx, label, target
                                ),
                            });
                        }
                    }
                }
                Some((_, false)) => {
                    return Err(ApiError::Validation {
                        field: "nodes".into(),
                        message: format!(
                            "node[{}] '{}' (subflow) target '{}' 存在但未发布",
                            idx, label, target
                        ),
                    })
                }
                None => {
                    return Err(ApiError::Validation {
                        field: "nodes".into(),
                        message: format!(
                            "node[{}] '{}' (subflow) target '{}' 不存在",
                            idx, label, target
                        ),
                    })
                }
            }
        }

        // 1. 节点语义实体行：end→statement 叶表范例 / task 驱动 start→task 叶表范例 /
        //    其余（event 驱动 start/中间节点）→ even-approve 事件模板（timeline/SLA 载体）。
        //    终端范例 tpl_id=NULL（模板本体）；运行时实例 tpl_id→范例（同表关联）。
        let mut terminal_entity: Option<(&str, i64)> = None;
        let mut template_id: Option<i64> = None;
        if is_end {
            let leaf = node
                .get("statementLeaf")
                .and_then(|v: &Value| v.as_str())
                .ok_or_else(|| ApiError::Validation {
                    field: "nodes".into(),
                    message: format!(
                        "node[{}] '{}' (end) 缺 statementLeaf 配置——流程结束节点须在设计器显式配置结论承载叶表",
                        idx, label
                    ),
                })?;
            if !context_meta::is_statement_leaf(leaf) {
                return Err(ApiError::Validation {
                    field: "nodes".into(),
                    message: format!(
                        "node[{}] statementLeaf '{}' 不在 statement 真叶表白名单",
                        idx, leaf
                    ),
                });
            }
            let insert_sql = context_meta::statement_leaf_insert_sql(leaf)
                .expect("whitelisted statement leaf has insert arm");
            let row_id: i64 = sqlx::query_scalar(insert_sql)
                .bind(label)
                .bind(&graph_id)
                .bind(Option::<i64>::None)
                .bind(user_id)
                // 类写入契约 §4.3.3：发布物化 = 实现·范例（形态 2 显式字面量对）
                .bind("实现")
                .bind("范例")
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("node statement[{}]: {}", idx, e)))?;
            terminal_entity = Some(("statement", row_id));
        } else if is_task_start {
            let leaf = node
                .get("taskLeaf")
                .and_then(|v: &Value| v.as_str())
                .ok_or_else(|| ApiError::Validation {
                    field: "nodes".into(),
                    message: format!(
                        "node[{}] '{}' (start, task 驱动) 缺 taskLeaf 配置——task 驱动开始节点须显式配置 task 叶表",
                        idx, label
                    ),
                })?;
            if !context_meta::is_task_leaf(leaf) {
                return Err(ApiError::Validation {
                    field: "nodes".into(),
                    message: format!("node[{}] taskLeaf '{}' 不在 task 真叶表白名单", idx, leaf),
                });
            }
            let insert_sql = context_meta::task_leaf_insert_sql(leaf)
                .expect("whitelisted task leaf has insert arm");
            let row_id: i64 = sqlx::query_scalar(insert_sql)
                .bind(label)
                .bind(&graph_id)
                .bind(Option::<i64>::None)
                .bind(user_id)
                // 类写入契约 §4.3.3：发布物化 = 实现·范例
                .bind("实现")
                .bind("范例")
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("node task[{}]: {}", idx, e)))?;
            terminal_entity = Some(("task", row_id));
        } else {
            let tid: i64 = sqlx::query_scalar(
                r#"INSERT INTO isahl."zc_id_even-approve"
                   (notice, created_by_id, code, qk_sla, comments, timeline)
                   VALUES ($1, $2, $3, $4, $5, $6)
                   RETURNING id"#,
            )
            .bind(label)
            .bind(user_id)
            .bind(&graph_id)
            .bind(sla_id)
            // 节点 meta 不再入 comments（comments-text-semantics；fix-avic-approval-node-model）：
            // 审批人/签署模式物化到操作模型表，见下方节点接线块。
            .bind(Option::<String>::None)
            .bind(timeline)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| ApiError::Database(format!("node template[{}]: {}", idx, e)))?;
            template_id = Some(tid);
            snapshot_written = true;
            // 图源快照挂首个 even-approve 载体（restore 经 rro.code 关联定位——
            // event/task start 与 end 载体无 rr_event 桥，但 code=graph_id 与
            // rro.code 一致，可关联恢复）
            if !graph_snapshot_done {
                sqlx::query(
                    r#"UPDATE isahl."zc_id_even-approve"
                       SET timeline = timeline || $2::jsonb
                       WHERE id = $1 AND deleted_at IS NULL"#,
                )
                .bind(tid)
                .bind(serde_json::json!({ "graph": parsed }))
                .execute(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("snapshot graph[{}]: {}", idx, e)))?;
                graph_snapshot_done = true;
            }
            // 2026-08-31 缺口补全：event 驱动 start 在 even-approve 载体行
            // （timeline/SLA/版本快照载体）之外，追加物化事件真叶表范例行
            // （实现·范例，rr_event 直挂叶表行；此前 EVENT 族既不校验也不物化）
            if is_event_start {
                let leaf = node
                    .get("eventLeaf")
                    .and_then(|v: &Value| v.as_str())
                    .ok_or_else(|| ApiError::Validation {
                        field: "nodes".into(),
                        message: format!(
                            "node[{}] '{}' (start, event 驱动) 缺 eventLeaf 配置——event 驱动开始节点须显式配置事件叶表",
                            idx, label
                        ),
                    })?;
                if !context_meta::EVENT_LEAVES.iter().any(|i| i.table == leaf) {
                    return Err(ApiError::Validation {
                        field: "nodes".into(),
                        message: format!(
                            "node[{}] eventLeaf '{}' 不在 event 真叶表白名单",
                            idx, leaf
                        ),
                    });
                }
                let insert_sql = context_meta::event_leaf_insert_sql(leaf)
                    .expect("whitelisted event leaf has insert arm");
                let row_id: i64 = sqlx::query_scalar(insert_sql)
                    .bind(label)
                    .bind(&graph_id)
                    .bind(Option::<i64>::None)
                    .bind(user_id)
                    .bind("实现")
                    .bind("范例")
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|e| ApiError::Database(format!("node event[{}]: {}", idx, e)))?;
                terminal_entity = Some(("event", row_id));
            }
        }

        // 2. operation 节点主体（refactor-flow-node-operation-model：节点=操作）。
        //    动作子类选择：approve→oper-approve / action→oper-action / review 与
        //    自动节点→operation 基表（类型经 rr_operation.code=node_type 标注）。
        // 范例 operation 行 = 实现·范例（类写入契约 §4.3.3 形态 2；实例行由
        // advance 执行期物化为 实现·实例，tpl_id 回挂本范例行）
        let op_id: i64 = match node_type {
            // vote 复用 oper-approve 实例链路（我的审批待办可见；投票人经 rr_approve 桥）
            "approval" | "approve" | "oper-approve" | "vote" => {
                // 备选阈值随范例行物化（2026-09-03）：operation.meta 为运行时读取载体
                // （resolve_node_assign 备选积压判定）；无配置 → NULL（缺省 10）
                let op_meta: Option<serde_json::Value> = node
                    .get("backupThreshold")
                    .and_then(|v| v.as_i64())
                    .map(|n| serde_json::json!({ "backupThreshold": n }));
                sqlx::query_scalar::<_, i64>(
                    r#"INSERT INTO isahl."zc_id_oper-approve"
                           (notice, code, created_by_id, _f_, _t_, meta)
                           VALUES ($1, $2, $3, '实现', '范例', $4) RETURNING id"#,
                )
                .bind(label)
                .bind(&graph_id)
                .bind(user_id)
                .bind(op_meta)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("node operation[{}]: {}", idx, e)))?
            }
            "action" => sqlx::query_scalar(
                r#"INSERT INTO isahl."zc_id_oper-action"
                       (notice, code, created_by_id, _f_, _t_)
                       VALUES ($1, $2, $3, '实现', '范例') RETURNING id"#,
            )
            .bind(label)
            .bind(&graph_id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| ApiError::Database(format!("node action[{}]: {}", idx, e)))?,
            "review" => {
                // 评审动作 → oper-check 子类（检查/评审语义；叶表 INSERT 规约）
                sqlx::query_scalar(
                    r#"INSERT INTO isahl."zc_id_oper-check"
                       (notice, code, created_by_id, _f_, _t_)
                       VALUES ($1, $2, $3, '实现', '范例') RETURNING id"#,
                )
                .bind(label)
                .bind(&graph_id)
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("node review[{}]: {}", idx, e)))?
            }
            _ => {
                // 自动节点（start/end/condition/cc/parallel/branch/gate/loop）
                // → oper-gate 子类（无 fk_approve 列；模板关联走 rr_event 桥）
                sqlx::query_scalar(
                    r#"INSERT INTO isahl."zc_id_oper-gate"
                       (notice, code, created_by_id, _f_, _t_)
                       VALUES ($1, $2, $3, '实现', '范例') RETURNING id"#,
                )
                .bind(label)
                .bind(&graph_id)
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("node gate[{}]: {}", idx, e)))?
            }
        };

        // 2026-09-02 end outcome 物化（operation.meta，2026-09-02 裁决）：
        // 运行时终局判定依据——complete 物化结论实例；rejected/cancelled 不物化。
        if node_type == "end" {
            let outcome = node
                .get("outcome")
                .and_then(|v: &Value| v.as_str())
                .unwrap_or("complete");
            sqlx::query(
                r#"UPDATE isahl.zc_id_operation
                   SET meta = jsonb_set(COALESCE(meta, '{}'::jsonb), '{end_outcome}',
                         to_jsonb($2::text), true)
                   WHERE id = $1 AND deleted_at IS NULL"#,
            )
            .bind(op_id)
            .bind(outcome)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::Database(format!("node end outcome[{}]: {}", idx, e)))?;
        }

        // 2.5 loop 公式链物化（refactor-flow-loop-formula-model）：
        // operation.meta 写循环运行时局部变量（vars/cursor）；自动创建
        // formula（Rhai）+ standard 范例/实例 + operation_rr_standard +
        // standard_r_formula。公式语法经 RhaiEngine validate 预检（fail-closed）。
        if node_type == "loop" {
            let formula = node
                .get("loopFormula")
                .and_then(|v: &Value| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let legacy_expr = node
                .get("loopExpr")
                .and_then(|v: &Value| v.as_str())
                .filter(|s| !s.trim().is_empty());
            // 新契约：新图必须有 loopFormula（fail-closed）；旧图（loopExpr 存量）
            // 兼容放行（运行时走旧 cond 路径），formula 链不建。
            if let Some(f) = formula {
                // Rhai 语法预检（compile-only）
                let rhai = runtime_engine::RhaiExpressionEngine::new();
                rhai.validate(f).map_err(|e| ApiError::Validation {
                    field: "nodes".into(),
                    message: format!("node[{}] '{}' (loop) 公式语法错误: {}", idx, label, e),
                })?;
                // 局部变量归一化：loopVars → {name: init}；保留键 cursor 禁定义；
                // maxIter 未定义时注入 10 兜底。
                let mut vars: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
                if let Some(varr) = node.get("loopVars").and_then(|v: &Value| v.as_array()) {
                    for item in varr {
                        if let Some(name) = item.get("name").and_then(|v: &Value| v.as_str()) {
                            if name.trim().is_empty() || name.trim() == "cursor" {
                                continue;
                            }
                            let init = item.get("init").cloned().unwrap_or(serde_json::json!(0));
                            vars.insert(name.trim().to_string(), init);
                        }
                    }
                }
                // D2（fix-approval-engine-gap-closure）：迭代上限采纳设计器
                // node.maxIter（大于等于 1）——节点级契约优先；缺省/非法时回退
                // loopVars 既有声明，仍无则 10 兜底（legacy 模板图行为不劣化）。
                if let Some(m) = node
                    .get("maxIter")
                    .and_then(|v: &Value| v.as_i64())
                    .filter(|m| *m >= 1)
                {
                    vars.insert("maxIter".to_string(), serde_json::json!(m));
                } else if !vars.contains_key("maxIter") {
                    vars.insert("maxIter".to_string(), serde_json::json!(10));
                }
                // operation.meta 写循环运行时状态
                sqlx::query(
                    r#"UPDATE isahl.zc_id_operation
                       SET meta = jsonb_set(COALESCE(meta, '{}'::jsonb), '{loop}',
                             $2::jsonb, true)
                       WHERE id = $1 AND deleted_at IS NULL"#,
                )
                .bind(op_id)
                // D2：cursors = 执行域键到迭代计数的对象（实体隔离）；legacy
                // flat cursor 不再写（advance.rs 读侧仅对变更前旧 op 行回退）。
                .bind(serde_json::json!({ "vars": vars, "cursors": {}, "formula": f }).to_string())
                .execute(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("node loop meta[{}]: {}", idx, e)))?;
                // formula 行 find-or-create（code = LOOP-FMLA-<graph_id>，实现·范例）
                let fmla_code = format!("LOOP-FMLA-{}", graph_id);
                let formula_id: i64 = match sqlx::query_scalar(
                    r#"SELECT id FROM isahl.zc_id_formula
                       WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
                )
                .bind(&fmla_code)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("lookup formula[{}]: {}", idx, e)))?
                .flatten()
                {
                    Some(id) => {
                        // 2026-09-02 复用行同步表达式：重发布改 loopFormula 时旧行
                        // 表达式必须随图更新（此前 LIMIT 1 复用遗留表达式——运行时求值
                        // 陈旧公式的生产级 bug）
                        sqlx::query(
                            r#"UPDATE isahl.zc_id_formula
                               SET expression = $2,
                                   context = '{"engine":"rhai"}'::jsonb,
                                   updated_at = NOW(), updated_by_id = $3
                               WHERE id = $1 AND deleted_at IS NULL"#,
                        )
                        .bind(id)
                        .bind(f)
                        .bind(user_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            ApiError::Database(format!("update formula[{}]: {}", idx, e))
                        })?;
                        id
                    }
                    None => sqlx::query_scalar(
                        r#"INSERT INTO isahl.zc_id_formula
                           (notice, code, expression, active, context, created_by_id)
                           VALUES ($1, $2, $3, true, '{"engine":"rhai"}'::jsonb, $4)
                           RETURNING id"#,
                    )
                    .bind(label)
                    .bind(&fmla_code)
                    .bind(f)
                    .bind(user_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|e| ApiError::Database(format!("create formula[{}]: {}", idx, e)))?,
                };
                // standard 范例 find-or-create + 实现·实例（tpl_id→范例）
                let std_code = format!("LOOP-STD-{}", graph_id);
                let std_tpl: i64 = match sqlx::query_scalar(
                    r#"SELECT id FROM isahl.zc_id_standard
                       WHERE code = $1 AND deleted_at IS NULL AND tpl_id IS NULL LIMIT 1"#,
                )
                .bind(&std_code)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("lookup std tpl[{}]: {}", idx, e)))?
                .flatten()
                {
                    Some(id) => id,
                    None => sqlx::query_scalar(
                        r#"INSERT INTO isahl.zc_id_standard
                           (notice, code, _f_, _t_, created_by_id)
                           VALUES ($1, $2, '实现', '范例', $3) RETURNING id"#,
                    )
                    .bind(label)
                    .bind(&std_code)
                    .bind(user_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|e| ApiError::Database(format!("create std tpl[{}]: {}", idx, e)))?,
                };
                // 实现·实例（用户裁决：创建 operation 时自动创建 standard 实现·实例）
                let std_inst: i64 = sqlx::query_scalar(
                    r#"INSERT INTO isahl.zc_id_standard
                       (notice, code, tpl_id, _f_, _t_, created_by_id)
                       VALUES ($1, $2, $3, '实现', '实例', $4) RETURNING id"#,
                )
                .bind(label)
                .bind(&std_code)
                .bind(std_tpl)
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("create std inst[{}]: {}", idx, e)))?;
                // 双桥：operation_rr_standard + standard_r_formula
                sqlx::query(
                    r#"INSERT INTO isahl.zc_id_operation_rr_standard (ref_left, ref_right, created_by_id)
                       VALUES ($1, $2, $3)"#,
                )
                .bind(op_id)
                .bind(std_inst)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("node rr_standard[{}]: {}", idx, e)))?;
                sqlx::query(
                    r#"INSERT INTO isahl.zc_id_standard_r_formula (ref_left, ref_right, created_by_id)
                       VALUES ($1, $2, $3)"#,
                )
                .bind(std_inst)
                .bind(formula_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("std r_formula[{}]: {}", idx, e)))?;
            } else if legacy_expr.is_none() {
                return Err(ApiError::Validation {
                    field: "nodes".into(),
                    message: format!(
                        "node[{}] '{}' (loop) 缺 loopFormula 配置——循环节点须配置 Rhai 公式",
                        idx, label
                    ),
                });
            }
        }

        let is_action_node = matches!(
            node_type,
            "approval" | "approve" | "oper-approve" | "review" | "action" | "vote"
        );
        let mode = node.get("mode").and_then(|v| v.as_str()).unwrap_or("");
        let is_terminal = is_task_start || is_event_start || is_end;
        if !is_terminal {
            let cat_code: String = if node_type == "vote" {
                // vote 语义经 timeline.quorum 表达（mode 不适用）；cate 码恒 'vote'
                "vote".to_string()
            } else if is_action_node {
                if !mode.is_empty() {
                    mode.to_string()
                } else {
                    match node_type {
                        "review" => "review".to_string(),
                        "action" => "action".to_string(),
                        _ => "approve".to_string(),
                    }
                }
            } else {
                node_type.to_string()
            };
            let cat_id: Option<i64> = sqlx::query_scalar(
                r#"SELECT id FROM isahl."zc_id_cate-proc_op"
                   WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
            )
            .bind(&cat_code)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| ApiError::Database(format!("lookup proc_op cat: {}", e)))?;
            let cat_id = match cat_id {
                Some(id) => id,
                None => sqlx::query_scalar(
                    r#"INSERT INTO isahl."zc_id_cate-proc_op" (notice, code, enable, created_by_id)
                       VALUES ($1, $1, true, $2) RETURNING id"#,
                )
                .bind(&cat_code)
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("create proc_op cat: {}", e)))?,
            };
            sqlx::query(
                r#"UPDATE isahl.zc_id_operation
                   SET "ck_cate-proc_op" = $1 WHERE id = $2"#,
            )
            .bind(cat_id)
            .bind(op_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::Database(format!("set proc_op cat[{}]: {}", idx, e)))?;
        }

        // 3. 语义实体桥（2026-08-29 裁决链路 process↔operation↔<语义实体>；2026-08-31
        //    event 驱动 start 补全）：end→rr_statement（→statement 范例·实现）/
        //    task-start→rr_task（→task 范例·实现）/ event-start→rr_event（→事件真叶表
        //    范例·实现）/ 其余→rr_event 模板桥（→even-approve 载体；实例挂模板、节点解析经此反查）
        match terminal_entity {
            Some(("statement", row_id)) => {
                sqlx::query(
                    r#"INSERT INTO isahl.zc_id_operation_rr_statement (ref_left, ref_right, created_by_id)
                       VALUES ($1, $2, $3)"#,
                )
                .bind(op_id)
                .bind(row_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("node rr_statement[{}]: {}", idx, e)))?;
            }
            Some(("task", row_id)) => {
                sqlx::query(
                    r#"INSERT INTO isahl.zc_id_operation_rr_task (ref_left, ref_right, created_by_id)
                       VALUES ($1, $2, $3)"#,
                )
                .bind(op_id)
                .bind(row_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("node rr_task[{}]: {}", idx, e)))?;
            }
            Some(("event", row_id)) => {
                // event 驱动 start：rr_event 直挂事件真叶表范例行（实现·范例）；
                // even-approve 载体行（timeline/SLA）不再承担事件语义
                sqlx::query(
                    r#"INSERT INTO isahl.zc_id_operation_rr_event (ref_left, ref_right, created_by_id)
                       VALUES ($1, $2, $3)"#,
                )
                .bind(op_id)
                .bind(row_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("node rr_event[{}]: {}", idx, e)))?;
            }
            _ => {
                sqlx::query(
                    r#"INSERT INTO isahl.zc_id_operation_rr_event (ref_left, ref_right, created_by_id)
                       VALUES ($1, $2, $3)"#,
                )
                .bind(op_id)
                .bind(template_id.expect("event path sets template"))
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| ApiError::Database(format!("node rr_event[{}]: {}", idx, e)))?;
            }
        }

        graph_to_db.insert(graph_id.clone(), op_id);

        // 4. process ↔ operation 关联（DAG 节点=操作；不含 next-ops，第三步补充）
        sqlx::query(
            r#"INSERT INTO isahl.zc_id_process_rr_operation
               (id, code, ref_left, ref_right, comments, created_by_id)
               VALUES (isahl.gen_next_uid(791), $1, $2, $3, $4, $5)"#,
        )
        // code = 图内节点编号（graph_id；§4.4 code 语义——不再承载节点类型）
        .bind(&graph_id)
        .bind(flow_id)
        .bind(op_id)
        .bind(label)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::Database(format!("rr_operation[{}]: {}", idx, e)))?;

        if is_action_node {
            let action = match node_type {
                "review" => "review",
                "action" => "action",
                _ => "approve",
            };
            let role = node.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let role_kind = node
                .get("roleKind")
                .and_then(|v| v.as_str())
                .unwrap_or("role");

            if !role.is_empty() {
                // roleKind：canonical 'role' | 'employee'；历史 'engineer' 别名读兼容（员工解析）
                let is_employee = role_kind == "employee" || role_kind == "engineer";
                // 动作桥分表：approve→rr_approve（带岗位类别 ck_cate-role 四类语义）/
                // review→rr_review / action→rr_post（无岗位类别，单岗位直配）
                let bridge_sql = match action {
                    "review" => {
                        r#"INSERT INTO isahl."zc_id_operation_rr_review" (ref_left, ref_right, created_by_id)
                                   VALUES ($1, $2, $3)"#
                    }
                    "action" => {
                        r#"INSERT INTO isahl."zc_id_operation_rr_post" (ref_left, ref_right, created_by_id)
                                  VALUES ($1, $2, $3)"#
                    }
                    _ => {
                        r#"INSERT INTO isahl."zc_id_operation_rr_approve" (ref_left, ref_right, created_by_id, "ck_cate-role")
                            VALUES ($1, $2, $3, $4)"#
                    }
                };
                if action == "approve" {
                    // 审批岗位类别（2026-09-03 语义接线）：字典 code → id；
                    // 种子缺失（未扩散库）时 None → 桥列 NULL（advance 视同直管，防断链）
                    let cate_err = |e: sqlx::Error| {
                        ApiError::Database(format!("node {action} cate lookup[{}]: {}", idx, e))
                    };
                    let direct_cate = approval_role_cate_id(&mut *tx, "ROLE-DIRECT")
                        .await
                        .map_err(cate_err)?;
                    let deputy_cate = approval_role_cate_id(&mut *tx, "ROLE-DEPUTY")
                        .await
                        .map_err(cate_err)?;
                    let escalate_cate = approval_role_cate_id(&mut *tx, "ROLE-ESCALATE")
                        .await
                        .map_err(cate_err)?;
                    let backup_cate = approval_role_cate_id(&mut *tx, "ROLE-BACKUP")
                        .await
                        .map_err(cate_err)?;
                    // 直管（role）解析；缺位兜底：直管岗位未设立/无活跃成员 → 代理岗位接管
                    let mut pending: Vec<(i64, Option<i64>)> =
                        resolve_approver_positions(&mut *tx, &role, is_employee)
                            .await
                            .map_err(|e| {
                                ApiError::Database(format!("resolve direct[{}]: {}", idx, e))
                            })?
                            .into_iter()
                            .map(|p| (p, direct_cate))
                            .collect();
                    if pending.is_empty() {
                        if let Some(dep) = node.get("roleDeputy").and_then(|v| v.as_str()) {
                            if !dep.trim().is_empty() {
                                let deputies =
                                    resolve_approver_positions(&mut *tx, dep.trim(), false)
                                        .await
                                        .map_err(|e| {
                                            ApiError::Database(format!(
                                                "resolve deputy[{}]: {}",
                                                idx, e
                                            ))
                                        })?;
                                pending.extend(deputies.into_iter().map(|p| (p, deputy_cate)));
                            }
                        }
                    }
                    // 升级岗位（SLA 超时接管目标）：解析落桥；岗位名由 timeline 段物化（读兼容 sla_timeout）
                    if let Some(esc) = node.get("roleEscalate").and_then(|v| v.as_str()) {
                        if !esc.trim().is_empty() {
                            let es = resolve_approver_positions(&mut *tx, esc.trim(), false)
                                .await
                                .map_err(|e| {
                                    ApiError::Database(format!("resolve escalate[{}]: {}", idx, e))
                                })?;
                            pending.extend(es.into_iter().map(|p| (p, escalate_cate)));
                        }
                    }
                    // 备选岗位（直管过载后备选；运行时由 advance 按积压阈值判定并入待办）
                    if let Some(bak) = node.get("roleBackup").and_then(|v| v.as_str()) {
                        if !bak.trim().is_empty() {
                            let bs = resolve_approver_positions(&mut *tx, bak.trim(), false)
                                .await
                                .map_err(|e| {
                                    ApiError::Database(format!("resolve backup[{}]: {}", idx, e))
                                })?;
                            pending.extend(bs.into_iter().map(|p| (p, backup_cate)));
                        }
                    }
                    for (pid, cate) in pending {
                        sqlx::query(bridge_sql)
                            .bind(op_id)
                            .bind(pid)
                            .bind(user_id)
                            .bind(cate)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| {
                                ApiError::Database(format!("node {action} bridge[{}]: {}", idx, e))
                            })?;
                    }
                } else {
                    // review/action：单岗位直配（无岗位类别）
                    let pos_ids = resolve_approver_positions(&mut *tx, &role, is_employee)
                        .await
                        .map_err(|e| {
                            ApiError::Database(format!("resolve role positions: {}", e))
                        })?;
                    for pid in pos_ids {
                        sqlx::query(bridge_sql)
                            .bind(op_id)
                            .bind(pid)
                            .bind(user_id)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| {
                                ApiError::Database(format!("node {action} bridge[{}]: {}", idx, e))
                            })?;
                    }
                }
            }

            // 2026-09-02 加权（岗位制多源）桥：voteSources 各源用户 → 岗位 → rr_approve
            // （岗位桥复用；源为 role/employee——不引入项目/群组岗位模型）
            if node_type == "vote" {
                if let Some(srcs) = node.get("voteSources").and_then(|v: &Value| v.as_array()) {
                    for item in srcs {
                        let kind = item.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                        let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        if id.is_empty() {
                            continue;
                        }
                        let users: Vec<i64> = if kind != "role" {
                            sqlx::query_scalar(
                                r#"SELECT u.id FROM isahl_auth.auth_users u
                                   WHERE u.is_active = TRUE AND (u.id IN (SELECT e.fk_user FROM isahl."zc_id_subj-employee" e WHERE e.deleted_at IS NULL AND (e.id::text = $1 OR e.notice = $1 OR e.code = $1)) OR u.username = $1 OR u.name = $1)
                                   LIMIT 50"#,
                            )
                            .bind(id)
                            .fetch_all(&mut *tx)
                            .await
                            .map_err(|e| ApiError::Database(format!("vote src user[{}]: {}", idx, e)))?
                        } else {
                            // 岗位类别成员经 common::ngac_org 收敛解析（指派 UA ∪ 岗位持有者）
                            common::ngac_org::resolve_member_user_ids(&mut *tx, id, 200).await
                        };
                        for uid in users {
                            let pos_ids: Vec<i64> = sqlx::query_scalar(
                                r#"SELECT pos.id FROM isahl."zc_id_subj-position" pos
                                   WHERE pos.fk_user = $1 AND pos.deleted_at IS NULL
                                   ORDER BY pos.id LIMIT 20"#,
                            )
                            .bind(uid)
                            .fetch_all(&mut *tx)
                            .await
                            .map_err(|e| {
                                ApiError::Database(format!("vote src pos[{}]: {}", idx, e))
                            })?;
                            for pid in pos_ids {
                                sqlx::query(
                                    r#"INSERT INTO isahl."zc_id_operation_rr_approve"
                                       (ref_left, ref_right, created_by_id)
                                       VALUES ($1, $2, $3)"#,
                                )
                                .bind(op_id)
                                .bind(pid)
                                .bind(user_id)
                                .execute(&mut *tx)
                                .await
                                .map_err(|e| {
                                    ApiError::Database(format!("vote src bridge[{}]: {}", idx, e))
                                })?;
                            }
                        }
                    }
                }
            }

            // event/task 为流程级上下文（fk_context 范畴绑定），节点不挂上下文；
            // operation 自身无状态（状态在流程 _r_status 桥与实例生命周期桥）
        }

        node_map.push(serde_json::json!({
            "index": idx,
            "id": op_id.to_string(),
            "graphId": graph_id,
            "type": node_type,
            "label": label,
            "drive": if is_task_start { "task" } else { "event" },
            "entityId": terminal_entity.map(|(_, id)| id).or(template_id),
            "entityKind": terminal_entity.map(|(k, _)| k).unwrap_or("event"),
        }));
    }

    // 版本快照载体守卫：全图无 even-approve 载体（task 驱动 start + end 的退化图）
    // → 图源快照无处安放，诚实报错回滚（版本恢复 D6 依赖快照）。
    if !snapshot_written {
        return Err(ApiError::Validation {
            field: "nodes".into(),
            message: "流程图无 even-approve 载体节点（全部节点为 task 驱动 start/end）——\
                      版本快照无落点，请至少配置一个人工/中间节点"
                .into(),
        });
    }

    // 4. 第二步：填充 next-ops（所有节点已创建，可以引用目标 ID）
    // 边一律写对象项 {"id":<数值>,…}（2026-09-02 A1：反向入边源定位依赖对象形态；
    // parse_next_op_entries 对两种形态均兼容，旧批裸数值图读取路径不变）。
    for (graph_id, op_id) in &graph_to_db {
        let targets = match edge_map.get(graph_id.as_str()) {
            Some(entries) => {
                let items: Vec<Value> = entries
                    .iter()
                    .filter_map(|t| {
                        graph_to_db.get(t.key.as_str()).map(|db_id| {
                            let mut obj = serde_json::Map::new();
                            obj.insert("id".into(), serde_json::json!(db_id));
                            if let Some(c) = &t.cond {
                                obj.insert("cond".into(), serde_json::json!(c));
                            }
                            if let Some(l) = &t.label {
                                obj.insert("label".into(), serde_json::json!(l));
                            }
                            Value::Object(obj)
                        })
                    })
                    .collect();
                if items.is_empty() {
                    Value::Null
                } else {
                    Value::Array(items)
                }
            }
            None => Value::Null,
        };

        if targets.is_null() {
            continue;
        }

        let targets_json = targets.to_string();
        sqlx::query(
            r#"UPDATE isahl.zc_id_process_rr_operation
               SET "next-ops" = $1::jsonb
               WHERE ref_left = $2 AND ref_right = $3 AND deleted_at IS NULL"#,
        )
        .bind(&targets_json)
        .bind(flow_id)
        .bind(op_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::Database(format!("update next-ops[{}]: {}", graph_id, e)))?;
    }

    // 5. 标记流程已发布：版本号推进 + 生命周期主状态桥（_r_status 体系）
    sqlx::query(
        r#"UPDATE isahl.zc_id_process
           SET tk_version = $3,
               updated_at = NOW(), updated_by_id = $1
           WHERE id = $2"#,
    )
    .bind(user_id)
    .bind(flow_id)
    .bind(new_version)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    update_flow_lifecycle_status_tx(&mut tx, flow_id, "published", "已发布", user_id).await?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;

    Ok(serde_json::json!({
        "id": flow_id.to_string(),
        "name": flow_name,
        "status": "published",
        "node_count": node_map.len(),
        "nodes": node_map,
    }))
}

/// 发布级图校验（fix-flow-designer-runtime-chain D8）：
/// nodes 提取 + 节点类型白名单 + edges 提取。发布（materialize_graph）与
/// `POST /approval-flows/validate` 共用——保存前预检与发布同判据。
/// 审批岗位成员解析（2026-09-03 语义接线）：
/// - employee/engineer：员工标识宽容匹配（员工 id/notice/code/username/name）→ 活跃用户挂岗；
/// - role/岗位：zc_id_subj-position 按 position id 或岗位名直配（过滤活跃用户）。
///   NGAC 用户属性域不再承担岗位解析——岗位实体权威 = zc_id_subj-position（identity-org 维护）。
async fn resolve_approver_positions(
    tx: &mut sqlx::PgConnection,
    val: &str,
    is_employee: bool,
) -> Result<Vec<i64>, sqlx::Error> {
    if is_employee {
        sqlx::query_scalar(
            r#"SELECT pos.id FROM isahl_auth.auth_users u
               JOIN isahl."zc_id_subj-position" pos
                 ON pos.fk_user = u.id AND pos.deleted_at IS NULL
               WHERE u.is_active = TRUE AND (u.id IN (SELECT e.fk_user FROM isahl."zc_id_subj-employee" e WHERE e.deleted_at IS NULL AND (e.id::text = $1 OR e.notice = $1 OR e.code = $1)) OR u.username = $1 OR u.name = $1)
               ORDER BY pos.id"#,
        )
        .bind(val)
        .fetch_all(&mut *tx)
        .await
    } else {
        sqlx::query_scalar(
            r#"SELECT pos.id
               FROM isahl."zc_id_subj-position" pos
               JOIN isahl_auth.auth_users u ON u.id = pos.fk_user AND u.is_active = TRUE
               WHERE pos.deleted_at IS NULL AND pos.fk_user IS NOT NULL
                 AND (pos.id::text = $1 OR pos.notice = $1)
               ORDER BY pos.id"#,
        )
        .bind(val)
        .fetch_all(&mut *tx)
        .await
    }
}

/// 审批岗位类别 id（zc_id_cate-approve_role code → id；字典未种子时 None → 桥列 NULL）
async fn approval_role_cate_id(
    tx: &mut sqlx::PgConnection,
    code: &str,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_cate-approve_role" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(code)
    .fetch_optional(&mut *tx)
    .await
}

/// backTo 上游可达校验（A5）：target 经出边（node.next 下标）可达 current 即合法。
fn reject_back_reachable(nodes: &[Value], from: usize, to: usize) -> bool {
    let mut stack = vec![from];
    let mut seen = vec![false; nodes.len()];
    while let Some(u) = stack.pop() {
        if u == to {
            return true;
        }
        if seen[u] || u >= nodes.len() {
            continue;
        }
        seen[u] = true;
        if let Some(next) = nodes[u].get("next").and_then(|v| v.as_array()) {
            for e in next {
                if let Some(t) = e.get("to").and_then(|v| v.as_i64()) {
                    if t >= 0 && (t as usize) < nodes.len() {
                        stack.push(t as usize);
                    }
                }
            }
        }
    }
    false
}

pub(crate) fn validate_graph(parsed: &Value) -> Result<(&[Value], Option<&[Value]>), ApiError> {
    let nodes = match parsed {
        Value::Object(map) => map.get("nodes").and_then(|v| v.as_array()),
        Value::Array(arr) => Some(arr),
        _ => None,
    }
    .ok_or_else(|| ApiError::Validation {
        field: "nodes".into(),
        message: "flow graph must contain a 'nodes' array".into(),
    })?;

    if nodes.is_empty() {
        return Err(ApiError::Validation {
            field: "nodes".into(),
            message: "flow graph has zero nodes".into(),
        });
    }

    // 节点类型白名单（fix-approval-action-chain P2-6，fail-closed）：
    // 设计器调色板（FlowDesigner utils.ts NODE_TYPES）+ 引擎词汇兼容集
    // （oper-approve/approve=历史词汇，gate/loop=自动节点词汇）。
    // subflow 2026-09-01 放行：运行时按 target 触发被引用流程实例
    // （advance.rs subflow 特判），发布时校验 target 存在且已发布。
    const ALLOWED_NODE_TYPES: &[&str] = &[
        "start",
        "end",
        "approve",
        "approval",
        "oper-approve",
        "review",
        "action",
        "vote",
        "condition",
        "cc",
        "parallel",
        "branch",
        "gate",
        "loop",
        "subflow",
    ];
    for (idx, node) in nodes.iter().enumerate() {
        let node_type = node
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if !ALLOWED_NODE_TYPES.contains(&node_type) {
            let label = node.get("label").and_then(|v| v.as_str()).unwrap_or("");
            return Err(ApiError::Validation {
                field: "nodes".into(),
                message: format!(
                    "node[{}] '{}' has unsupported type '{}'",
                    idx, label, node_type
                ),
            });
        }
    }

    // 终端节点语义配置校验（2026-08-29 裁决，fail-closed）：
    // end 须显式配置 statement 叶表；start 驱动 ∈ {event, task}，task 驱动
    // 须显式配置 task 叶表。叶表白名单 = context_meta 编译期快照。
    for (idx, node) in nodes.iter().enumerate() {
        let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let label = node.get("label").and_then(|v| v.as_str()).unwrap_or("");
        match node_type {
            "end" => {
                let leaf = node.get("statementLeaf").and_then(|v| v.as_str());
                match leaf {
                    Some(leaf) if crate::context_meta::is_statement_leaf(leaf) => {}
                    Some(leaf) => {
                        return Err(ApiError::Validation {
                            field: "nodes".into(),
                            message: format!(
                                "node[{}] '{}' (end) statementLeaf '{}' 不在 statement 真叶表白名单",
                                idx, label, leaf
                            ),
                        })
                    }
                    None => {
                        return Err(ApiError::Validation {
                            field: "nodes".into(),
                            message: format!(
                                "node[{}] '{}' (end) 缺 statementLeaf 配置——流程结束节点须在设计器显式配置结论承载叶表",
                                idx, label
                            ),
                        })
                    }
                }
                // 2026-09-02 end outcome 白名单（fail-closed）：complete/rejected/cancelled
                let outcome = node
                    .get("outcome")
                    .and_then(|v| v.as_str())
                    .unwrap_or("complete");
                if !matches!(outcome, "complete" | "rejected" | "cancelled") {
                    return Err(ApiError::Validation {
                        field: "nodes".into(),
                        message: format!(
                            "node[{}] '{}' (end) outcome '{}' 非法——须为 complete/rejected/cancelled",
                            idx, label, outcome
                        ),
                    });
                }
            }
            "start" => {
                let drive = node
                    .get("drive")
                    .and_then(|v| v.as_str())
                    .unwrap_or("event");
                match drive {
                    "event" => {
                        let leaf = node.get("eventLeaf").and_then(|v| v.as_str());
                        match leaf {
                            Some(leaf) if crate::context_meta::EVENT_LEAVES.iter().any(|i| i.table == leaf) => {}
                            Some(leaf) => {
                                return Err(ApiError::Validation {
                                    field: "nodes".into(),
                                    message: format!(
                                        "node[{}] '{}' (start, event 驱动) eventLeaf '{}' 不在 event 真叶表白名单",
                                        idx, label, leaf
                                    ),
                                })
                            }
                            None => {
                                return Err(ApiError::Validation {
                                    field: "nodes".into(),
                                    message: format!(
                                        "node[{}] '{}' (start, event 驱动) 缺 eventLeaf 配置——event 驱动开始节点须显式配置事件叶表",
                                        idx, label
                                    ),
                                })
                            }
                        }
                    }
                    "task" => {
                        let leaf = node.get("taskLeaf").and_then(|v| v.as_str());
                        match leaf {
                            Some(leaf) if crate::context_meta::is_task_leaf(leaf) => {}
                            Some(leaf) => {
                                return Err(ApiError::Validation {
                                    field: "nodes".into(),
                                    message: format!(
                                        "node[{}] '{}' (start, task 驱动) taskLeaf '{}' 不在 task 真叶表白名单",
                                        idx, label, leaf
                                    ),
                                })
                            }
                            None => {
                                return Err(ApiError::Validation {
                                    field: "nodes".into(),
                                    message: format!(
                                        "node[{}] '{}' (start, task 驱动) 缺 taskLeaf 配置",
                                        idx, label
                                    ),
                                })
                            }
                        }
                    }
                    other => {
                        return Err(ApiError::Validation {
                            field: "nodes".into(),
                            message: format!(
                                "node[{}] '{}' (start) drive '{}' 非法（event/task）",
                                idx, label, other
                            ),
                        })
                    }
                }
            }
            _ if matches!(
                node_type,
                "approval" | "approve" | "oper-approve" | "review" | "action"
            ) =>
            {
                if let Some(ra) = node.get("rejectAction").and_then(|v| v.as_str()) {
                    if !matches!(ra, "stop" | "back") {
                        return Err(ApiError::Validation {
                            field: "nodes".into(),
                            message: format!(
                                "node[{}] '{}' rejectAction '{}' 非法——须为 stop/back",
                                idx, label, ra
                            ),
                        });
                    }
                    if ra == "back" {
                        let bt = node.get("backTo").and_then(|v| v.as_i64());
                        let Some(bt) = bt else {
                            return Err(ApiError::Validation {
                                field: "nodes".into(),
                                message: format!(
                                    "node[{}] '{}' rejectAction=back 须配置 backTo 目标节点",
                                    idx, label
                                ),
                            });
                        };
                        let n = nodes.len() as i64;
                        if bt < 0 || bt >= n || bt == idx as i64 {
                            return Err(ApiError::Validation {
                                field: "nodes".into(),
                                message: format!(
                                    "node[{}] '{}' backTo {} 非法——须为图内上游节点且非自身",
                                    idx, label, bt
                                ),
                            });
                        }
                        if !reject_back_reachable(nodes, bt as usize, idx) {
                            return Err(ApiError::Validation {
                                field: "nodes".into(),
                                message: format!(
                                    "node[{}] '{}' backTo {} 非本节点上游（无 目标→本节点 边路径）",
                                    idx, label, bt
                                ),
                            });
                        }
                    }
                }
            }

            "condition" => {
                if let Some(r) = node.get("routing").and_then(|v| v.as_str()) {
                    if !matches!(r, "exclusive" | "inclusive") {
                        return Err(ApiError::Validation {
                            field: "nodes".into(),
                            message: format!(
                                "node[{}] '{}' (condition) routing '{}' 非法——须为 exclusive/inclusive",
                                idx, label, r
                            ),
                        });
                    }
                }
            }
            "vote" => {
                let has_q = node.get("quorum").and_then(|v| v.as_i64()).is_some();
                let has_pct = node.get("quorumPct").and_then(|v| v.as_i64()).is_some();
                if has_q && has_pct {
                    return Err(ApiError::Validation {
                        field: "nodes".into(),
                        message: format!(
                            "node[{}] '{}' (vote) quorum 与 quorumPct 互斥——只能配置其一",
                            idx, label
                        ),
                    });
                }
                if let Some(pct) = node.get("quorumPct").and_then(|v| v.as_i64()) {
                    if !(1..=100).contains(&pct) {
                        return Err(ApiError::Validation {
                            field: "nodes".into(),
                            message: format!(
                                "node[{}] '{}' (vote) quorumPct '{}' 非法——须 1..=100",
                                idx, label, pct
                            ),
                        });
                    }
                }
                if let Some(q) = node.get("quorum").and_then(|v| v.as_i64()) {
                    if q < 0 {
                        return Err(ApiError::Validation {
                            field: "nodes".into(),
                            message: format!(
                                "node[{}] '{}' (vote) quorum '{}' 非法——须 ≥ 0（0 = 全部投票人）",
                                idx, label, q
                            ),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // 解析边 (edges) —— 兼容两种设计图格式：
    // 1) 显式 `edges: [{source, target}]`（既有契约，行为不变）
    // 2) 设计器 serializeFlow 格式（无 edges 数组，节点携带 `next: [{to: 下标}]` 索引边）——
    //    仅当显式 edges 缺失/为空时按节点下标推导，与 Framework FlowDesigner/flow-persistence 契约一致。
    let edges = match parsed {
        Value::Object(map) => map.get("edges").and_then(|v| v.as_array()),
        _ => None,
    };

    Ok((nodes, edges.map(|v| v.as_slice())))
}

/// 审批流程停用 Handler（F-3：publish 的对称端点）
///
/// `POST /approval-flows/{id}/unpublish` — 流程回到 draft（生命周期主状态桥）；
/// 已发布模板与历史实例保留（仅停用新发起）。
pub async fn unpublish_flow(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let flow_id = path.into_inner();
    let user_id = context::require_auth(&req)?;

    // 守卫：仅已发布（主状态桥 published）流程可停用
    let published: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
             SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
             JOIN isahl."zc_id_stus-process" s ON s.id = ls.ref_right
             WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL AND s.code = 'published'
           )"#,
    )
    .bind(flow_id)
    .fetch_one(&**pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    if !published {
        return Err(ApiError::NotFound(format!(
            "ApprovalFlow {} not found or not published",
            flow_id
        )));
    }

    sqlx::query(
        r#"UPDATE isahl.zc_id_process
           SET tk_version = COALESCE(tk_version, 0) + 1,
               updated_at = NOW(), updated_by_id = $1
           WHERE id = $2 AND deleted_at IS NULL"#,
    )
    .bind(user_id)
    .bind(flow_id)
    .execute(&**pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    update_flow_lifecycle_status(pool.get_ref(), flow_id, "draft", "草稿", user_id).await?;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "id": flow_id.to_string(),
            "status": "draft",
        }))),
    )
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route("/approval-flows/{id}/publish", web::post().to(publish_flow));
    cfg.route(
        "/approval-flows/{id}/unpublish",
        web::post().to(unpublish_flow),
    );
}
