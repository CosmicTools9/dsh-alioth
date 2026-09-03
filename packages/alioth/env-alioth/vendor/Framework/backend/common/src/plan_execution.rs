//! 计划执行实例化（plan → oper-planing）公共设施
//!
//! P5 泛化：任何 plan 族实体的完成/推进动作都产生 `zc_id_oper-planing` 执行实例
//! （fk_subject=计划 id），状态桥（r_primary-status）只是快捷投影，operation 为
//! 可审计的事实源（SLA 质量审计模式的泛化）。事实引用（fact_ref）经 comments
//! JSON 结构化承载（如 {"table":"zc_id_bill-check","id":...}），意图与事实可回溯对账。
//!
//! ## 时间对称模型与三级分解链（本体语义）
//!
//! **event 与 task 时间对称**：同一"时间切片"概念在现在时刻两侧的镜像——
//! 过去到现在时间轴上，已发生的切片叫**事件（event，过去事实）**，
//! 未发生的切片叫**任务（task，未来待办）**。任务完成即切片翻转
//! （task → event），operation 是翻转时刻的动作痕迹。
//!
//! ```text
//! 时间轴：─────过去──────┼─────未来──────
//!         event（已发生）    task（将发生）
//!
//!              ┌──(zc_id_plan_rr_event)──> event 群（过去切片）
//!    plan ─────┤
//!              └──(zc_id_plan_rr_task)───> task 群（未来切片）
//!   │                                            │ 完成翻转（operation 动作痕迹）
//!   └──直接执行（record_plan_execution）──> oper-planing；task → event
//! ```
//!
//! **双桥锚定**：`plan_rr_task`（未来）+ `plan_rr_event`（过去）刚好把计划在时间轴上
//! 补齐要素——plan 是组织锚点，两张对称桥是其向两侧时间方向的展开；配 `qk_date-segm`
//! （时间跨度）与 `qk_progress`（→ zc_id_rati-progress 标量，进度非实体），
//! **甘特图自然从该体系完整绘制**（跨度=条、task=前瞻、event=已完成、progress=百分比）。
//! 甘特是 plan 的投影视图而非独立模型。
//!
//! plan 是跨时间的意图组织（顶层），task/event 是其时间切片的对称两态；
//! operation 经标准关系族桥 `operation_rr_task`（ref_left=operation 声明归属 task，
//! 同范式 rr_event/rr_approve/rr_bill）挂接。`plan_execution_chain`
//! 提供读侧完整链查询（计划 → 任务分解 → 各任务操作 → 直接执行实例）。
//!
//! 写入范式：operation 族（gen_next_zuid 默认 + fk_operator/fk_subject + op_number），
//! 对齐 scheduler::record_execution / oper-approve 先例。
use sqlx::PgPool;

/// 计划执行实例写入错误
#[derive(Debug, thiserror::Error)]
pub enum PlanExecutionError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// 事实引用（P6：意图-事实对账；comments JSON 承载，不硬造 event 行）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FactRef {
    /// 事实所在表（如 "zc_id_bill-check"）
    pub table: String,
    /// 事实行 id（serde_zuid 兼容）
    #[serde(with = "crate::serde_zuid")]
    pub id: i64,
}

/// 写计划执行实例（幂等由调用方业务保证；单条写入）
///
/// - `plan_id`：计划行 id（任意 zc_id_plan 族子表行——父表 fk_subject 引用）
/// - `summary`：执行摘要（如 "付款计划兑付 → paid"、"日程标记完成"）
/// - `fact_ref`：保留参数位（此前经 comments JSON 承载，已文本化；仅入摘要文本）
/// - `user_id`：执行者（业务操作人；系统身份传 SYSTEM_USER_ID）
pub async fn record_plan_execution(
    pool: &PgPool,
    plan_id: i64,
    summary: &str,
    fact_ref: Option<&FactRef>,
    user_id: i64,
) -> Result<(), PlanExecutionError> {
    // comments 为纯文本语义（remove-comments-json-embedding）：人类可读摘要（事实引用以 文本#id 并入）
    let mut comments_text = format!("执行摘要：{summary}");
    if let Some(fr) = fact_ref {
        comments_text.push_str(&format!("（事实：{}#{}）", fr.table, fr.id));
    }
    sqlx::query(
        r#"
        INSERT INTO isahl."zc_id_oper-planing"
            (notice, code, fk_subject, fk_operator, comments, created_by_id)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(format!("计划执行：{summary}"))
    .bind(format!("plan-exec-{plan_id}"))
    .bind(plan_id)
    .bind(user_id)
    .bind(comments_text)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 链节点：任务分解及其操作
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlanChainTask {
    /// 任务行 id（zc_id_task 族）
    #[serde(with = "crate::serde_zuid")]
    pub task_id: i64,
    /// 任务 notice
    pub notice: Option<String>,
    /// 任务的操作分解（operation_rr_task 正桥，按 operation.id 时序）
    pub operations: Vec<PlanChainOperation>,
}

/// 链上操作节点（operation 族行，经 operation_rr_task 挂接：ref_left=operation 归属 task）
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlanChainOperation {
    /// operation 族行 id
    #[serde(with = "crate::serde_zuid")]
    pub operation_id: i64,
    /// operation notice
    pub notice: Option<String>,
    /// 来源表（tableoid 区分 oper-planing/oper-approve 等）
    pub source_table: String,
}

/// 计划完整分解执行链（读侧查询：直接执行实例 + task 分解 + 各任务操作）
///
/// 返回：(直接执行实例 ids, task 分解)。直接实例 = oper-planing.fk_subject = plan；
/// 分解 = plan_rr_task → task ← operation_rr_task（ref_left=operation 声明归属，
/// operation 标准关系族同范式 rr_event/rr_approve/rr_bill 等）→ operation 族。
#[allow(clippy::type_complexity)]
pub async fn plan_execution_chain(
    pool: &PgPool,
    plan_id: i64,
) -> Result<(Vec<i64>, Vec<PlanChainTask>), PlanExecutionError> {
    // 直接执行实例（P5 写入路径）
    let direct: Vec<(i64,)> = sqlx::query_as(
        r#"SELECT id FROM isahl."zc_id_oper-planing"
           WHERE fk_subject = $1 AND deleted_at IS NULL ORDER BY id"#,
    )
    .bind(plan_id)
    .fetch_all(pool)
    .await?;

    // task 分解（plan_rr_task）
    let tasks: Vec<(i64, Option<String>)> = sqlx::query_as(
        r#"SELECT t.id, t.notice FROM isahl."zc_id_plan_rr_task" rr
           JOIN isahl.zc_id_task t ON t.id = rr.ref_right AND t.deleted_at IS NULL
           WHERE rr.ref_left = $1 AND rr.deleted_at IS NULL ORDER BY t.id"#,
    )
    .bind(plan_id)
    .fetch_all(pool)
    .await?;

    let mut chain = Vec::with_capacity(tasks.len());
    for (task_id, notice) in tasks {
        // 任务操作（operation_rr_task 正桥：ref_left=operation 声明归属 task——
        // operation 标准关系族同范式 rr_event/rr_approve/rr_bill 等；
        // ref_right=task 反查。无 op_seq 列，按 operation.id 时序近似）
        let ops: Vec<(i64, Option<String>, String)> = sqlx::query_as(
            r#"SELECT o.id, o.notice, c.relname
               FROM isahl."zc_id_operation_rr_task" rr
               JOIN isahl.zc_id_operation o ON o.id = rr.ref_left
               JOIN pg_class c ON c.oid = o.tableoid
               WHERE rr.ref_right = $1 AND rr.deleted_at IS NULL
               ORDER BY o.id"#,
        )
        .bind(task_id)
        .fetch_all(pool)
        .await?;
        chain.push(PlanChainTask {
            task_id,
            notice,
            operations: ops
                .into_iter()
                .map(
                    |(operation_id, op_notice, source_table)| PlanChainOperation {
                        operation_id,
                        notice: op_notice,
                        source_table,
                    },
                )
                .collect(),
        });
    }
    Ok((direct.into_iter().map(|(id,)| id).collect(), chain))
}

/// 切片翻转（task → event）：任务/计划完成时的对称完备三件套
///
/// 时间对称模型：未来切片（task/plan 待办）完成 → 过去切片（event）落定。
/// 本 helper 落事件侧两件：`zc_id_even-alert`（叶表，事件切片）+ `zc_id_plan_rr_event`
/// （计划-事件关联）；动作侧（oper-planing）由 `record_plan_execution` 承载（调用方
/// 先/后调用）。`task_id` 存在时同时 `zc_id_operation_rr_task` 挂接动作归属。
///
/// 幂等性由调用方业务保证（如状态桥先判再翻）；fail-open 语义由调用方决定。
pub async fn record_slice_flip(
    pool: &PgPool,
    plan_id: i64,
    task_id: Option<i64>,
    summary: &str,
    user_id: i64,
) -> Result<(i64, Option<i64>), PlanExecutionError> {
    // 事件切片（even-alert 叶表——zc_id_event 有子表，INSERT 必须落叶）
    let event_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_even-alert"
           (notice, code, comments, created_by_id)
           VALUES ($1, $2, $3, $4) RETURNING id"#,
    )
    .bind(format!("完成：{summary}"))
    .bind(format!("flip-{plan_id}"))
    .bind(
        serde_json::json!({"slice_flip": true, "plan_id": plan_id, "summary": summary}).to_string(),
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    // 计划-事件关联（plan_rr_event：意图与落定事实可回溯对账）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_plan_rr_event" (notice, ref_left, ref_right, created_by_id)
           VALUES ($1, $2, $3, $4)"#,
    )
    .bind(format!("flip：{summary}"))
    .bind(plan_id)
    .bind(event_id)
    .bind(user_id)
    .execute(pool)
    .await?;

    // 任务归属挂接（operation_rr_task 正桥：动作归属 task）——先写动作再挂
    let mut op_link: Option<i64> = None;
    if let Some(tid) = task_id {
        let op_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_oper-planing"
               (notice, code, fk_subject, fk_operator, created_by_id)
               VALUES ($1, $2, $3, $4, $4) RETURNING id"#,
        )
        .bind(format!("翻转动作：{summary}"))
        .bind(format!("flip-op-{plan_id}"))
        .bind(plan_id)
        .bind(user_id)
        .fetch_one(pool)
        .await?;
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_operation_rr_task" (notice, ref_left, ref_right, created_by_id)
               VALUES ($1, $2, $3, $4)"#,
        )
        .bind(format!("task-op：{summary}"))
        .bind(op_id)
        .bind(tid)
        .bind(user_id)
        .execute(pool)
        .await?;
        op_link = Some(op_id);
    }
    Ok((event_id, op_link))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fact_ref_serializes() {
        let f = FactRef {
            table: "zc_id_bill-check".to_string(),
            id: 123,
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("zc_id_bill-check"));
        let back: FactRef = serde_json::from_str(&s).unwrap();
        assert_eq!(back.id, 123);
    }
}
