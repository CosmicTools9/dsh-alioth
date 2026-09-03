//! 流程上下文域判定（refactor-flow-node-operation-model 阶段 3）：
//!
//! `zc_id_proc-context` 是 event/task 的父表（继承链：proc-context → zc_id_event /
//! zc_id_task；zc_id_event → zc_id_even-approve → zc_id_appr-*）。
//! 新建审批流程时在选定域的父表创建流程专属上下文范例行
//! （`_t_='flow-context'`），`process.fk_context` 指向该范例行——
//! 模板（范例）链：even-approve(范例) ↔ operation(范例) ↔ process(范例)
//! + process.fk_context → even-approve(范例)。
//!
//! 本模块独立于 AUTO-GENERATED context_meta.rs（避免被生成器覆盖）。

/// 上下文叶表 → 域（task / event / approve）。
/// 前缀规则与生成器白名单一致：
/// - `zc_id_appr-*`（含 zc_id_even-approve 自身）→ approve（审批事件族）
/// - `zc_id_task-*` → task
/// - `zc_id_even-*`（非 approve）→ event
/// - 其余（proc-* 流程范畴等）→ None
pub fn domain_of_leaf(table: &str) -> Option<&'static str> {
    if table == "zc_id_even-approve" || table.starts_with("zc_id_appr-") {
        Some("approve")
    } else if table.starts_with("zc_id_task-") {
        Some("task")
    } else if table.starts_with("zc_id_even-") {
        Some("event")
    } else {
        None
    }
}

/// 域父表（流程上下文范例行落表）：approve→zc_id_even-approve /
/// task→zc_id_task / event→zc_id_event。
pub fn context_family_table(domain: &str) -> Option<&'static str> {
    match domain {
        "approve" => Some("zc_id_even-approve"),
        "task" => Some("zc_id_task"),
        "event" => Some("zc_id_event"),
        _ => None,
    }
}

/// 流程上下文范例行 INSERT SQL（静态白名单分表，防动态表名注入）。
pub fn flow_context_insert_sql(domain: &str) -> Option<&'static str> {
    match domain {
        "approve" => Some(
            r#"INSERT INTO isahl."zc_id_even-approve" (notice, _t_, created_by_id)
               VALUES ($1, 'flow-context', $2) RETURNING id"#,
        ),
        "task" => Some(
            r#"INSERT INTO isahl."zc_id_task" (notice, _t_, created_by_id)
               VALUES ($1, 'flow-context', $2) RETURNING id"#,
        ),
        "event" => Some(
            r#"INSERT INTO isahl."zc_id_event" (notice, _t_, created_by_id)
               VALUES ($1, 'flow-context', $2) RETURNING id"#,
        ),
        _ => None,
    }
}
