//! 流程推进 — 审批完成后自动推进到下一节点
//!
//! 通过 zc_id_process_rr_operation.next-ops（DAG 边）查找下一节点，
//! 根据节点类型创建对应的 operation。
//!
//! 本模块还提供 `init_project_flow` — 项目创建后从流程模板实例化首节点。

use crate::node_meta::{resolve_node_assign, SignMode};
use common::error::AliothError as ApiError;
use common::event_bus::{DomainEvent, DomainEventBus};
use common::SYSTEM_USER_ID;
use sqlx::PgPool;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

// ============================================================
// P1-5 条件选边 + P0-1/P1-4 指派语义辅助
// ============================================================

/// next-ops 项解析：兼容裸数值（旧 DAG）与对象项 {"id":N,"cond":…}（publish 新批）
fn parse_next_op_entries(value: &serde_json::Value) -> Vec<(i64, Option<String>)> {
    match value {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|item| match item {
                serde_json::Value::Number(n) => n.as_i64().map(|id| (id, None)),
                serde_json::Value::Object(obj) => {
                    obj.get("id").and_then(|v| v.as_i64()).map(|id| {
                        (
                            id,
                            obj.get("cond").and_then(|c| c.as_str()).map(str::to_string),
                        )
                    })
                }
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

/// 实例 ↔ 业务实体桥解析（fix-flow-designer-runtime-chain D3）：
/// 实体绑定随实例创建逐实例写入（create_approval_instances 从 actx.entity 复制）：
/// task 域 → zc_id_operation_rr_task；event/approve 域 → zc_id_operation_rr_event。
/// 返回（叶表名, 实体行 id）；无绑定（旧路径实例）→ None。
///
/// rr_event 双用途区分：节点接线行 ref_right 为 even-approve 基表行（publish 物化
/// 节点模板），实体绑定行 ref_right 为叶表行业务实体——以 tableoid 非基表判定。
async fn resolve_entity_ref(
    pool: &PgPool,
    instance_id: i64,
) -> Result<Option<(String, i64)>, ApiError> {
    // task 域（rr_task 无节点接线双用途，直接判定）
    let task_entity: Option<i64> = sqlx::query_scalar(
        r#"SELECT ref_right FROM isahl.zc_id_operation_rr_task
           WHERE ref_left = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    if let Some(entity_id) = task_entity {
        let leaf: Option<String> = sqlx::query_scalar(
            r#"SELECT tableoid::regclass::text FROM isahl.zc_id_task WHERE id = $1"#,
        )
        .bind(entity_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?
        .flatten();
        return Ok(leaf.map(|t| (t.trim_matches('"').to_string(), entity_id)));
    }
    // event/approve 域（排除节点接线行：ref_right 在 even-approve 基表 = 节点模板）
    let event_entity: Option<(String, i64)> = sqlx::query_as(
        r#"SELECT e.tableoid::regclass::text AS leaf, e.id
           FROM isahl.zc_id_event e
           JOIN isahl.zc_id_operation_rr_event rr
             ON rr.ref_right = e.id AND rr.deleted_at IS NULL
           WHERE rr.ref_left = $1 AND e.deleted_at IS NULL
             AND e.tableoid <> 'zc_id_even-approve'::regclass
           ORDER BY rr.id LIMIT 1"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .map(|(leaf, id): (String, i64)| (leaf.trim_matches('"').to_string(), id));
    Ok(event_entity)
}

/// 表达式上下文（fix-flow-designer-runtime-chain D3）：
/// - `entityId` 保留键（= 当前节点事件 id，现状语义）；
/// - 实体行扁平列（to_jsonb，物理列名 = RuleBuilder 字段名）；
///
/// 无实体绑定 / 行不存在 → 仅 entityId（不阻断推进；cond 引用缺失标识符
/// UndefinedIdent → fail-closed，既有容错语义）。
/// 叶表名来自桥解析/范畴校验通道（非用户直接输入），表名拼接安全。
async fn build_expr_ctx(
    pool: &PgPool,
    entity_ref: Option<&(String, i64)>,
    entity_id: i64,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    map.insert("entityId".to_string(), serde_json::json!(entity_id));
    let Some((leaf, row_id)) = entity_ref else {
        return map;
    };
    // 静态 SQL 分发（sqlx 0.9 &'static str 约束 + 注入审计，禁 format! 动态表名）：
    // 叶表经桥/范畴通道解析，必然命中 context_meta 三域白名单；未命中 warn 降级。
    let Some(sql) = crate::context_meta::entity_row_sql(leaf) else {
        common::telemetry::warn!(
            "build_expr_ctx: entity leaf '{}' 不在三域白名单 — ctx 仅保留键",
            leaf
        );
        return map;
    };
    let row: Option<serde_json::Value> = sqlx::query_scalar(sql)
        .bind(row_id)
        .fetch_optional(pool)
        .await
        .unwrap_or_else(|e| {
            common::telemetry::warn!(
                "build_expr_ctx: entity row load failed ({} #{})：{}",
                leaf,
                row_id,
                e
            );
            None
        });
    match row.and_then(|v| v.as_object().cloned()) {
        Some(obj) => {
            for (k, v) in &obj {
                map.insert(k.clone(), v.clone());
            }
            // 模型设计规则（2026-09-01）：外键列（fk/lk/qk/ck/sk/ref/dk/ak_*）
            // 不直接入选条件/计算参数——引用值经 `_refs` 模式访问：按行内非空
            // 引用列逐一解析目标行 {id,label,color}（CONTEXT_REFS 静态分发，
            // 与 context-fields 同源生成；行缺失/解析失败降级为缺该项——引用
            // 缺失成员时条件求值 UndefinedIdent fail-closed，既有容错语义）。
            let mut refs = serde_json::Map::new();
            for (l, col, ref_sql) in crate::context_meta::CONTEXT_REFS.iter() {
                if *l != leaf {
                    continue;
                }
                let Some(val) = obj.get(*col) else {
                    continue;
                };
                let Some(ref_id) = val.as_i64() else {
                    continue;
                };
                let resolved: Option<serde_json::Value> = sqlx::query_scalar(*ref_sql)
                    .bind(ref_id)
                    .fetch_optional(pool)
                    .await
                    .unwrap_or_else(|e| {
                        common::telemetry::warn!(
                            "build_expr_ctx: _refs resolve failed ({}#{} {})：{}",
                            leaf,
                            ref_id,
                            col,
                            e
                        );
                        None
                    });
                if let Some(r) = resolved {
                    refs.insert((*col).to_string(), r);
                }
            }
            map.insert("_refs".to_string(), serde_json::Value::Object(refs));
        }
        None => {
            common::telemetry::warn!(
                "build_expr_ctx: entity row not found ({} #{}) — ctx 仅保留键",
                leaf,
                row_id
            );
        }
    }
    map
}

/// 流程条件求值（统一引擎：runtime-engine ExpressionEvaluator；
/// fail-closed——未定义标识符（strict 模式）/类型不匹配 → Err，调用方阻断；
/// 顶层非 bool 视同 false（对齐原 expr.rs 语义））
fn eval_flow_condition(
    expr: &str,
    ctx: &serde_json::Map<String, serde_json::Value>,
) -> Result<bool, String> {
    use std::collections::HashMap;
    let ast = runtime_contract::expression::parse_constraint_expression(expr)
        .map_err(|e| format!("parse: {e}"))?;
    let vars: HashMap<String, serde_json::Value> =
        ctx.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let v = runtime_engine::ExpressionEvaluator::eval_expr_to_json_strict(&ast, &vars)
        .map_err(|e| format!("eval: {e}"))?;
    match v {
        serde_json::Value::Bool(b) => Ok(b),
        _ => Ok(false),
    }
}

/// 选边（2026-09-02 routing 模式，fix-flow-gateway-semantics A2）：
/// inclusive（缺省/存量）：带 cond 且求值 true 的边全部入选；无 cond 边为兜底
/// （仅无 cond 命中时入选）；cond 求值错误 → fail-closed（跳过该边 + warn）。
/// exclusive：按出边顺序首个 cond=true 边首中即止；无 cond 命中时按首个
/// 无 cond 兜底边；无兜底 → 空（调用方不扇出）。
fn select_targets(
    entries: &[(i64, Option<String>)],
    ctx: &serde_json::Map<String, serde_json::Value>,
    exclusive: bool,
) -> Vec<i64> {
    if exclusive {
        let mut first_default: Option<i64> = None;
        for (id, cond) in entries {
            match cond {
                Some(expr) => {
                    match eval_flow_condition(expr, ctx) {
                        Ok(true) => return vec![*id],
                        Ok(false) => {}
                        Err(e) => {
                            common::telemetry::warn!(
                            "edge cond '{}' eval failed ({}) (exclusive): edge to node {} skipped",
                            expr, e, id
                        );
                        }
                    }
                }
                None => {
                    if first_default.is_none() {
                        first_default = Some(*id);
                    }
                }
            }
        }
        return first_default.map(|id| vec![id]).unwrap_or_default();
    }
    let mut matched = Vec::new();
    let mut defaults = Vec::new();
    for (id, cond) in entries {
        match cond {
            Some(expr) => {
                match eval_flow_condition(expr, ctx) {
                    Ok(true) => matched.push(*id),
                    Ok(false) => {}
                    Err(e) => {
                        common::telemetry::warn!(
                        "edge cond '{}' eval failed ({}): edge to node {} skipped (fail-closed)",
                        expr, e, id
                    );
                    }
                }
            }
            None => defaults.push(*id),
        }
    }
    if matched.is_empty() {
        defaults
    } else {
        matched
    }
}

/// condition 节点路由模式（fix-flow-gateway-semantics A2）：载体 timeline
/// routing ∈ {exclusive, inclusive}；缺省/缺失 = inclusive（存量兼容）。
async fn node_routing(pool: &PgPool, op_id: i64) -> Result<String, ApiError> {
    let raw: Option<String> = sqlx::query_scalar(
        r#"SELECT ea.timeline->>'routing' FROM isahl."zc_id_even-approve" ea
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
           WHERE oe.ref_left = $1 AND ea.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(op_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten();
    Ok(raw.unwrap_or_else(|| "inclusive".to_string()))
}

/// 推进上下文（P0-1）：发起人/触发人/链指针/实体上下文随自动节点链透传，
/// 下游人工实例的 fk_subject=发起人、fk_previous=源审批实例不被自动节点跳断。
#[derive(Clone)]
struct AdvanceCtx<'a> {
    /// 流程发起人（写 fk_subject）
    initiator: i64,
    /// 触发推进的操作人（写 created_by_id）
    trigger: i64,
    /// 源审批实例（写 fk_previous；项目流初始化等无源场景为 None）
    prev_instance: Option<i64>,
    /// 实体上下文（实例 comments 可读摘要，随链复制）
    comments: Option<&'a str>,
    /// 业务实体绑定（fix-flow-designer-runtime-chain D3）：
    /// （叶表名, 行 id）——随实例创建逐实例写 rr 桥，全程透传
    entity: Option<(String, i64)>,
    /// 领域事件总线（cc 抄送事件；无注入时降级不发布）
    bus: Option<&'a Arc<dyn DomainEventBus>>,
}

/// 同节点其余在途（非终态）实例 id（排除自身）——会签门控与兄弟取消共用
async fn pending_sibling_ids(
    pool: &PgPool,
    node_event_id: i64,
    exclude_instance: i64,
) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar(
        // tpl_id IS NOT NULL 排除操作定义行（fix-avic-approval-node-model 后
        // operation 与 instance 同表 zc_id_oper-approve；操作行无生命周期状态，
        // 不排除会作为恒 pending 兄弟阻塞 and_sign 门禁）
        r#"SELECT oa.id FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe
             ON oe.ref_left = oa.id AND oe.ref_right = $1 AND oe.deleted_at IS NULL
           WHERE oa.id <> $2 AND oa.deleted_at IS NULL
             AND oa.tpl_id IS NOT NULL
             AND NOT EXISTS (
                 SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                 JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
                 WHERE ls.ref_left = oa.id AND ls.deleted_at IS NULL
                   AND s.code IN ('approved','rejected','withdrawn','cancelled','abstained')
             )
           ORDER BY oa.id"#,
    )
    .bind(node_event_id)
    .bind(exclude_instance)
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))
}

/// 取消兄弟实例（写 cancelled 桥；状态行由 update_lifecycle_status find-or-create）
pub(crate) async fn cancel_pending_siblings(
    pool: &PgPool,
    node_event_id: i64,
    exclude_instance: i64,
    user_id: i64,
) -> Result<usize, ApiError> {
    let siblings = pending_sibling_ids(pool, node_event_id, exclude_instance).await?;
    let n = siblings.len();
    for sid in siblings {
        crate::handlers::approve_reject::update_lifecycle_status(
            pool,
            sid,
            "cancelled",
            "已取消",
            user_id,
        )
        .await?;
    }
    Ok(n)
}

/// 节点已 approved 实例数（含当前刚通过的实例——approve_inner 先写桥再推进）
async fn approved_count(pool: &PgPool, node_event_id: i64) -> Result<i64, ApiError> {
    sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe
             ON oe.ref_left = oa.id AND oe.ref_right = $1 AND oe.deleted_at IS NULL
           WHERE oa.deleted_at IS NULL
             AND EXISTS (
                 SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                 JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
                 WHERE ls.ref_left = oa.id AND ls.deleted_at IS NULL AND s.code = 'approved'
             )"#,
    )
    .bind(node_event_id)
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))
}

/// 审批完成后推进流程到下一节点
pub async fn advance_flow(
    pool: &PgPool,
    instance_id: i64,
    user_id: i64,
    bus: Option<&Arc<dyn DomainEventBus>>,
) -> Result<(), ApiError> {
    // 1. 获取当前实例所在节点（实例经 operation_rr_event 桥 → even-approve 模板）
    //    与所属流程（even-approve.fk_process 已移除——流程归属经
    //    「节点 oper 行 ∈ process_rr_operation」桥链反查；无桥即模板未绑定流程，
    //    Option 解码避免 “unexpected null” decode 失败）。
    let (current_event_id, flow_id) = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
        r#"SELECT oe.ref_right, (SELECT rro.ref_left
                  FROM isahl.zc_id_operation_rr_event oe2
                  JOIN isahl.zc_id_process_rr_operation rro
                    ON rro.ref_right = oe2.ref_left AND rro.deleted_at IS NULL
                   WHERE oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL
                   ORDER BY oe2.created_at LIMIT 1)
           FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe
             ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           JOIN isahl."zc_id_even-approve" ea
             ON ea.id = oe.ref_right AND ea.deleted_at IS NULL
           WHERE oa.id = $1 AND oa.deleted_at IS NULL
             AND EXISTS (
                 SELECT 1 FROM isahl.zc_id_operation_rr_event oe3
                 JOIN isahl.zc_id_process_rr_operation rro3
                   ON rro3.ref_right = oe3.ref_left AND rro3.deleted_at IS NULL
                 WHERE oe3.ref_right = oe.ref_right AND oe3.deleted_at IS NULL
             )
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .unwrap_or((None, None));

    // 2. 无流程模板（桥链无流程归属）或无当前节点 → 无可推进，安全返回
    let (Some(flow_id), Some(current_event_id)) = (flow_id, current_event_id) else {
        return Ok(());
    };

    // 源实例上下文：fk_subject（流程发起人，向下游透传）+ comments（实体上下文）
    let source_ctx: Option<(Option<i64>, Option<String>)> = sqlx::query_as(
        r#"SELECT fk_subject, comments FROM isahl."zc_id_oper-approve"
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    let (initiator, source_comments) = match source_ctx {
        Some((subj, c)) => (subj.unwrap_or(user_id), c),
        None => (user_id, None),
    };

    // 2.5 节点完成门控（P1-4 会签/或签/依次语义，与 P0-1 岗位解析互锁）：
    // 节点 meta 从模型表解析（fix-avic-approval-node-model：操作→岗位桥 +
    // 操作分类签署模式；comments 不再承载结构）。实例挂节点事件模板，
    // 经模板桥反查 operation 节点主体（refactor-flow-node-operation-model）。
    let current_op_id: i64 = sqlx::query_scalar(
        r#"SELECT oe.ref_left FROM isahl.zc_id_operation_rr_event oe
           JOIN isahl.zc_id_operation o ON o.id = oe.ref_left AND o.tpl_id IS NULL
           WHERE oe.ref_right = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(current_event_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten()
    .unwrap_or(current_event_id);
    let node_assign = resolve_node_assign(pool, current_op_id).await?;
    // 实体桥解析（D3）：当前实例自携绑定，一次解析全程透传
    let entity = resolve_entity_ref(pool, instance_id).await?;
    let actx = AdvanceCtx {
        initiator,
        trigger: user_id,
        prev_instance: Some(instance_id),
        comments: source_comments.as_deref(),
        entity: entity.clone(),
        bus,
    };
    match node_assign.sign_mode {
        SignMode::AndSign => {
            // 会签：兄弟仍在途 → 等齐，不推进
            let pending = pending_sibling_ids(pool, current_event_id, instance_id).await?;
            if !pending.is_empty() {
                common::telemetry::info!(
                    "and_sign gate: node {} waits for {} pending sibling(s)",
                    current_event_id,
                    pending.len()
                );
                return Ok(());
            }
        }
        SignMode::OrSign => {
            // 或签：首个通过定案——取消其余在途兄弟后推进
            cancel_pending_siblings(pool, current_event_id, instance_id, user_id).await?;
        }
        SignMode::Vote => {
            // 投票 quorum 门控（2026-09-02 A3 增强）：阈值 = quorumPct 百分位
            // （ceil(N*pct/100)）优先，quorum 正数次之，缺省 = 投票人总数（全员）。
            // 任一终态动作（approve/reject/abstain 经 vote_terminal_advance）触发本臂：
            // 达标 → 取消余票推进；全员已行动未达标 → 自动 rejected 终局（消除滞留）。
            let raw: (Option<i64>, Option<i64>) = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
                r#"SELECT NULLIF(ea.timeline->>'quorum','')::bigint,
                          NULLIF(ea.timeline->>'quorumPct','')::bigint
                     FROM isahl."zc_id_even-approve" ea
                     JOIN isahl.zc_id_operation_rr_event oe
                       ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
                    WHERE oe.ref_left = $1 AND ea.deleted_at IS NULL
                    ORDER BY oe.created_at LIMIT 1"#,
            )
            .bind(current_op_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?
            .unwrap_or((None, None));
            // 权重表（resolvedWeights，岗位制多源；空 = 等权 1）
            let weights_raw: Option<serde_json::Value> = sqlx::query_scalar(
                r#"SELECT ea.timeline->'resolvedWeights' FROM isahl."zc_id_even-approve" ea
                   JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
                   WHERE oe.ref_left = $1 AND ea.deleted_at IS NULL
                   ORDER BY oe.created_at LIMIT 1"#,
            )
            .bind(current_op_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?
            .flatten();
            let mut weights: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
            if let Some(serde_json::Value::Array(items)) = &weights_raw {
                for it in items {
                    if let (Some(uid), Some(w)) = (
                        it.get("uid").and_then(|v| v.as_i64()),
                        it.get("weight").and_then(|v| v.as_i64()),
                    ) {
                        weights.insert(uid, w.max(1));
                    }
                }
            }
            let voters_total = node_assign.assignees.len() as i64;
            let total_weight: i64 = if weights.is_empty() {
                voters_total
            } else {
                weights.values().sum()
            };
            let threshold: i64 = match raw.1.filter(|p| *p > 0) {
                Some(pct) => (total_weight * pct + 99) / 100,
                None => raw.0.filter(|q| *q > 0).unwrap_or(total_weight),
            };
            // approved 用户 → 权重和（无权重表/未解析用户按 1；空权重表 = 票数语义）
            let approved_users: Vec<Option<i64>> = sqlx::query_scalar(
                r#"SELECT oa.fk_operator FROM isahl."zc_id_oper-approve" oa
                   JOIN isahl.zc_id_operation_rr_event oe
                     ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
                   WHERE oe.ref_right = $1 AND oa.deleted_at IS NULL
                     AND EXISTS (
                       SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                       JOIN isahl."zc_id_stus-approve" st ON st.id = ls.ref_right
                       WHERE ls.ref_left = oa.id AND ls.deleted_at IS NULL AND st.code = 'approved'
                     )"#,
            )
            .bind(current_event_id)
            .fetch_all(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?;
            let approved: i64 = approved_users
                .iter()
                .map(|u| u.and_then(|id| weights.get(&id)).copied().unwrap_or(1))
                .sum();
            if approved >= threshold {
                cancel_pending_siblings(pool, current_event_id, instance_id, user_id).await?;
            } else {
                let pending = pending_sibling_ids(pool, current_event_id, instance_id).await?;
                if !pending.is_empty() {
                    common::telemetry::info!(
                        "vote gate: node {} approved {}/{} quorum — waiting",
                        current_event_id,
                        approved,
                        threshold
                    );
                    return Ok(());
                }
                // 全员已行动未达标：自动 rejected 终局（本实例若已是 rejected 不重复发布）
                common::telemetry::warn!(
                    "vote node {} (flow {}) all voters acted, quorum {}/{} unmet — auto-rejected",
                    current_event_id,
                    flow_id,
                    approved,
                    threshold
                );
                if let Some(bus) = actx.bus {
                    let already_rejected: bool = sqlx::query_scalar(
                        r#"SELECT EXISTS (
                            SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                            JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
                            WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL AND s.code = 'rejected'
                        )"#,
                    )
                    .bind(instance_id)
                    .fetch_one(pool)
                    .await
                    .map_err(|e| ApiError::Database(e.to_string()))?;
                    if !already_rejected {
                        crate::handlers::approve_reject::publish_approval_completed(
                            bus,
                            pool,
                            instance_id,
                            "rejected",
                            Some("quorum 未达自动终局"),
                        )
                        .await;
                    }
                }
                return Ok(());
            }
        }
        SignMode::Sequential => {
            // 依次签署：审批人集合 >1 且还有下一位 → 建下一位实例，不推进
            let assignees = node_assign.assignees;
            if assignees.len() > 1 {
                let done = approved_count(pool, current_event_id).await? as usize;
                if done < assignees.len() {
                    create_approval_instances(
                        pool,
                        current_event_id,
                        "",
                        &actx,
                        Some(&assignees[done..=done.min(assignees.len() - 1)]),
                    )
                    .await?;
                    return Ok(());
                }
            }
        }
    }
    // 读取当前节点的 next-ops（DAG 出边）：ref_right = 当前节点 operation id
    let next_ops: Option<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT "next-ops" FROM isahl.zc_id_process_rr_operation
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(current_op_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten();

    let entries = match next_ops {
        Some(v) => parse_next_op_entries(&v),
        _ => return Ok(()),
    };
    if entries.is_empty() {
        return Ok(());
    }

    // P1-5 条件选边：带 cond 命中者优先，无条件边兜底；exclusive 按源节点 routing
    let ctx = build_expr_ctx(pool, actx.entity.as_ref(), current_op_id).await;
    let routing = node_routing(pool, current_op_id).await?;
    let targets = select_targets(&entries, &ctx, routing == "exclusive");
    if targets.is_empty() {
        common::telemetry::warn!(
            "advance_flow: no outgoing edge selected at node {} (flow {}) — flow stalls here",
            current_event_id,
            flow_id
        );
        return Ok(());
    }

    // 3. 为每个目标节点创建 operation
    for target_id in targets {
        let node_info = sqlx::query_as::<_, (String, String)>(
            r#"SELECT CASE
                        WHEN c.code IS NOT NULL
                          AND c.code NOT IN ('and_sign', 'or_sign', 'sequential')
                          THEN c.code
                        WHEN replace(o.tableoid::regclass::text, '"', '') = 'zc_id_oper-approve' THEN 'approve'
                        WHEN replace(o.tableoid::regclass::text, '"', '') = 'zc_id_oper-action' THEN 'action'
                        WHEN replace(o.tableoid::regclass::text, '"', '') = 'zc_id_oper-check' THEN 'review'
                        WHEN EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_statement rs
                                     WHERE rs.ref_left = o.id AND rs.deleted_at IS NULL)
                          THEN 'end'
                        ELSE 'gate'
                      END,
                      COALESCE(rro.comments, o.notice)
               FROM isahl.zc_id_process_rr_operation rro
               JOIN isahl.zc_id_operation o ON o.id = rro.ref_right AND o.deleted_at IS NULL
               LEFT JOIN isahl."zc_id_cate-proc_op" c
                 ON c.id = o."ck_cate-proc_op" AND c.deleted_at IS NULL
               WHERE rro.ref_left = $1 AND rro.ref_right = $2 AND rro.deleted_at IS NULL"#,
        )
        .bind(flow_id)
        .bind(target_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;

        let (node_type, node_label) = match node_info {
            Some(n) => n,
            None => continue,
        };

        match node_type.as_str() {
            // 词汇兼容（fix-approval-flow-chain-breaks F1）：
            // - "approve"          — seed/历史直插数据（zc_id_even-approve.t_color_ 或 rro.code）
            // - "oper-approve"     — 遗留词汇
            // - "approval"         — 设计器发布物化值（publish.rs 原样存前端 node.type）
            // 三者均为人工审批节点，统一创建审批实例，不得落入自动节点分支。
            // review/action 亦为人工节点（评审/执行岗位），统一创建实例
            "oper-approve" | "approve" | "approval" | "review" | "action" | "vote" => {
                create_approval_instances(pool, target_id, &node_label, &actx, None).await?;
            }
            _ => {
                // 自动节点（gate/branch/loop/parallel/cc 等）统一处理
                advance_auto_node(
                    pool,
                    flow_id,
                    target_id,
                    &node_type,
                    &node_label,
                    &actx,
                    None,
                )
                .await?;
            }
        }
    }

    Ok(())
}

/// vote 节点终态动作后的终局判定（2026-09-02 A3）：实例节点 cate=vote 时
/// 调 advance_flow（其 Vote 臂处理 quorum 达标推进或全员终态未达自动 rejected 终局）；
/// 非 vote 节点不推进（reject/abstain 保持现状终态语义）。
pub(crate) async fn vote_terminal_advance(
    pool: &PgPool,
    instance_id: i64,
    user_id: i64,
    bus: Option<&Arc<dyn DomainEventBus>>,
) -> Result<(), ApiError> {
    let cate: Option<String> = sqlx::query_scalar(
        r#"SELECT c.code FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_operation_rr_event oe2
             ON oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL
           JOIN isahl.zc_id_operation o ON o.id = oe2.ref_left AND o.tpl_id IS NULL
           LEFT JOIN isahl."zc_id_cate-proc_op" c
             ON c.id = o."ck_cate-proc_op" AND c.deleted_at IS NULL
           WHERE oa.id = $1 AND oa.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten();
    if cate.as_deref() == Some("vote") {
        advance_flow(pool, instance_id, user_id, bus).await?;
    }
    Ok(())
}

/// 发布版本图快照读取（fix-approval-engine-gap-closure D4）：publish 把整图
/// 快照挂当批首个 even-approve 载体 `timeline.graph`（版本化——每发布一批旧
/// 载体退役、新载体持最新快照；重设计未发布时推进期仍锚定最后一次发布的图）。
async fn published_graph(
    pool: &PgPool,
    flow_id: i64,
) -> Result<Option<serde_json::Value>, ApiError> {
    sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT ea.timeline->'graph' FROM isahl."zc_id_even-approve" ea
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_left = $1 AND rro.code = ea.code AND rro.deleted_at IS NULL
            AND rro.created_at = ea.created_at
          WHERE ea.deleted_at IS NULL AND ea.timeline ? 'graph'
          ORDER BY ea.id LIMIT 1"#,
    )
    .bind(flow_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))
}

/// 图文档读取（D4 统一入口）：发布版本快照优先；快照缺失（pre-快照 legacy 图）
/// 回退 process.meta——兼容语义，行为不劣化。
async fn flow_graph(pool: &PgPool, flow_id: i64) -> Result<Option<serde_json::Value>, ApiError> {
    if let Some(graph) = published_graph(pool, flow_id).await? {
        return Ok(Some(graph));
    }
    sqlx::query_scalar::<_, Option<serde_json::Value>>(
        r#"SELECT meta FROM isahl.zc_id_process WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))
    .map(|row| row.flatten())
}

/// 驳回路由（2026-09-02 A5，add-flow-reject-routing）：
/// 实例节点 rejectAction=back 时——取消全流程在途（下游重置）→ 在打回目标
/// operation 重新创建审批实例（同发起人/实体上下文，fk_previous 指向驳回实例）。
/// stop/缺省（或 vote 表决节点）返回 false 不动作；目标不可解析 → warn 降级 false。
/// 返回是否发生打回路由。
pub(crate) async fn route_reject(
    pool: &PgPool,
    instance_id: i64,
    user_id: i64,
) -> Result<bool, ApiError> {
    // 1) 实例 → 载体/流程/当前范例 op（advance_flow 同源查询）
    let (carrier, flow_id) = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
        r#"SELECT oe.ref_right, (SELECT rro.ref_left
                  FROM isahl.zc_id_operation_rr_event oe2
                  JOIN isahl.zc_id_process_rr_operation rro
                    ON rro.ref_right = oe2.ref_left AND rro.deleted_at IS NULL
                   WHERE oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL
                   ORDER BY oe2.created_at LIMIT 1)
           FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe
             ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           WHERE oa.id = $1 AND oa.deleted_at IS NULL
             AND EXISTS (
                 SELECT 1 FROM isahl.zc_id_operation_rr_event oe3
                 JOIN isahl.zc_id_process_rr_operation rro3
                   ON rro3.ref_right = oe3.ref_left AND rro3.deleted_at IS NULL
                 WHERE oe3.ref_right = oe.ref_right AND oe3.deleted_at IS NULL
             )
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .unwrap_or((None, None));
    let (Some(flow_id), Some(carrier)) = (flow_id, carrier) else {
        return Ok(false);
    };
    let current_op: Option<i64> = sqlx::query_scalar(
        r#"SELECT oe.ref_left FROM isahl.zc_id_operation_rr_event oe
           JOIN isahl.zc_id_operation o ON o.id = oe.ref_left AND o.tpl_id IS NULL
           WHERE oe.ref_right = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(carrier)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten();
    let Some(current_op) = current_op else {
        return Ok(false);
    };

    // 2) cate（vote 表决不路由）+ rejectAction/backTo
    let cate: Option<String> = sqlx::query_scalar(
        r#"SELECT c.code FROM isahl.zc_id_operation o
           LEFT JOIN isahl."zc_id_cate-proc_op" c
             ON c.id = o."ck_cate-proc_op" AND c.deleted_at IS NULL
           WHERE o.id = $1 AND o.deleted_at IS NULL"#,
    )
    .bind(current_op)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten();
    if cate.as_deref() == Some("vote") {
        return Ok(false);
    }
    let cfg: (Option<String>, Option<i64>) = sqlx::query_as::<_, (Option<String>, Option<i64>)>(
        r#"SELECT ea.timeline->>'rejectAction',
                  NULLIF(ea.timeline->>'backTo','')::bigint
             FROM isahl."zc_id_even-approve" ea
             JOIN isahl.zc_id_operation_rr_event oe
               ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
            WHERE oe.ref_left = $1 AND ea.deleted_at IS NULL
            ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(current_op)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .unwrap_or((None, None));
    let (action, back_idx) = cfg;
    if action.as_deref() != Some("back") {
        return Ok(false);
    }
    let Some(back_idx) = back_idx.filter(|i| *i >= 0) else {
        common::telemetry::warn!(
            "route_reject: node {} backTo 缺失/非法——按 stop 处理",
            current_op
        );
        return Ok(false);
    };

    // 3) nodes[back_idx].id → 目标 op（D4：发布版本图快照优先，缺失回退 process.meta）
    let meta: Option<serde_json::Value> = flow_graph(pool, flow_id).await?;
    let target_id: Option<String> = meta.as_ref().and_then(|m| {
        m.get("nodes").and_then(|v| v.as_array()).and_then(|arr| {
            arr.get(back_idx as usize)
                .and_then(|n| n.get("id").and_then(|i| i.as_str()).map(str::to_string))
        })
    });
    let Some(target_id) = target_id else {
        common::telemetry::warn!(
            "route_reject: backTo index {} 越界/无 id——按 stop 处理",
            back_idx
        );
        return Ok(false);
    };
    let target_op: Option<(i64, String)> = sqlx::query_as::<_, (i64, String)>(
        r#"SELECT rro.ref_right, COALESCE(rro.comments, o.notice)
             FROM isahl.zc_id_process_rr_operation rro
             JOIN isahl.zc_id_operation o ON o.id = rro.ref_right AND o.deleted_at IS NULL
            WHERE rro.ref_left = $1 AND rro.code = $2 AND rro.deleted_at IS NULL
            LIMIT 1"#,
    )
    .bind(flow_id)
    .bind(&target_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;

    // 4) 取消全流程在途 + 目标重开
    let Some((target_op, target_label)) = target_op else {
        common::telemetry::warn!(
            "route_reject: 目标 op '{}' 不可解析——按 stop 处理",
            target_id
        );
        return Ok(false);
    };
    let pending_ids: Vec<i64> = sqlx::query_scalar(
        r#"SELECT oa.id FROM isahl."zc_id_oper-approve" oa
           JOIN isahl.zc_id_operation_rr_event oe
             ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
           WHERE oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL
             AND oe.ref_right IN (
               SELECT oe2.ref_right FROM isahl."zc_id_operation_rr_event" oe2
               JOIN isahl."zc_id_process_rr_operation" rro2
                 ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL
               WHERE rro2.ref_left = $1 AND oe2.deleted_at IS NULL
             )
             AND NOT EXISTS (
               SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
               JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
               WHERE ls.ref_left = oa.id AND ls.deleted_at IS NULL
                 AND s.code IN ('approved','rejected','withdrawn','cancelled','abstained')
             )"#,
    )
    .bind(flow_id)
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    for pid in &pending_ids {
        crate::handlers::approve_reject::update_lifecycle_status(
            pool,
            *pid,
            "cancelled",
            "驳回打回：在途重置",
            user_id,
        )
        .await?;
    }
    // 发起人/实体上下文沿用驳回链
    let source: Option<(Option<i64>, Option<String>)> =
        sqlx::query_as::<_, (Option<i64>, Option<String>)>(
            r#"SELECT fk_subject, comments FROM isahl."zc_id_oper-approve"
           WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(instance_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
    let (initiator, comments) = source.unwrap_or((None, None));
    let entity = resolve_entity_ref(pool, instance_id).await?;
    let actx = AdvanceCtx {
        initiator: initiator.unwrap_or(user_id),
        trigger: user_id,
        prev_instance: Some(instance_id),
        comments: comments.as_deref(),
        entity: entity.clone(),
        bus: None,
    };
    create_approval_instances(pool, target_op, &target_label, &actx, None).await?;
    common::telemetry::info!(
        "route_reject: instance {} rejected -> rework at op {} (flow {})",
        instance_id,
        target_op,
        flow_id
    );
    Ok(true)
}

/// 创建下一审批实例（P0-1/P1-4：岗位解析 + 会签/或签/依次语义）
///
/// - `actx`：推进上下文（initiator→fk_subject、trigger→created_by_id、
///   prev_instance→fk_previous、comments→实体上下文透传）
/// - `only_assignees`：sequential 门控指定「只建这几位」时传入；None = 按节点 mode 自行解析
///
/// 返回新建实例 id 列表。岗位解析为空集 → fk_operator=NULL（admin 全可见兜底）
/// + warn，**不回退申请人/上一审批人**（那是 P0-1 的 bug 本体）。
async fn create_approval_instances(
    pool: &PgPool,
    event_id: i64,
    label: &str,
    actx: &AdvanceCtx<'_>,
    only_assignees: Option<&[i64]>,
) -> Result<Vec<i64>, ApiError> {
    let node_assign = resolve_node_assign(pool, event_id).await?;

    // 目标审批人集合：显式指定（sequential 链）优先；否则按模型解析
    let targets: Vec<Option<i64>> = match only_assignees {
        Some(list) => list.iter().map(|u| Some(*u)).collect(),
        None => {
            let assignees = node_assign.assignees;
            match node_assign.sign_mode {
                // 会签/或签/投票：全员同时建实例
                SignMode::AndSign | SignMode::OrSign | SignMode::Vote if !assignees.is_empty() => {
                    assignees.into_iter().map(Some).collect()
                }
                // 依次/无岗位：首人（或空集 → NULL 兜底）
                _ => vec![assignees.first().copied()],
            }
        }
    };
    if targets.is_empty() || targets.iter().all(|t| t.is_none()) {
        common::telemetry::warn!(
            "create_approval_instances: node {} ('{}') resolved zero assignees — fk_operator NULL (admin-visible fallback)",
            event_id,
            label
        );
    }
    if targets.is_empty() {
        // only_assignees 传入空片的防御（正常路径不会发生）
        return Ok(vec![]);
    }

    // 实例 label 兜底：调用方未给（sequential 链）时读节点 operation.notice
    let label_owned;
    let label = if label.is_empty() {
        label_owned = sqlx::query_scalar::<_, Option<String>>(
            r#"SELECT notice FROM isahl.zc_id_operation WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?
        .flatten()
        .unwrap_or_else(|| "审批".to_string());
        &label_owned
    } else {
        label
    };

    let mut created = Vec::with_capacity(targets.len());
    for operator in targets {
        // 节点事件模板（实例 fk_approve → even-approve 事件模板，经模板桥反查；
        // 范例/实例分层：tpl_id 同表关联——实例 tpl_id → 本表操作模板行
        // （refactor-flow-node-operation-model：tpl_id 不跨表））
        let template_id: i64 = sqlx::query_scalar(
            r#"SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
               WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
               ORDER BY oe.created_at LIMIT 1"#,
        )
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?
        .flatten()
        .unwrap_or(event_id);
        let instance_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_oper-approve"
               (id, notice, code, fk_subject, fk_operator, fk_previous, comments, created_by_id, tpl_id, _f_, _t_)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8, '实现', '实例')
               RETURNING id"#,
        )
        .bind(label)
        .bind(label)
        .bind(actx.initiator)
        .bind(operator)
        .bind(actx.prev_instance)
        .bind(actx.comments)
        .bind(actx.trigger)
        .bind(event_id)
        .fetch_one(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;

        sqlx::query(
            r#"INSERT INTO isahl.zc_id_operation_rr_event
               (id, ref_left, ref_right, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3)"#,
        )
        .bind(instance_id)
        .bind(template_id)
        .bind(actx.trigger)
        .execute(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;

        // 实体桥逐实例复制（fix-flow-designer-runtime-chain D3）：
        // task 域 → rr_task；event/approve 域 → rr_event——后续推进选边
        // 经 resolve_entity_ref 自携解析，无需链遍历。
        if let Some((leaf, entity_row)) = &actx.entity {
            if leaf.starts_with("zc_id_task-") {
                sqlx::query(
                    r#"INSERT INTO isahl.zc_id_operation_rr_task
                       (id, ref_left, ref_right, created_by_id)
                       VALUES (isahl.gen_next_zuid(), $1, $2, $3)"#,
                )
                .bind(instance_id)
                .bind(entity_row)
                .bind(actx.trigger)
                .execute(pool)
                .await
                .map_err(|e| ApiError::Database(e.to_string()))?;
            } else {
                sqlx::query(
                    r#"INSERT INTO isahl.zc_id_operation_rr_event
                       (id, ref_left, ref_right, created_by_id)
                       VALUES (isahl.gen_next_zuid(), $1, $2, $3)"#,
                )
                .bind(instance_id)
                .bind(entity_row)
                .bind(actx.trigger)
                .execute(pool)
                .await
                .map_err(|e| ApiError::Database(e.to_string()))?;
            }
        }
        // D5 委托自动转派：按**本实例审批人**查有效委托并改派
        // （修复：此前以上一审批人身份查委托，改派对象错误）
        if let Some(op) = operator {
            apply_delegation(pool, instance_id, op).await?;
        }
        created.push(instance_id);
    }

    Ok(created)
}

/// 创建门禁操作（自动节点），返回 gate_id
async fn create_gate_operation(
    pool: &PgPool,
    event_id: i64,
    _flow_id: i64,
    label: &str,
    user_id: i64,
) -> Result<i64, ApiError> {
    let gate_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-gate"
           (id, notice, code, fk_subject, created_by_id, tpl_id, _f_, _t_)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, '实现', '实例')
           RETURNING id"#,
    )
    .bind(label)
    .bind(label)
    .bind(user_id)
    .bind(user_id)
    .bind(event_id)
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;

    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event
           (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3)"#,
    )
    .bind(gate_id)
    .bind(event_id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;

    // D5 委托自动转派：门禁操作同样按当前操作人查有效委托并改派
    apply_delegation_inner(pool, DelegationTarget::Gate, gate_id, user_id).await?;

    Ok(gate_id)
}

/// 扇出推进（2026-09-02：parallel 全边扇出与 loop 回边单步共用）：
/// 对每条目标边解析节点类型——human（approve/review/action/vote）创建审批
/// 实例，自动节点经 BoxFuture 递归 advance_auto_node（打破静态递归环
/// E0733；嵌套深度有限——发布校验已拒自引用/图深度受限）。
async fn advance_fan_out(
    pool: &PgPool,
    flow_id: i64,
    entries: &[(i64, Option<String>)],
    actx: &AdvanceCtx<'_>,
    mut created: Option<&mut Vec<i64>>,
) -> Result<(), ApiError> {
    for (target_id, _cond) in entries {
        let node_info = sqlx::query_as::<_, (String, String)>(
            r#"SELECT CASE
                        WHEN c.code IS NOT NULL
                          AND c.code NOT IN ('and_sign', 'or_sign', 'sequential')
                          THEN c.code
                        WHEN replace(o.tableoid::regclass::text, '"', '') = 'zc_id_oper-approve' THEN 'approve'
                        WHEN replace(o.tableoid::regclass::text, '"', '') = 'zc_id_oper-action' THEN 'action'
                        WHEN replace(o.tableoid::regclass::text, '"', '') = 'zc_id_oper-check' THEN 'review'
                        WHEN EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_statement rs
                                     WHERE rs.ref_left = o.id AND rs.deleted_at IS NULL)
                          THEN 'end'
                        ELSE 'gate'
                      END,
                      COALESCE(rro.comments, o.notice)
                 FROM isahl.zc_id_process_rr_operation rro
                 JOIN isahl.zc_id_operation o ON o.id = rro.ref_right AND o.deleted_at IS NULL
                 LEFT JOIN isahl."zc_id_cate-proc_op" c
                   ON c.id = o."ck_cate-proc_op" AND c.deleted_at IS NULL
                 WHERE rro.ref_left = $1 AND rro.ref_right = $2 AND rro.deleted_at IS NULL"#,
        )
        .bind(flow_id)
        .bind(*target_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
        if let Some((target_type, target_label)) = node_info {
            match target_type.as_str() {
                // review/action/vote 亦为人工节点（评审/执行/投票岗位），统一建实例
                "oper-approve" | "approve" | "approval" | "review" | "action" | "vote" => {
                    create_approval_instances(pool, *target_id, &target_label, actx, None).await?;
                }
                _ => {
                    // BoxFuture 打破 advance_auto_node 自递归（自动链经 box 隔离静态递归检测）
                    let fut = Box::pin(async {
                        advance_auto_node(
                            pool,
                            flow_id,
                            *target_id,
                            &target_type,
                            &target_label,
                            actx,
                            created.as_deref_mut(),
                        )
                        .await
                    });
                    fut.await?;
                }
            }
        }
    }
    Ok(())
}

/// 汇聚规则 any 判定（2026-09-01 能力补齐）：branch 节点 joinRule='any' 时，
/// 任一前驱分支源节点（入边源 operation）已有终态（approved）实例即放行。
/// 入边 = `next-ops @> [{"id": <branch_op_id>}]` jsonb containment（publish
/// 新批对象形态；旧批裸数值项不匹配 → 走缺省 all，行为不变）。
async fn any_branch_reached(
    pool: &PgPool,
    flow_id: i64,
    branch_op_id: i64,
) -> Result<bool, ApiError> {
    let reached: Option<bool> = sqlx::query_scalar(
        r#"SELECT EXISTS (
            SELECT 1 FROM isahl.zc_id_process_rr_operation src
            WHERE src.ref_left = $1 AND src.deleted_at IS NULL
              AND src."next-ops" @> $2::jsonb
              AND EXISTS (
                  SELECT 1 FROM isahl.zc_id_operation_rr_event oe
                  JOIN isahl."zc_id_even-approve" ea
                    ON ea.id = oe.ref_right AND ea.deleted_at IS NULL
                  JOIN isahl."zc_id_oper-approve" oa
                    ON oa.id = oe.ref_left AND oa.deleted_at IS NULL
                  WHERE oe.deleted_at IS NULL
                    AND oa.tpl_id IS NOT NULL
                    AND EXISTS (
                        SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                        JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
                        WHERE ls.ref_left = oa.id AND ls.deleted_at IS NULL
                          AND s.code = 'approved'
                    )
                    AND EXISTS (
                        SELECT 1 FROM isahl.zc_id_operation_rr_event oe2
                        JOIN isahl.zc_id_process_rr_operation rro2
                          ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL
                        WHERE oe2.ref_right = ea.id AND oe2.deleted_at IS NULL
                          AND rro2.ref_left = $1
                    )
              )
        )"#,
    )
    .bind(flow_id)
    .bind(serde_json::json!([{ "id": branch_op_id.to_string() }]).to_string())
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    Ok(reached.unwrap_or(false))
}

/// 子流程触发（2026-09-01 能力补齐）：以同实体上下文发起被引用流程实例。
/// BoxFuture 包装打破 advance_auto_node → initiate_flow → advance_auto_node
/// 静态递归环（async fn 直接递归 E0733）；子流程嵌套深度有限（发布校验
/// 已拒自引用），同步 await 不构成运行时无限递归。
fn trigger_subflow<'a>(
    pool: &'a PgPool,
    target_flow_id: i64,
    trigger: i64,
    entity_table: &'a str,
    entity_id: i64,
    execution_anchor: Option<i64>,
    bus: Option<&'a Arc<dyn DomainEventBus>>,
) -> Pin<Box<dyn Future<Output = Result<(), ApiError>> + Send + 'a>> {
    Box::pin(async move {
        initiate_flow(
            pool,
            target_flow_id,
            trigger,
            entity_table,
            entity_id,
            execution_anchor,
            bus,
        )
        .await?;
        Ok(())
    })
}

/// 自动节点推进 — 条件检查 + 创建操作 + 继续遍历
/// 被 advance_flow 和 process_node_advancement 共用
async fn advance_auto_node(
    pool: &PgPool,
    flow_id: i64,
    template_node_id: i64,
    node_type: &str,
    _node_label: &str,
    actx: &AdvanceCtx<'_>,
    created: Option<&mut Vec<i64>>,
) -> Result<(), ApiError> {
    // cc 抄送（fix-flow-designer-runtime-chain D5）：发布 ApprovalCc 事件
    // （recipients 物化于节点行 timeline），随后按自动节点语义继续推进。
    if node_type == "cc" {
        if let Some(bus) = actx.bus {
            let recipients_raw: Option<serde_json::Value> = sqlx::query_scalar(
                r#"SELECT ea.timeline->'recipients' FROM isahl."zc_id_even-approve" ea
                   JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
                   WHERE oe.ref_left = $1 AND ea.deleted_at IS NULL
                   ORDER BY oe.created_at LIMIT 1"#,
            )
            .bind(template_node_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .flatten();
            // A6：结构化 recipients（数组）解析出 resolvedUsers；文本兼容原样透传
            let mut resolved_users: Vec<i64> = Vec::new();
            if let Some(serde_json::Value::Array(refs)) = &recipients_raw {
                for item in refs {
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
                        .fetch_all(pool)
                        .await
                        .unwrap_or_default()
                    } else {
                        sqlx::query_scalar(
                            r#"SELECT u.id FROM isahl_auth.auth_users u
                               JOIN isahl_auth.ngac_user_rr_attribute rel
                                 ON rel.fk_user = u.id AND rel.deleted_at IS NULL
                               JOIN isahl_auth.ngac_user_attribute ua
                                 ON ua.id = rel.fk_user_attribute AND ua.deleted_at IS NULL
                               WHERE ua.o_name = $1 AND u.is_active = TRUE
                               LIMIT 200"#,
                        )
                        .bind(id)
                        .fetch_all(pool)
                        .await
                        .unwrap_or_default()
                    };
                    resolved_users.extend(users);
                }
            }
            let payload = serde_json::json!({
                "flow_id": flow_id.to_string(),
                "node_id": template_node_id.to_string(),
                "label": _node_label,
                "recipients": recipients_raw,
                "resolvedUsers": resolved_users,
                "entity_table": actx.entity.as_ref().map(|(t, _)| t),
                "entity_id": actx.entity.as_ref().map(|(_, id)| id),
            });
            if let Ok(event) =
                DomainEvent::new("ApprovalCc", "commitment", template_node_id, payload)
            {
                let _ = bus.publish("ApprovalCc", &event).await;
            }
        }
    }
    // P1-5 条件节点 expr 求值（统一引擎；归属恢复）：expr 由 Flow-Design meta
    // 管理——经 rr_operation.code（图内编号）反查 meta.nodes[].expr；求值
    // false/错误 → fail-closed 阻断推进（恢复注释承诺的语义）。
    if node_type == "condition" {
        let graph_id: Option<String> = sqlx::query_scalar(
            r#"SELECT rro.code FROM isahl.zc_id_process_rr_operation rro
               WHERE rro.ref_left = $1 AND rro.ref_right = $2 AND rro.deleted_at IS NULL
               ORDER BY rro.id LIMIT 1"#,
        )
        .bind(flow_id)
        .bind(template_node_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?
        .flatten();
        if let Some(gid) = graph_id {
            // D4：版本快照优先（重设计未发布时运行时仍锚定最后一次发布图）
            let meta: Option<serde_json::Value> = flow_graph(pool, flow_id).await?;
            let node_expr: Option<String> = meta.as_ref().and_then(|m| {
                m.get("nodes").and_then(|v| v.as_array()).and_then(|arr| {
                    arr.iter()
                        .find(|n| n.get("id").and_then(|i| i.as_str()) == Some(gid.as_str()))
                        .and_then(|n| n.get("expr").and_then(|e| e.as_str()).map(str::to_string))
                })
            });
            if let Some(e) = node_expr.filter(|s| !s.trim().is_empty()) {
                let ctx = build_expr_ctx(pool, actx.entity.as_ref(), template_node_id).await;
                match eval_flow_condition(&e, &ctx) {
                    Ok(true) => {}
                    Ok(false) => {
                        common::telemetry::info!(
                            "condition node {} (flow {}) expr '{}' false — flow blocked (fail-closed)",
                            template_node_id, flow_id, e
                        );
                        return Ok(());
                    }
                    Err(err) => {
                        common::telemetry::warn!(
                            "condition node {} (flow {}) expr '{}' eval failed ({}) — flow blocked (fail-closed)",
                            template_node_id, flow_id, e, err
                        );
                        // P2 结构化：表达式求值错误显式报出（前端可识别并回灌 chat-ai 自愈）
                        return Err(ApiError::Validation {
                            field: "flow".into(),
                            message: format!(
                                "condition 节点 {template_node_id}（flow {flow_id}）表达式求值失败——fail-closed 阻断。表达式: {e}；错误: {err}"
                            ),
                        });
                    }
                }
            }
        }
    }

    // 2026-09-01 汇聚规则：branch joinRule='any' 时改判任一前驱分支已终态；
    // 2026-09-02 A1：all/缺省改为局部汇聚（等待本节点入边源分支终态），
    // legacy 裸数值图回退 flow 级等待。
    let mut join_rule_any = false;
    if node_type == "branch" {
        let join_rule: Option<String> = sqlx::query_scalar(
            r#"SELECT ea.timeline->>'joinRule' FROM isahl."zc_id_even-approve" ea
               JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
               WHERE oe.ref_left = $1 AND ea.deleted_at IS NULL
               ORDER BY oe.created_at LIMIT 1"#,
        )
        .bind(template_node_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?
        .flatten()
        .unwrap_or_default();
        join_rule_any = join_rule.as_deref() == Some("any");
    }
    if node_type == "branch" && join_rule_any {
        let reached = any_branch_reached(pool, flow_id, template_node_id).await?;
        if !reached {
            common::telemetry::info!(
                "branch any-join blocked: no predecessor branch terminal at node {} (flow {})",
                template_node_id,
                flow_id
            );
            return Ok(());
        }
        // any 放行：取消其余分支在途实例（对齐 or_sign 取消语义），
        // 避免后续分支完成再次推进同一汇聚点造成重复扇出。
        let pending_ids: Vec<i64> = sqlx::query_scalar(
            r#"SELECT oa.id FROM isahl."zc_id_oper-approve" oa
               JOIN isahl.zc_id_operation_rr_event oe
                 ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
               JOIN isahl."zc_id_even-approve" ea ON ea.id = oe.ref_right
               WHERE EXISTS (
                     SELECT 1 FROM isahl.zc_id_operation_rr_event oe2
                     JOIN isahl.zc_id_process_rr_operation rro2
                       ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL
                     WHERE oe2.ref_right = ea.id AND oe2.deleted_at IS NULL
                       AND rro2.ref_left = $1
                 )
                 AND oa.deleted_at IS NULL
                 AND oa.tpl_id IS NOT NULL
                 AND NOT EXISTS (
                     SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                     JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
                     WHERE ls.ref_left = oa.id
                       AND ls.deleted_at IS NULL
                       AND s.code IN ('approved','rejected','withdrawn','cancelled','abstained')
                 )"#,
        )
        .bind(flow_id)
        .fetch_all(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
        for pid in pending_ids {
            crate::handlers::approve_reject::update_lifecycle_status(
                pool,
                pid,
                "cancelled",
                "汇聚任一完成，其余分支取消",
                actx.trigger,
            )
            .await?;
        }
    } else if node_type == "branch" {
        // 局部 all 汇聚（2026-09-02 fix-flow-gateway-semantics A1）：
        // 入边源 = 本流程中 next-ops 反向引用本 branch 的 operation；等待全部
        // 入边源的在途实例终态。legacy 裸数值图（定位不到入边源）回退 flow 级。
        let source_count: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(DISTINCT rro2.ref_right) FROM isahl."zc_id_process_rr_operation" rro2
               WHERE rro2.ref_left = $1 AND rro2.deleted_at IS NULL
                 AND rro2."next-ops" @> $2::jsonb"#,
        )
        .bind(flow_id)
        .bind(serde_json::json!([{ "id": template_node_id }]).to_string())
        .fetch_one(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
        let pending: Option<bool> = if source_count > 0 {
            sqlx::query_scalar(
                r#"SELECT EXISTS (
                    SELECT 1 FROM isahl."zc_id_oper-approve" oa
                    JOIN isahl."zc_id_operation_rr_event" oe
                      ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
                    WHERE oa.deleted_at IS NULL AND oa.tpl_id IS NOT NULL
                      AND oe.ref_right IN (
                        SELECT oe2.ref_right FROM isahl."zc_id_operation_rr_event" oe2
                        WHERE oe2.deleted_at IS NULL AND oe2.ref_left IN (
                          SELECT rro2.ref_right FROM isahl."zc_id_process_rr_operation" rro2
                          WHERE rro2.ref_left = $1 AND rro2.deleted_at IS NULL
                            AND rro2."next-ops" @> $2::jsonb
                        )
                      )
                      AND NOT EXISTS (
                        SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                        JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
                        WHERE ls.ref_left = oa.id AND ls.deleted_at IS NULL
                          AND s.code IN ('approved','rejected','withdrawn','cancelled','abstained')
                      )
                )"#,
            )
            .bind(flow_id)
            .bind(serde_json::json!([{ "id": template_node_id }]).to_string())
            .fetch_one(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?
        } else {
            // legacy 回退：flow 级等待（与归档 change 前一致）
            sqlx::query_scalar(
                r#"SELECT EXISTS (
                    SELECT 1 FROM isahl."zc_id_oper-approve" oa
                    JOIN isahl."zc_id_operation_rr_event" oe
                      ON oe.ref_left = oa.id AND oe.deleted_at IS NULL
                    JOIN isahl."zc_id_even-approve" ea
                      ON ea.id = oe.ref_right 
                    WHERE EXISTS (
                        SELECT 1 FROM isahl."zc_id_operation_rr_event" oe2
                        JOIN isahl."zc_id_process_rr_operation" rro2
                          ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL
                        WHERE oe2.ref_right = ea.id AND oe2.deleted_at IS NULL
                          AND rro2.ref_left = $1
                    )
                      AND oa.deleted_at IS NULL
                      AND oa.tpl_id IS NOT NULL
                      AND NOT EXISTS (
                          SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                          JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
                          WHERE ls.ref_left = oa.id
                            AND ls.deleted_at IS NULL
                            AND s.code IN ('approved','rejected','withdrawn','cancelled','abstained')
                      )
                )"#,
            )
            .bind(flow_id)
            .fetch_one(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?
        };
        if pending.unwrap_or(true) {
            common::telemetry::info!(
                "branch all-join blocked: pending branch approvals in flow {} at node {}",
                flow_id,
                template_node_id
            );
            return Ok(());
        }
    }

    let gate_id =
        create_gate_operation(pool, template_node_id, flow_id, _node_label, actx.trigger).await?;
    // 终局陈述物化（2026-08-29 裁决）：到达 end = 流程终局，按 end 范例同表
    // 物化结论 statement 实例并挂 gate 实例。
    if node_type == "end" {
        materialize_end_statement(pool, template_node_id, gate_id, _node_label, actx.trigger)
            .await?;
        // A4：本流程若是 wait 子流程 → 终局回调续推父流程
        resume_parent_flow(pool, flow_id, actx.entity.as_ref(), actx.bus).await?;
    }

    // 2026-09-01 能力补齐：subflow 引用触发——创建 gate 操作后，以同实体
    // 上下文发起被引用流程实例（initiate_flow）。target 流程不可用或实体
    // 未绑定时降级为仅 gate 推进（warn），不阻断父流程。
    let mut subflow_waiting = false;
    if node_type == "subflow" {
        let target: Option<String> = sqlx::query_scalar(
            r#"SELECT ea.timeline->>'target' FROM isahl."zc_id_even-approve" ea
               JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
               WHERE oe.ref_left = $1 AND ea.deleted_at IS NULL
               ORDER BY oe.created_at LIMIT 1"#,
        )
        .bind(template_node_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?
        .flatten()
        .flatten();
        // A4：wait 标志（同步等待子流程终局——end 物化回调续链）
        let wait_flag: bool = {
            let raw: Option<String> = sqlx::query_scalar(
                r#"SELECT ea.timeline->>'wait' FROM isahl."zc_id_even-approve" ea
                   JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
                   WHERE oe.ref_left = $1 AND ea.deleted_at IS NULL
                   ORDER BY oe.created_at LIMIT 1"#,
            )
            .bind(template_node_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?
            .flatten();
            raw.as_deref() == Some("true")
        };
        match (target, actx.entity.as_ref()) {
            (Some(target_code), Some((entity_table, entity_id))) => {
                let target_flow: Option<i64> = sqlx::query_scalar(
                    r#"SELECT id FROM isahl.zc_id_process
                       WHERE code = $1 AND deleted_at IS NULL
                         AND EXISTS (SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                                     JOIN isahl."zc_id_stus-process" s ON s.id = ls.ref_right
                                     WHERE ls.ref_left = zc_id_process.id
                                       AND ls.deleted_at IS NULL AND s.code = 'published')
                       ORDER BY CASE WHEN zc_id_process._f_ = '实现'
                                        AND (zc_id_process._t_ = '范例'
                                             OR zc_id_process._t_ IS NULL)
                                    THEN 0 ELSE 1 END,
                                zc_id_process.updated_at DESC
                       LIMIT 1"#,
                )
                .bind(target_code.trim())
                .fetch_optional(pool)
                .await
                .map_err(|e| ApiError::Database(e.to_string()))?
                .flatten();
                match target_flow {
                    Some(target_flow_id) => {
                        // 同步触发（父流程不等待子流程终局，仅发起首链实例）：
                        // 经 BoxFuture 打破 advance_auto_node → initiate_flow 静态
                        // 递归调用环（E0733）；子流程嵌套深度有限，同步 await 可接受。
                        // A4：wait=true → 先物化子流程执行行（作锚点 + parent meta）再触发
                        let wait_exec: Option<i64> = if wait_flag {
                            match materialize_flow_execution(
                                pool,
                                target_flow_id,
                                actx.trigger,
                                "子流程（wait=true）",
                            )
                            .await
                            {
                                Ok(exec) => {
                                    sqlx::query(
                                        r#"UPDATE isahl."zc_id_proc-approve"
                                           SET meta = jsonb_set(COALESCE(meta, '{}'::jsonb), '{parent}',
                                                 $2::jsonb, true)
                                           WHERE id = $1 AND deleted_at IS NULL"#,
                                    )
                                    .bind(exec)
                                    .bind(
                                        serde_json::json!({
                                            "flow": flow_id,
                                            "node": template_node_id,
                                            "initiator": actx.initiator,
                                            "entity_table": entity_table,
                                            "entity_id": entity_id.to_string(),
                                        })
                                        .to_string(),
                                    )
                                    .execute(pool)
                                    .await
                                    .map_err(|e| ApiError::Database(e.to_string()))?;
                                    Some(exec)
                                }
                                Err(e) => {
                                    return Err(ApiError::Database(format!(
                                        "subflow '{}' (flow {}) wait 执行行物化失败: {}",
                                        target_code.trim(),
                                        flow_id,
                                        e
                                    )));
                                }
                            }
                        } else {
                            None
                        };
                        if let Err(e) = trigger_subflow(
                            pool,
                            target_flow_id,
                            actx.trigger,
                            entity_table,
                            *entity_id,
                            wait_exec,
                            actx.bus,
                        )
                        .await
                        {
                            if wait_flag {
                                return Err(ApiError::Database(format!(
                                    "subflow '{}' (flow {}) trigger failed: {}",
                                    target_code.trim(),
                                    flow_id,
                                    e
                                )));
                            }
                            common::telemetry::warn!(
                                "subflow '{}' (flow {}) trigger failed: {}",
                                target_code.trim(),
                                flow_id,
                                e
                            );
                        } else if wait_flag {
                            subflow_waiting = true;
                        }
                    }
                    None => {
                        common::telemetry::warn!(
                            "subflow node {} (flow {}): target '{}' 不存在或未发布——降级仅 gate",
                            template_node_id,
                            flow_id,
                            target_code
                        );
                    }
                }
            }
            _ => {
                common::telemetry::warn!(
                    "subflow node {} (flow {}): target 缺失或实体未绑定——降级仅 gate",
                    template_node_id,
                    flow_id
                );
            }
        }
        if subflow_waiting {
            // A4：wait=true 等待子流程终局（end 物化回调 resume_parent_flow 续链）
            return Ok(());
        }
    }
    // 2026-09-02 并行扇出显式语义：parallel 节点忽略出边 cond——全部分支
    // 并发执行（gate 已建）；汇聚由下游 branch 节点 joinRule 决定。
    if node_type == "parallel" {
        let next_ops: Option<serde_json::Value> = sqlx::query_scalar(
            r#"SELECT "next-ops" FROM isahl.zc_id_process_rr_operation
               WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
        )
        .bind(flow_id)
        .bind(template_node_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?
        .flatten();
        if let Some(nv) = next_ops {
            let entries = parse_next_op_entries(&nv);
            if !entries.is_empty() {
                advance_fan_out(pool, flow_id, &entries, actx, created).await?;
            }
        }
        return Ok(());
    }
    // 2026-09-01 refactor-flow-loop-formula-model：loop 公式驱动递归闭包——
    // 读 operation.meta loop（vars/cursor）+ 经 standard 链取 formula；
    // Rhai 求值（ctx = 实体行业务变量 + meta 局部变量）：true 且 cursor <
    // maxIter → cursor+1 写回 meta + 沿回边（next[0]）重激活循环体；false
    // 或超限 → 出口（next[1]）。旧图（loopExpr 存量，无 formula）沿用旧
    // select_targets cond 路径（gate 创建 + process_node_advancement 栈推进）。

    if node_type == "loop" {
        let meta: Option<serde_json::Value> = sqlx::query_scalar(
            r#"SELECT meta FROM isahl.zc_id_operation WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(template_node_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?
        .flatten();
        let loop_meta = meta.as_ref().and_then(|m| m.get("loop")).cloned();
        let formula: Option<String> = match &loop_meta {
            Some(meta_obj) => meta_obj
                .get("formula")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            None => None,
        };
        match formula {
            Some(f) if !f.trim().is_empty() => {
                let expr: Option<String> = sqlx::query_scalar(
                    r#"SELECT fo.expression FROM isahl.zc_id_formula fo
                       JOIN isahl.zc_id_standard_r_formula rf ON rf.ref_right = fo.id AND rf.deleted_at IS NULL
                       JOIN isahl.zc_id_operation_rr_standard rs ON rs.ref_right = rf.ref_left AND rs.deleted_at IS NULL
                       WHERE rs.ref_left = $1 AND fo.deleted_at IS NULL
                       ORDER BY rf.id LIMIT 1"#,
                )
                .bind(template_node_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| ApiError::Database(e.to_string()))?
                .flatten();
                let mut ctx = build_expr_ctx(pool, actx.entity.as_ref(), template_node_id).await;
                if let Some(meta_obj) = &loop_meta {
                    if let Some(vars) = meta_obj.get("vars").and_then(|v| v.as_object()) {
                        for (k, v) in vars {
                            ctx.insert(k.clone(), v.clone());
                        }
                    }
                }
                // 执行域键（D2 fix-approval-engine-gap-closure）：实体行 id
                // （无实体绑定 → "_"）——同模板多实体各自计迭代，互不共享 cursor。
                let cursor_key = actx
                    .entity
                    .as_ref()
                    .map(|(_, id)| id.to_string())
                    .unwrap_or_else(|| "_".to_string());
                // 读：meta.loop.cursors[key]（键缺失 → 0）；cursors 整体缺失
                // （变更前发布的旧 op 行持 flat cursor）→ 回退 flat cursor 续跑。
                let cursor: i64 = loop_meta
                    .as_ref()
                    .and_then(|m| match m.get("cursors") {
                        Some(map) => map.get(&cursor_key).and_then(|v| v.as_i64()).or(Some(0)),
                        None => m.get("cursor").and_then(|v| v.as_i64()),
                    })
                    .unwrap_or(0);
                let max_iter: i64 = loop_meta
                    .as_ref()
                    .and_then(|m| m.get("vars"))
                    .and_then(|v| v.get("maxIter"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(10);
                let continue_loop = match expr.as_deref() {
                    Some(e) => {
                        let rhai = runtime_engine::RhaiExpressionEngine::new();
                        let mut scope_ctx: std::collections::HashMap<String, serde_json::Value> =
                            ctx.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                        scope_ctx.insert("cursor".to_string(), serde_json::json!(cursor));
                        match rhai.evaluate_bool(e, &scope_ctx) {
                            Ok(v) => v,
                            Err(err) => {
                                common::telemetry::warn!(
                                    "loop node {} (flow {}): formula eval failed ({}) — fail-closed 阻断",
                                    template_node_id,
                                    flow_id,
                                    err
                                );
                                return Err(ApiError::Validation {
                                    field: "flow".into(),
                                    message: format!(
                                        "loop 节点 {template_node_id}（flow {flow_id}）公式求值失败——fail-closed 阻断。公式: {e}；错误: {err}"
                                    ),
                                });
                            }
                        }
                    }
                    None => false,
                };
                if continue_loop && cursor < max_iter {
                    let next_cursor = cursor + 1;
                    sqlx::query(
                        r#"UPDATE isahl.zc_id_operation
                           SET meta = jsonb_set(COALESCE(meta, '{}'::jsonb),
                                 '{loop,cursors}'::text[] || ARRAY[$2],
                                 to_jsonb($3::bigint), true)
                           WHERE id = $1 AND deleted_at IS NULL"#,
                    )
                    .bind(template_node_id)
                    .bind(&cursor_key)
                    .bind(next_cursor)
                    .execute(pool)
                    .await
                    .map_err(|e| ApiError::Database(e.to_string()))?;
                    common::telemetry::info!(
                        "loop node {} (flow {}): iterate {} → {} (max {})",
                        template_node_id,
                        flow_id,
                        cursor,
                        next_cursor,
                        max_iter
                    );
                    let next_ops: Option<serde_json::Value> = sqlx::query_scalar(
                        r#"SELECT "next-ops" FROM isahl.zc_id_process_rr_operation
                           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
                    )
                    .bind(flow_id)
                    .bind(template_node_id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| ApiError::Database(e.to_string()))?
                    .flatten();
                    if let Some(nv) = next_ops {
                        let entries = parse_next_op_entries(&nv);
                        if let Some(first) = entries.first() {
                            // 单步迭代：沿回边首目标推进（扇出语义与 parallel 共用）
                            advance_fan_out(
                                pool,
                                flow_id,
                                std::slice::from_ref(first),
                                actx,
                                created,
                            )
                            .await?;
                        }
                    }
                } else {
                    process_node_advancement(pool, flow_id, template_node_id, actx, created)
                        .await?;
                }
            }
            _ => {
                process_node_advancement(pool, flow_id, template_node_id, actx, created).await?;
            }
        }
        return Ok(());
    }
    process_node_advancement(pool, flow_id, template_node_id, actx, created).await?;
    Ok(())
}

/// A4：子流程终局回调续链（resume_parent_flow）——子流程执行行 meta.parent
/// 记录父流程/节点/发起人/实体；子流程 end 物化后以父上下文续推父流程
/// （process_node_advancement），实现 wait=true 的同步等待语义。
/// D3（fix-approval-engine-gap-closure）：候选限定未 ended 行并按执行域实体
/// 匹配（同模板多实体并发 wait 不再误配）；续推前先标 ended（同请求栈防重入）；
/// trigger/initiator 兜底系统身份。
async fn resume_parent_flow(
    pool: &PgPool,
    child_flow_id: i64,
    entity: Option<&(String, i64)>,
    bus: Option<&Arc<dyn DomainEventBus>>,
) -> Result<(), ApiError> {
    let parent_row: Option<(i64, serde_json::Value)> = match entity {
        Some((table, id)) => sqlx::query_as::<_, (i64, serde_json::Value)>(
            r#"SELECT id, meta->'parent' FROM isahl."zc_id_proc-approve"
               WHERE tpl_id = $1 AND deleted_at IS NULL AND meta->>'ended' IS NULL
                 AND meta->'parent'->>'entity_table' = $2
                 AND meta->'parent'->>'entity_id' = $3
               ORDER BY id DESC LIMIT 1"#,
        )
        .bind(child_flow_id)
        .bind(table)
        .bind(id.to_string())
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?,
        None => sqlx::query_as::<_, (i64, serde_json::Value)>(
            r#"SELECT id, meta->'parent' FROM isahl."zc_id_proc-approve"
               WHERE tpl_id = $1 AND deleted_at IS NULL AND meta->>'ended' IS NULL
               ORDER BY id DESC LIMIT 1"#,
        )
        .bind(child_flow_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?,
    };
    let Some((exec_row_id, parent)) = parent_row else {
        return Ok(());
    };
    let (p_flow, p_node, p_initiator, p_entity) = (
        parent.get("flow").and_then(|v| v.as_i64()),
        parent.get("node").and_then(|v| v.as_i64()),
        parent.get("initiator").and_then(|v| v.as_i64()),
        parent
            .get("entity_table")
            .and_then(|v| v.as_str())
            .zip(parent.get("entity_id").and_then(|v| v.as_i64()))
            .map(|(t, id)| (t.to_string(), id)),
    );
    let (Some(p_flow), Some(p_node)) = (p_flow, p_node) else {
        return Ok(());
    };
    common::telemetry::info!(
        "subflow {} ended — resuming parent flow {} at node {}",
        child_flow_id,
        p_flow,
        p_node
    );
    // D3：先标 ended 后续推——同请求栈内重复终局回调只见 ended 行而跳过，
    // 父流程续推只发生一次（wait 同实体并发子流程各自持独立执行行）。
    sqlx::query(
        r#"UPDATE isahl."zc_id_proc-approve"
           SET meta = jsonb_set(COALESCE(meta, '{}'::jsonb), '{ended}',
                 'true'::jsonb, true)
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(exec_row_id)
    .execute(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    let actx = AdvanceCtx {
        initiator: p_initiator.unwrap_or(SYSTEM_USER_ID),
        trigger: SYSTEM_USER_ID,
        prev_instance: None,
        comments: None,
        entity: p_entity,
        bus,
    };
    process_node_advancement(pool, p_flow, p_node, &actx, None).await?;
    Ok(())
}

/// 终局陈述物化（2026-08-29 裁决）：end 节点语义实体是 statement——
/// 范例（op→rr_statement→statement 行）经 tableoid 解析真叶表（DB 真相源，
/// 非用户输入），同表物化结论实例（tpl_id→范例，tpl_id 同表关联铁律），
/// 经 operation_rr_statement 挂 gate 实例（gate 实例→statement 实例）。
/// 范例缺失（legacy DAG 无 statement 范例）→ warn 降级跳过，不阻断推进。
async fn materialize_end_statement(
    pool: &PgPool,
    node_op_id: i64,
    gate_id: i64,
    node_label: &str,
    user_id: i64,
) -> Result<(), ApiError> {
    // 2026-09-02 end outcome 终局判定（publish 物化 operation.meta end_outcome）：
    // rejected/cancelled 终局不物化结论实例（advance 门控保证到达时 flow 无在途
    // 审批，无需取消——branch joinRule/or_sign 已取消其余分支）；complete 物化。
    let outcome: String = sqlx::query_scalar(
        r#"SELECT COALESCE(meta->>'end_outcome', 'complete')
             FROM isahl.zc_id_operation
            WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(node_op_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten()
    .unwrap_or_else(|| "complete".to_string());
    if outcome != "complete" {
        common::telemetry::warn!(
            "end node {} outcome '{}' — conclusion not materialized",
            node_op_id,
            outcome
        );
        return Ok(());
    }
    let tpl: Option<(String, i64)> = sqlx::query_as(
        r#"SELECT replace(s.tableoid::regclass::text, '"', ''), rs.ref_right
           FROM isahl.zc_id_operation_rr_statement rs
           JOIN isahl.zc_id_statement s ON s.id = rs.ref_right AND s.deleted_at IS NULL
           WHERE rs.ref_left = $1 AND rs.deleted_at IS NULL
           ORDER BY rs.id LIMIT 1"#,
    )
    .bind(node_op_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    let Some((leaf, tpl_id)) = tpl else {
        common::telemetry::warn!(
            "end node {} has no statement template (legacy DAG) — conclusion not materialized",
            node_op_id
        );
        return Ok(());
    };
    let Some(insert_sql) = crate::context_meta::statement_leaf_insert_sql(&leaf) else {
        common::telemetry::warn!(
            "statement leaf {} not in compiled whitelist — conclusion not materialized",
            leaf
        );
        return Ok(());
    };
    let stmt_id: i64 = sqlx::query_scalar(insert_sql)
        .bind(node_label)
        .bind(Option::<String>::None)
        .bind(tpl_id)
        .bind(user_id)
        // 类写入契约 §4.3.3：执行期结论实例 = 实现·实例（范例行 tpl_id 关联）
        .bind("实现")
        .bind("实例")
        .fetch_one(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_statement (ref_left, ref_right, created_by_id)
           VALUES ($1, $2, $3)"#,
    )
    .bind(gate_id)
    .bind(stmt_id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    common::telemetry::info!(
        "flow conclusion materialized: statement instance {} (tpl {}) in {}",
        stmt_id,
        tpl_id,
        leaf
    );
    Ok(())
}

/// 从模板节点读取 next-ops，继续推进所有自动节点
/// 使用栈（非递归）+ 深度限制（防循环）
/// actx.prev_instance：链指针透传——自动节点链下游的人工实例
/// fk_previous 仍指向真正的源审批实例，撤回级联链不被自动节点跳断。
async fn process_node_advancement(
    pool: &PgPool,
    flow_id: i64,
    start_node_id: i64,
    actx: &AdvanceCtx<'_>,
    mut created: Option<&mut Vec<i64>>,
) -> Result<(), ApiError> {
    let mut stack: Vec<(i64, u32)> = vec![(start_node_id, 0)];
    const MAX_DEPTH: u32 = 50;
    let ctx = build_expr_ctx(pool, actx.entity.as_ref(), start_node_id).await;

    while let Some((current, depth)) = stack.pop() {
        if depth >= MAX_DEPTH {
            common::telemetry::warn!(
                "process_node_advancement: max depth reached at node {}",
                current
            );
            continue;
        }

        let next_ops: Option<serde_json::Value> = sqlx::query_scalar(
            r#"SELECT "next-ops" FROM isahl.zc_id_process_rr_operation
               WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
        )
        .bind(flow_id)
        .bind(current)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?
        .flatten();

        let entries = match next_ops {
            Some(v) => parse_next_op_entries(&v),
            _ => continue,
        };
        // P1-5 条件选边（与 advance_flow 同一规则；condition exclusive 按源节点 routing）
        let routing = node_routing(pool, current).await?;
        let targets = select_targets(&entries, &ctx, routing == "exclusive");
        if targets.is_empty() {
            common::telemetry::warn!(
                "process_node_advancement: no outgoing edge selected at node {} (flow {})",
                current,
                flow_id
            );
            continue;
        }

        for target_id in targets {
            let node_info = sqlx::query_as::<_, (String, String)>(
                r#"SELECT CASE
                        WHEN c.code IS NOT NULL
                          AND c.code NOT IN ('and_sign', 'or_sign', 'sequential')
                          THEN c.code
                        WHEN replace(o.tableoid::regclass::text, '"', '') = 'zc_id_oper-approve' THEN 'approve'
                        WHEN replace(o.tableoid::regclass::text, '"', '') = 'zc_id_oper-action' THEN 'action'
                        WHEN replace(o.tableoid::regclass::text, '"', '') = 'zc_id_oper-check' THEN 'review'
                        WHEN EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_statement rs
                                     WHERE rs.ref_left = o.id AND rs.deleted_at IS NULL)
                          THEN 'end'
                        ELSE 'gate'
                      END,
                      COALESCE(rro.comments, o.notice)
                   FROM isahl.zc_id_process_rr_operation rro
                   JOIN isahl.zc_id_operation o ON o.id = rro.ref_right AND o.deleted_at IS NULL
                   LEFT JOIN isahl."zc_id_cate-proc_op" c
                     ON c.id = o."ck_cate-proc_op" AND c.deleted_at IS NULL
                   WHERE rro.ref_left = $1 AND rro.ref_right = $2 AND rro.deleted_at IS NULL"#,
            )
            .bind(flow_id)
            .bind(target_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?;

            let (node_type, node_label) = match node_info {
                Some(n) => n,
                None => continue,
            };

            match node_type.as_str() {
                // review/action 亦为人工节点（评审/执行岗位），统一创建实例
                "oper-approve" | "approve" | "approval" | "review" | "action" | "vote" => {
                    let ids =
                        create_approval_instances(pool, target_id, &node_label, actx, None).await?;
                    if let Some(v) = created.as_deref_mut() {
                        v.extend(ids);
                    }
                }
                _ => {
                    let _ =
                        create_gate_operation(pool, target_id, flow_id, &node_label, actx.trigger)
                            .await?;
                    stack.push((target_id, depth + 1));
                }
            }
        }
    }

    Ok(())
}

// ============================================================
// 流程发起（fix-flow-designer-runtime-chain D4）
// ============================================================

/// 以业务实体发起已发布流程：定位 start 节点 → next-ops → 实体行上下文
/// 条件选边 → 创建首链实例（实体桥随实例写入，comments 为可读摘要）。
/// 返回首链实例 id 列表（可能为空——首跳全为自动节点时实体绑定仍写入）。
pub async fn initiate_flow(
    pool: &PgPool,
    flow_id: i64,
    user_id: i64,
    entity_table: &str,
    entity_id: i64,
    // 本次执行的 实现·实例 行 id（flow-lifecycle-split）：非空时写为首链
    // 实例的 fk_previous 链根——执行 = fk_previous 链，根挂在执行行上。
    execution_anchor: Option<i64>,
    bus: Option<&Arc<dyn DomainEventBus>>,
) -> Result<Vec<i64>, ApiError> {
    // start 节点（活跃批）：Flow-Design 图端点由 meta 管理（§4.4.1）——
    // 解析 nodes 中 type='start' 的图内编号（D4：发布版本图快照优先，缺失
    // 回退 process.meta），经 rr_operation.code 定位首节点 operation
    // （code = 图内节点编号，publish 物化写入；§4.4 code 语义）。
    let meta: Option<serde_json::Value> = flow_graph(pool, flow_id).await?;
    let start_graph_id: Option<String> = meta.as_ref().and_then(|m| {
        m.get("nodes").and_then(|v| v.as_array()).and_then(|arr| {
            arr.iter()
                .find(|n| n.get("type").and_then(|t| t.as_str()) == Some("start"))
                .and_then(|n| n.get("id").and_then(|i| i.as_str()).map(str::to_string))
        })
    });
    let start_node: Option<i64> = match start_graph_id {
        Some(gid) => sqlx::query_scalar(
            r#"SELECT ref_right FROM isahl.zc_id_process_rr_operation
               WHERE ref_left = $1 AND code = $2 AND deleted_at IS NULL
               ORDER BY id LIMIT 1"#,
        )
        .bind(flow_id)
        .bind(&gid)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?
        .flatten(),
        None => None,
    };
    let Some(start_node) = start_node else {
        return Err(ApiError::Validation {
            field: "flow".into(),
            message: format!("flow {flow_id} 无 start 节点（meta 图不完整或未发布）"),
        });
    };

    // 实体上下文（首跳选边即用实体行字段）
    let ctx = build_expr_ctx(
        pool,
        Some(&(entity_table.to_string(), entity_id)),
        start_node,
    )
    .await;
    let next_ops: Option<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT "next-ops" FROM isahl.zc_id_process_rr_operation
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .bind(start_node)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten();
    let entries = match next_ops {
        Some(v) => parse_next_op_entries(&v),
        _ => return Ok(vec![]),
    };
    if entries.is_empty() {
        return Ok(vec![]);
    }

    let targets = select_targets(&entries, &ctx, false);
    if targets.is_empty() {
        common::telemetry::warn!(
            "initiate_flow: no outgoing edge selected at start {} (flow {}) — no instances",
            start_node,
            flow_id
        );
        return Ok(vec![]);
    }

    let summary = format!("流程发起：实体 {entity_table}#{entity_id}");
    let actx = AdvanceCtx {
        initiator: user_id,
        trigger: user_id,
        prev_instance: execution_anchor,
        comments: Some(&summary),
        entity: Some((entity_table.to_string(), entity_id)),
        bus,
    };

    let mut created = Vec::new();
    for target_id in targets {
        let node_info = sqlx::query_as::<_, (String, String)>(
            r#"SELECT CASE
                        WHEN c.code IS NOT NULL
                          AND c.code NOT IN ('and_sign', 'or_sign', 'sequential')
                          THEN c.code
                        WHEN replace(o.tableoid::regclass::text, '"', '') = 'zc_id_oper-approve' THEN 'approve'
                        WHEN replace(o.tableoid::regclass::text, '"', '') = 'zc_id_oper-action' THEN 'action'
                        WHEN replace(o.tableoid::regclass::text, '"', '') = 'zc_id_oper-check' THEN 'review'
                        WHEN EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_statement rs
                                     WHERE rs.ref_left = o.id AND rs.deleted_at IS NULL)
                          THEN 'end'
                        ELSE 'gate'
                      END,
                      COALESCE(rro.comments, o.notice)
               FROM isahl.zc_id_process_rr_operation rro
               JOIN isahl.zc_id_operation o ON o.id = rro.ref_right AND o.deleted_at IS NULL
               LEFT JOIN isahl."zc_id_cate-proc_op" c
                 ON c.id = o."ck_cate-proc_op" AND c.deleted_at IS NULL
               WHERE rro.ref_left = $1 AND rro.ref_right = $2 AND rro.deleted_at IS NULL"#,
        )
        .bind(flow_id)
        .bind(target_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
        let Some((node_type, node_label)) = node_info else {
            continue;
        };
        match node_type.as_str() {
            // review/action 亦为人工节点（评审/执行岗位），统一创建实例
            "oper-approve" | "approve" | "approval" | "review" | "action" | "vote" => {
                let ids =
                    create_approval_instances(pool, target_id, &node_label, &actx, None).await?;
                created.extend(ids);
            }
            _ => {
                advance_auto_node(
                    pool,
                    flow_id,
                    target_id,
                    &node_type,
                    &node_label,
                    &actx,
                    Some(&mut created),
                )
                .await?;
            }
        }
    }
    Ok(created)
}

/// 物化执行实例行（flow-lifecycle-split）：以 实现·范例 行为模板，在同叶表
/// 物化一行「实现·实例」（function 码 `↓.{suffix}` → `↓_{suffix}` 换前缀交
/// trigger 派生，tpl_id → 范例行）。返回行 id——本次执行首链审批实例的
/// fk_previous 链根锚点。
pub(crate) async fn materialize_flow_execution(
    pool: &PgPool,
    exemplar_id: i64,
    user_id: i64,
    summary: &str,
) -> Result<i64, ApiError> {
    #[allow(clippy::type_complexity)]
    // SQL 行元组（tableoid/notice/scene/factor/function-code），一次性局部
    let row: Option<(
        String,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<String>,
    )> = sqlx::query_as(
        r#"SELECT replace(e.tableoid::regclass::text, '"', ''), e.notice, e.dk_scene,
                  e.dk_factor,
                  (SELECT f.code FROM isahl.zc_id_function f
                   WHERE f.id = e.dk_function AND f.deleted_at IS NULL LIMIT 1)
           FROM isahl.zc_id_process e
           WHERE e.id = $1 AND e.deleted_at IS NULL
             AND e._f_ = '实现' AND e._t_ = '范例'"#,
    )
    .bind(exemplar_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    let Some((branch, notice, dk_scene, dk_factor, fn_code)) = row else {
        return Err(ApiError::Validation {
            field: "flow".into(),
            message: format!("流程 {exemplar_id} 不存在或不是实现·范例（_f_=实现/_t_=范例）"),
        });
    };
    // function 码 → 实现·实例码：`↓.*` 前缀换 `↓_{suffix}`；非 ↓. 前缀
    //    （含 NULL/字典缺码）置 NULL——类契约由 _f_/_t_ 字面量承载（见 exec_insert_sql），
    //    dk 码侧仅做一致性装饰，不阻塞发起
    let exec_fn_id: Option<i64> = match fn_code.as_deref().filter(|c| c.starts_with("↓.")) {
        Some(code) => {
            let exec_fn_code = format!("↓_{}", &code["↓.".len()..]);
            sqlx::query_scalar(
                r#"SELECT id FROM isahl.zc_id_function
                   WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
            )
            .bind(&exec_fn_code)
            .fetch_optional(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?
            .flatten()
        }
        None => None,
    };
    let insert_sql = match branch.as_str() {
        "zc_id_proc-approve" => exec_insert_sql("isahl.\"zc_id_proc-approve\""),
        "zc_id_proc-cicd" => exec_insert_sql("isahl.\"zc_id_proc-cicd\""),
        "zc_id_proc-loading" => exec_insert_sql("isahl.\"zc_id_proc-loading\""),
        "zc_id_proc-make" => exec_insert_sql("isahl.\"zc_id_proc-make\""),
        "zc_id_proc-project" => exec_insert_sql("isahl.\"zc_id_proc-project\""),
        "zc_id_proc-purchase" => exec_insert_sql("isahl.\"zc_id_proc-purchase\""),
        "zc_id_proc-service" => exec_insert_sql("isahl.\"zc_id_proc-service\""),
        other => {
            return Err(ApiError::Validation {
                field: "branch".into(),
                message: format!("未知流程叶表分支 '{other}'——不可物化执行行"),
            });
        }
    };
    let exec_id: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(insert_sql.as_str()))
        .bind(exemplar_id)
        .bind(exec_fn_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(user_id)
        .bind(summary)
        .bind(&notice)
        // 静态 SQL（表名编译期常量），AssertSqlSafe 声明已审计
        .fetch_one(pool)
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
    Ok(exec_id)
}

/// 实现·实例 INSERT：notice=$7 范例原名、comments=$6 执行摘要、
/// dk 坐标换 实现·实例 function 码、tpl_id → 范例行；code 不设（发布位归范例）。
fn exec_insert_sql(table: &str) -> String {
    format!(
        r#"INSERT INTO {table}
           (notice, comments, dk_scene, dk_factor, dk_function, tpl_id, created_by_id,
            _f_, _t_)
           VALUES ($7, $6, $3, $4, $2, $1, $5, '实现', '实例')
           RETURNING id"#
    )
}

// ============================================================
// 项目流程初始化
// ============================================================

/// 项目创建后的模板子项复制。
///
/// 流程绑定（project_template.comments.flowTemplateId）已随 comments 文本化移除——
/// remove-comments-json-embedding 降级：项目不再自动实例化首阶段流程。
pub async fn init_project_flow(
    pool: &PgPool,
    project_id: i64,
    user_id: i64,
) -> Result<(), ApiError> {
    copy_template_children(pool, project_id, user_id).await?;
    Ok(())
}

/// 复制模板项目的 gantt/risk 子项到新项目（D10）
///
/// project.tpl_id 非空时执行；模板无子项时跳过（log 记录复制行数）。
/// - gantt 子项：`zc_id_plan-project`（fk_project = 模板项目 id，项目计划叶表）
///   → 复制到新项目（fk_project 改指新项目 id）
/// - risk 子项：`zc_id_even-accident`（fk_subject = 模板项目 id）→ 复制到新项目
///   （fk_subject 改指新项目 id）
async fn copy_template_children(
    pool: &PgPool,
    project_id: i64,
    user_id: i64,
) -> Result<(), ApiError> {
    let tpl_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT p.tpl_id FROM isahl.zc_id_project p
           WHERE p.id = $1 AND p.deleted_at IS NULL"#,
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(format!("copy_template_children: read tpl_id: {}", e)))?
    .flatten();

    let Some(tpl_id) = tpl_id else {
        return Ok(());
    };

    let gantt_copied = sqlx::query(
        r#"INSERT INTO isahl."zc_id_plan-project"
           (id, notice, code, qk_progress, "qk_date-segm", comments, cron, exclude, lk_health,
            "qk_time-segm", sort, t_color_, created_by_id, dk_scene, dk_factor, dk_function,
            fk_project)
           SELECT isahl.gen_next_zuid(), notice, code, qk_progress, "qk_date-segm", comments,
                  cron, exclude, lk_health, "qk_time-segm", sort, t_color_, $1, dk_scene,
                  dk_factor, dk_function, $2
           FROM isahl."zc_id_plan-project"
           WHERE fk_project = $3 AND deleted_at IS NULL"#,
    )
    .bind(user_id)
    .bind(project_id)
    .bind(tpl_id)
    .execute(pool)
    .await
    .map_err(|e| ApiError::Database(format!("copy gantt items: {}", e)))?
    .rows_affected();

    let risk_copied = sqlx::query(
        r#"INSERT INTO isahl."zc_id_even-accident"
           (id, notice, code, lk_risk, comments, fk_place, fk_subject, lk_severity, qk_date,
            t_color_, created_by_id, dk_scene, dk_factor, dk_function)
           SELECT isahl.gen_next_zuid(), notice, code, lk_risk, comments, fk_place, $1,
                  lk_severity, qk_date, t_color_, $2, dk_scene, dk_factor, dk_function
           FROM isahl."zc_id_even-accident"
           WHERE fk_subject = $3 AND deleted_at IS NULL"#,
    )
    .bind(project_id)
    .bind(user_id)
    .bind(tpl_id)
    .execute(pool)
    .await
    .map_err(|e| ApiError::Database(format!("copy risk items: {}", e)))?
    .rows_affected();

    if gantt_copied > 0 || risk_copied > 0 {
        common::telemetry::info!(
            "copy_template_children: project {project_id} from template {tpl_id}: gantt={gantt_copied} risk={risk_copied}"
        );
    }
    Ok(())
}

/// 委托自动转派目标实例表
#[derive(Clone, Copy)]
enum DelegationTarget {
    Approve, // "isahl.zc_id_oper-approve"
    Gate,    // "isahl.zc_id_oper-gate"
}

/// 委托自动转派（D5）：审批实例创建后，若当前审批人存在有效委托规则
/// （zc_id_operation._t_='delegation-rule'，fk_subject=当前审批人，时间窗
/// qk_period→zc_id_segm-date（date_st/date_ed）优先、legacy comments JSON
/// validFrom/validUntil 兜底，覆盖 now()，缺省边界视为不设限），将实例
/// fk_operator 改派为受托人。
/// 返回是否发生转派。
pub(crate) async fn apply_delegation(
    pool: &PgPool,
    instance_id: i64,
    current_operator: i64,
) -> Result<bool, ApiError> {
    apply_delegation_inner(
        pool,
        DelegationTarget::Approve,
        instance_id,
        current_operator,
    )
    .await
}

/// 按实例表执行委托转派（oper-approve / oper-gate 共用；UPDATE 语句为编译期常量）
async fn apply_delegation_inner(
    pool: &PgPool,
    target: DelegationTarget,
    instance_id: i64,
    current_operator: i64,
) -> Result<bool, ApiError> {
    // 1. 查当前审批人的候选委托规则（时间窗仅认 qk_period→zc_id_segm-date 列语义；
    //    comments 承载的 legacy 时间窗已随 comments 文本化失效——无 qk_period 的规则跳过）
    let rules = sqlx::query_as::<
        _,
        (
            i64,
            Option<i64>,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<chrono::DateTime<chrono::Utc>>,
        ),
    >(
        r#"SELECT o.id, o.fk_operator, sd.date_st, sd.date_ed
           FROM isahl.zc_id_operation o
           LEFT JOIN isahl."zc_id_segm-date" sd ON sd.id = o.qk_period AND sd.deleted_at IS NULL
           WHERE o._t_ = 'delegation-rule'
             AND o.fk_subject = $1
             AND o.deleted_at IS NULL
             AND o.qk_period IS NOT NULL
           ORDER BY o.id
           LIMIT 20"#,
    )
    .bind(current_operator)
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::Database(format!("apply_delegation: find rules: {}", e)))?;

    let now = chrono::Utc::now();
    let mut assignee: Option<i64> = None;
    for (_, operator, date_st, date_ed) in rules {
        // segm-date 时间窗（列语义 = 委托起止时间窗）；缺省边界视为不设限
        let in_window = date_st.is_none_or(|f| f <= now) && date_ed.is_none_or(|u| u >= now);
        if !in_window {
            continue;
        }
        if let Some(op) = operator {
            assignee = Some(op);
            break;
        }
    }

    let Some(assignee) = assignee else {
        return Ok(false);
    };

    // 2. 改派实例审批人（fk_operator）
    let update_sql = match target {
        DelegationTarget::Approve => {
            r#"UPDATE isahl."zc_id_oper-approve" SET fk_operator = $1, updated_at = NOW() WHERE id = $2"#
        }
        DelegationTarget::Gate => {
            r#"UPDATE isahl."zc_id_oper-gate" SET fk_operator = $1, updated_at = NOW() WHERE id = $2"#
        }
    };
    let updated = sqlx::query(update_sql)
        .bind(assignee)
        .bind(instance_id)
        .execute(pool)
        .await
        .map_err(|e| ApiError::Database(format!("apply_delegation: reassign: {}", e)))?;
    if updated.rows_affected() == 0 {
        // 实例不存在
        return Ok(false);
    }

    common::telemetry::info!(
        "delegation: instance {} reassigned from user {} to user {}",
        instance_id,
        current_operator,
        assignee
    );
    Ok(true)
}
