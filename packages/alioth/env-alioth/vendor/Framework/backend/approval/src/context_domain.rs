//! 流程上下文域判定（refactor-flow-node-operation-model 阶段 3）：
//!
//! `zc_id_proc-context` 是 event/task 的父表（继承链：proc-context → zc_id_event /
//! zc_id_task；zc_id_event → zc_id_even-approve → zc_id_appr-*）。
//! 新建审批流程时在选定域创建流程专属上下文范例行（`_t_='flow-context'`），
//! 绑定经 `zc_id_process_rr_context` 桥落行（ref_left=流程行，ref_right=范例行）——
//! 模板（范例）链：even-approve(范例) ↔ operation(范例) ↔ process(范例)
//! + process → rr_context → even-approve(范例)。
//!
//! 落点（route-flow-context-to-domain-leaves）：范例行 MUST 落**域内声明叶表**
//! （= 调用方声明的 `context_table`，approve→appr-* / task→task-* / event→even-*），
//! MUST NOT 直写域父表（`zc_id_even-approve` / `zc_id_task` / `zc_id_event`）——
//! §ENVIRONMENT_SPEC「所有业务行 MUST 写入叶表」。模板行与业务实例行由 `_t_`
//! 判别（flow-context / 实例），与落点解耦（advance.rs resolve_entity_ref）。
//!
//! 本模块独立于 AUTO-GENERATED context_meta.rs（避免被生成器覆盖）。

/// 上下文表 → 域（task / event / approve）。
/// 前缀规则与生成器白名单一致：
/// - `zc_id_appr-*`（含基表 `zc_id_even-approve` 自身）→ approve（审批事件族）
/// - `zc_id_task-*`（含基表 `zc_id_task`）→ task
/// - `zc_id_even-*`（含基表 `zc_id_event`）→ event
/// - 其余（proc-* 流程范畴等）→ None
///
/// 三个域基表同样归域：兼容遗留 flow-context 行（父表落点，本 change 前产物）与
/// 父表落点的 scope-definition 行的**读路径**域判定；写路径（`flow_context_insert_sql`）
/// 不受理父表。
pub fn domain_of_leaf(table: &str) -> Option<&'static str> {
    if table == "zc_id_even-approve" || table.starts_with("zc_id_appr-") {
        Some("approve")
    } else if table == "zc_id_task" || table.starts_with("zc_id_task-") {
        Some("task")
    } else if table == "zc_id_event" || table.starts_with("zc_id_even-") {
        Some("event")
    } else {
        None
    }
}

/// 流程上下文范例行 INSERT SQL（静态白名单分表，防动态表名注入）：
/// 落点 = 域内声明叶表（`context_meta::SCOPE_DOMAINS` 白名单同源，32 张）；
/// 域/表不匹配或落点为域父表 → None（调用方 fail-closed 400）。
///
/// 列清单含 dk 三元组（§6.12 声明即必须；值经 `ontology_binding::resolve` 解析 code→ZUID
/// 静态绑定，未声明叶表按 §7.3.3 绑 NULL）；`_t_='flow-context'` 为登记标签
/// （§4.3.3 形态 3，不参与六对类系统）。
pub fn flow_context_insert_sql(domain: &str, table: &str) -> Option<&'static str> {
    if domain_of_leaf(table) != Some(domain) {
        return None;
    }
    match table {
        "zc_id_task-commission" => Some(
            r#"INSERT INTO isahl."zc_id_task-commission" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_task-design" => Some(
            r#"INSERT INTO isahl."zc_id_task-design" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_task-develop" => Some(
            r#"INSERT INTO isahl."zc_id_task-develop" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_task-fix" => Some(
            r#"INSERT INTO isahl."zc_id_task-fix" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_task-storage" => Some(
            r#"INSERT INTO isahl."zc_id_task-storage" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_task-testing" => Some(
            r#"INSERT INTO isahl."zc_id_task-testing" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_even-accident" => Some(
            r#"INSERT INTO isahl."zc_id_even-accident" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_even-alert" => Some(
            r#"INSERT INTO isahl."zc_id_even-alert" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_even-counting" => Some(
            r#"INSERT INTO isahl."zc_id_even-counting" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_even-issue" => Some(
            r#"INSERT INTO isahl."zc_id_even-issue" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_even-log" => Some(
            r#"INSERT INTO isahl."zc_id_even-log" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_even-modify" => Some(
            r#"INSERT INTO isahl."zc_id_even-modify" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_even-report" => Some(
            r#"INSERT INTO isahl."zc_id_even-report" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_even-tracking" => Some(
            r#"INSERT INTO isahl."zc_id_even-tracking" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-authorization" => Some(
            r#"INSERT INTO isahl."zc_id_appr-authorization" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-bid-evaluation" => Some(
            r#"INSERT INTO isahl."zc_id_appr-bid-evaluation" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-code-review" => Some(
            r#"INSERT INTO isahl."zc_id_appr-code-review" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-damage" => Some(
            r#"INSERT INTO isahl."zc_id_appr-damage" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-org-structure" => Some(
            r#"INSERT INTO isahl."zc_id_appr-org-structure" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-payment" => Some(
            r#"INSERT INTO isahl."zc_id_appr-payment" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-pricing" => Some(
            r#"INSERT INTO isahl."zc_id_appr-pricing" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-prj-initiation" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj-initiation" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-prj_doc-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj_doc-push" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-prj_made-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj_made-push" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-prj_request-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj_request-push" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-prj_sales-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj_sales-push" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-process" => Some(
            r#"INSERT INTO isahl."zc_id_appr-process" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-project-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-project-push" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-purchase" => Some(
            r#"INSERT INTO isahl."zc_id_appr-purchase" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-recruitment" => Some(
            r#"INSERT INTO isahl."zc_id_appr-recruitment" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-req-time_off" => Some(
            r#"INSERT INTO isahl."zc_id_appr-req-time_off" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        "zc_id_appr-user_verify" => Some(
            r#"INSERT INTO isahl."zc_id_appr-user_verify" (notice, _t_, created_by_id, dk_scene, dk_factor, dk_function) VALUES ($1, 'flow-context', $2, $3, $4, $5) RETURNING id"#,
        ),
        _ => None,
    }
}
