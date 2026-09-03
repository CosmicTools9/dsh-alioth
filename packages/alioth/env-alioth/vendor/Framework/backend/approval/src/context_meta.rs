// AUTO-GENERATED from DB (pg_inherits + isahl_meta.meta_collections, 生成期) — DO NOT EDIT
// 生成: bun scripts/generate-context-fields.ts（模型升级后重跑；pre-commit
// check-context-fields.ts 漂移时自动重建并阻断提交）
//
//! 模型设计规则（operation 节点，2026-09-01 裁决）：
//! - 可入选上下文条件判断/数值计算参数的字段：物理类型族 MUST ∈
//!   {integer, boolean, text, numeric, enum}（ARRAY/jsonb/timestamptz/date 等不可入选）；
//! - [fk/lk/qk/ck/sk/ref/dk/ak/tk/tpl]_* 均为外键（含标量引用列、标签键引用列与
//!   模板引用列 tpl_id），禁止直接选入——引用值经 `_refs` 模式访问（CONTEXT_REFS
//!   静态表按行 id 解析目标行）；
//! - t_color_（text）为颜色徽章字段：可入选，domain=color（值即颜色值）。

/// scope 叶表项（编译期快照；concept = meta_collections.name，缺失为 None）
#[derive(Debug, Clone, Copy)]
pub struct ScopeItemMeta {
    pub table: &'static str,
    pub concept: Option<&'static str>,
}

/// scope 域（task / event / approve）
#[derive(Debug, Clone, Copy)]
pub struct ScopeDomainMeta {
    pub key: &'static str,
    pub items: &'static [ScopeItemMeta],
}

/// 流程分支（zc_id_process 叶表）
pub static SCOPE_BRANCHES: &[ScopeItemMeta] = &[
    ScopeItemMeta {
        table: "zc_id_proc-approve",
        concept: Some("流程-门禁审批"),
    },
    ScopeItemMeta {
        table: "zc_id_proc-cicd",
        concept: Some("流程-持续集成"),
    },
    ScopeItemMeta {
        table: "zc_id_proc-loading",
        concept: Some("流程-装载包装"),
    },
    ScopeItemMeta {
        table: "zc_id_proc-make",
        concept: Some("流程-生产制造"),
    },
    ScopeItemMeta {
        table: "zc_id_proc-project",
        concept: Some("流程-项目管理"),
    },
    ScopeItemMeta {
        table: "zc_id_proc-purchase",
        concept: Some("流程-采购管理"),
    },
    ScopeItemMeta {
        table: "zc_id_proc-service",
        concept: Some("流程-服务操作"),
    },
];

/// 终端节点语义实体真叶表（2026-08-29 裁决）：end 节点 statement 范例 /
/// task 驱动 start 范例的 INSERT 目标白名单（INSERT 落叶表铁律）
pub static STATEMENT_LEAVES: &[ScopeItemMeta] = &[
    ScopeItemMeta {
        table: "zc_id_stat-appeal",
        concept: Some("事实-售后申诉"),
    },
    ScopeItemMeta {
        table: "zc_id_stat-inspection",
        concept: Some("事实-质检单"),
    },
    ScopeItemMeta {
        table: "zc_id_stat-maintenance",
        concept: Some("事实-维修纪录"),
    },
    ScopeItemMeta {
        table: "zc_id_stat-bok-voucher",
        concept: Some("事实-借贷分录"),
    },
    ScopeItemMeta {
        table: "zc_id_stat-com-voucher",
        concept: Some("事实-贸易凭证"),
    },
    ScopeItemMeta {
        table: "zc_id_stat-slf-voucher",
        concept: Some("事实-上架凭证"),
    },
    ScopeItemMeta {
        table: "zc_id_stat-smt-bank",
        concept: Some("事实-银行流水"),
    },
    ScopeItemMeta {
        table: "zc_id_stat-smt-cash",
        concept: Some("事实-现金流水"),
    },
    ScopeItemMeta {
        table: "zc_id_stat-smt-channel",
        concept: Some("事实-渠道流水"),
    },
    ScopeItemMeta {
        table: "zc_id_stat-tsp-voucher",
        concept: Some("事实-承运凭证"),
    },
    ScopeItemMeta {
        table: "zc_id_stat-whs-voucher",
        concept: Some("事实-仓储凭证"),
    },
    ScopeItemMeta {
        table: "zc_id_stat-tsk-requisition",
        concept: Some("工单-领用"),
    },
    ScopeItemMeta {
        table: "zc_id_orde-consult",
        concept: Some("订单-咨询委托"),
    },
    ScopeItemMeta {
        table: "zc_id_orde-retail",
        concept: Some("订单-商品零售"),
    },
    ScopeItemMeta {
        table: "zc_id_orde-storage",
        concept: Some("订单-仓储服务"),
    },
    ScopeItemMeta {
        table: "zc_id_orde-ahbl",
        concept: Some("订单-空运代理"),
    },
    ScopeItemMeta {
        table: "zc_id_orde-airlift",
        concept: Some("订单-空运委托"),
    },
    ScopeItemMeta {
        table: "zc_id_orde-hbl",
        concept: Some("订单-海运代理"),
    },
    ScopeItemMeta {
        table: "zc_id_orde-land",
        concept: Some("订单-陆运委托"),
    },
    ScopeItemMeta {
        table: "zc_id_orde-lbl",
        concept: Some("订单-陆运代理"),
    },
    ScopeItemMeta {
        table: "zc_id_orde-multimodal",
        concept: Some("订单-多式联运"),
    },
    ScopeItemMeta {
        table: "zc_id_orde-railway",
        concept: Some("订单-铁路委托"),
    },
    ScopeItemMeta {
        table: "zc_id_orde-rbl",
        concept: Some("订单-铁运代理"),
    },
    ScopeItemMeta {
        table: "zc_id_orde-shipping",
        concept: Some("订单-海运委托"),
    },
    ScopeItemMeta {
        table: "zc_id_stat-training",
        concept: Some("事实-培训纪录"),
    },
    ScopeItemMeta {
        table: "zc_id_stat-volume",
        concept: Some("事实-量体单"),
    },
    ScopeItemMeta {
        table: "zc_id_stat-weight",
        concept: Some("事实-过磅单"),
    },
];

pub static TASK_LEAVES: &[ScopeItemMeta] = &[
    ScopeItemMeta {
        table: "zc_id_task-commission",
        concept: Some("任务-委托"),
    },
    ScopeItemMeta {
        table: "zc_id_task-design",
        concept: Some("任务-设计"),
    },
    ScopeItemMeta {
        table: "zc_id_task-develop",
        concept: Some("任务-研发"),
    },
    ScopeItemMeta {
        table: "zc_id_task-fix",
        concept: Some("任务-修复"),
    },
    ScopeItemMeta {
        table: "zc_id_task-storage",
        concept: Some("任务-储元"),
    },
    ScopeItemMeta {
        table: "zc_id_task-testing",
        concept: Some("任务-测试"),
    },
];

/// event 族全部真叶表（start 节点 event 驱动「具体事件」选项；2026-08-31 起含审批事件 appr-* 子树）
pub static EVENT_LEAVES: &[ScopeItemMeta] = &[
    ScopeItemMeta {
        table: "zc_id_even-accident",
        concept: Some("事件-事故"),
    },
    ScopeItemMeta {
        table: "zc_id_even-alert",
        concept: Some("事件-提示"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-authorization",
        concept: Some("审批-权限授予"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-bid-evaluation",
        concept: Some("审批-评标审定"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-code-review",
        concept: Some("审批-代码审查"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-damage",
        concept: Some("审批-损失申报"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-org-structure",
        concept: Some("审批-组织调整"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-payment",
        concept: Some("审批-付款申请"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-pricing",
        concept: Some("审批-价格调整"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-prj-initiation",
        concept: Some("审批-立项申请"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-prj_doc-push",
        concept: Some("审批-项目文档变更发布"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-prj_made-push",
        concept: Some("审批-项目内容变更发布"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-prj_request-push",
        concept: Some("审批-项目诉求变更发布"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-prj_sales-push",
        concept: Some("审批-项目需求变更发布"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-process",
        concept: Some("审批-处理流程"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-project-push",
        concept: Some("审批-项目基线变更发布"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-purchase",
        concept: Some("审批-采购申请"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-recruitment",
        concept: Some("审批-招聘申请"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-req-time_off",
        concept: Some("审批-休假申请"),
    },
    ScopeItemMeta {
        table: "zc_id_appr-user_verify",
        concept: Some("审批-用户认证"),
    },
    ScopeItemMeta {
        table: "zc_id_even-counting",
        concept: Some("事件-盘点"),
    },
    ScopeItemMeta {
        table: "zc_id_even-issue",
        concept: Some("事件-问题"),
    },
    ScopeItemMeta {
        table: "zc_id_even-log",
        concept: Some("事件-日志"),
    },
    ScopeItemMeta {
        table: "zc_id_even-modify",
        concept: Some("事件-变更"),
    },
    ScopeItemMeta {
        table: "zc_id_even-report",
        concept: Some("事件-汇报"),
    },
    ScopeItemMeta {
        table: "zc_id_even-tracking",
        concept: Some("事件-追踪"),
    },
];

/// 叶表是否 statement 真叶（end 范例 INSERT 白名单）
pub fn is_statement_leaf(table: &str) -> bool {
    STATEMENT_LEAVES.iter().any(|i| i.table == table)
}

/// 叶表是否 task 真叶（task 驱动 start 范例 INSERT 白名单）
pub fn is_task_leaf(table: &str) -> bool {
    TASK_LEAVES.iter().any(|i| i.table == table)
}

/// 终端节点语义实体 INSERT SQL 静态分发（sqlx 要求静态 str，禁 format! 动态表名）；
/// 范例行 tpl_id 传 NULL，实例行传范例 id（tpl_id 同表关联铁律）
pub fn statement_leaf_insert_sql(leaf: &str) -> Option<&'static str> {
    match leaf {
        "zc_id_stat-appeal" => Some(
            r#"INSERT INTO isahl."zc_id_stat-appeal" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_stat-inspection" => Some(
            r#"INSERT INTO isahl."zc_id_stat-inspection" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_stat-maintenance" => Some(
            r#"INSERT INTO isahl."zc_id_stat-maintenance" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_stat-bok-voucher" => Some(
            r#"INSERT INTO isahl."zc_id_stat-bok-voucher" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_stat-com-voucher" => Some(
            r#"INSERT INTO isahl."zc_id_stat-com-voucher" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_stat-slf-voucher" => Some(
            r#"INSERT INTO isahl."zc_id_stat-slf-voucher" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_stat-smt-bank" => Some(
            r#"INSERT INTO isahl."zc_id_stat-smt-bank" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_stat-smt-cash" => Some(
            r#"INSERT INTO isahl."zc_id_stat-smt-cash" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_stat-smt-channel" => Some(
            r#"INSERT INTO isahl."zc_id_stat-smt-channel" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_stat-tsp-voucher" => Some(
            r#"INSERT INTO isahl."zc_id_stat-tsp-voucher" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_stat-whs-voucher" => Some(
            r#"INSERT INTO isahl."zc_id_stat-whs-voucher" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_stat-tsk-requisition" => Some(
            r#"INSERT INTO isahl."zc_id_stat-tsk-requisition" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_orde-consult" => Some(
            r#"INSERT INTO isahl."zc_id_orde-consult" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_orde-retail" => Some(
            r#"INSERT INTO isahl."zc_id_orde-retail" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_orde-storage" => Some(
            r#"INSERT INTO isahl."zc_id_orde-storage" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_orde-ahbl" => Some(
            r#"INSERT INTO isahl."zc_id_orde-ahbl" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_orde-airlift" => Some(
            r#"INSERT INTO isahl."zc_id_orde-airlift" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_orde-hbl" => Some(
            r#"INSERT INTO isahl."zc_id_orde-hbl" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_orde-land" => Some(
            r#"INSERT INTO isahl."zc_id_orde-land" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_orde-lbl" => Some(
            r#"INSERT INTO isahl."zc_id_orde-lbl" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_orde-multimodal" => Some(
            r#"INSERT INTO isahl."zc_id_orde-multimodal" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_orde-railway" => Some(
            r#"INSERT INTO isahl."zc_id_orde-railway" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_orde-rbl" => Some(
            r#"INSERT INTO isahl."zc_id_orde-rbl" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_orde-shipping" => Some(
            r#"INSERT INTO isahl."zc_id_orde-shipping" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_stat-training" => Some(
            r#"INSERT INTO isahl."zc_id_stat-training" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_stat-volume" => Some(
            r#"INSERT INTO isahl."zc_id_stat-volume" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_stat-weight" => Some(
            r#"INSERT INTO isahl."zc_id_stat-weight" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        _ => None,
    }
}

pub fn task_leaf_insert_sql(leaf: &str) -> Option<&'static str> {
    match leaf {
        "zc_id_task-commission" => Some(
            r#"INSERT INTO isahl."zc_id_task-commission" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_task-design" => Some(
            r#"INSERT INTO isahl."zc_id_task-design" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_task-develop" => Some(
            r#"INSERT INTO isahl."zc_id_task-develop" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_task-fix" => Some(
            r#"INSERT INTO isahl."zc_id_task-fix" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_task-storage" => Some(
            r#"INSERT INTO isahl."zc_id_task-storage" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_task-testing" => Some(
            r#"INSERT INTO isahl."zc_id_task-testing" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        _ => None,
    }
}

pub fn event_leaf_insert_sql(leaf: &str) -> Option<&'static str> {
    match leaf {
        "zc_id_even-accident" => Some(
            r#"INSERT INTO isahl."zc_id_even-accident" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_even-alert" => Some(
            r#"INSERT INTO isahl."zc_id_even-alert" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-authorization" => Some(
            r#"INSERT INTO isahl."zc_id_appr-authorization" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-bid-evaluation" => Some(
            r#"INSERT INTO isahl."zc_id_appr-bid-evaluation" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-code-review" => Some(
            r#"INSERT INTO isahl."zc_id_appr-code-review" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-damage" => Some(
            r#"INSERT INTO isahl."zc_id_appr-damage" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-org-structure" => Some(
            r#"INSERT INTO isahl."zc_id_appr-org-structure" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-payment" => Some(
            r#"INSERT INTO isahl."zc_id_appr-payment" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-pricing" => Some(
            r#"INSERT INTO isahl."zc_id_appr-pricing" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-prj-initiation" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj-initiation" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-prj_doc-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj_doc-push" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-prj_made-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj_made-push" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-prj_request-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj_request-push" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-prj_sales-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj_sales-push" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-process" => Some(
            r#"INSERT INTO isahl."zc_id_appr-process" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-project-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-project-push" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-purchase" => Some(
            r#"INSERT INTO isahl."zc_id_appr-purchase" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-recruitment" => Some(
            r#"INSERT INTO isahl."zc_id_appr-recruitment" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-req-time_off" => Some(
            r#"INSERT INTO isahl."zc_id_appr-req-time_off" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_appr-user_verify" => Some(
            r#"INSERT INTO isahl."zc_id_appr-user_verify" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_even-counting" => Some(
            r#"INSERT INTO isahl."zc_id_even-counting" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_even-issue" => Some(
            r#"INSERT INTO isahl."zc_id_even-issue" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_even-log" => Some(
            r#"INSERT INTO isahl."zc_id_even-log" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_even-modify" => Some(
            r#"INSERT INTO isahl."zc_id_even-modify" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_even-report" => Some(
            r#"INSERT INTO isahl."zc_id_even-report" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        "zc_id_even-tracking" => Some(
            r#"INSERT INTO isahl."zc_id_even-tracking" (notice, code, tpl_id, created_by_id, _f_, _t_) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
        ),
        _ => None,
    }
}

/// 上下文三域（task / event / approve）
pub static SCOPE_DOMAINS: &[ScopeDomainMeta] = &[
    ScopeDomainMeta {
        key: "task",
        items: &[
            ScopeItemMeta {
                table: "zc_id_task-commission",
                concept: Some("任务-委托"),
            },
            ScopeItemMeta {
                table: "zc_id_task-design",
                concept: Some("任务-设计"),
            },
            ScopeItemMeta {
                table: "zc_id_task-develop",
                concept: Some("任务-研发"),
            },
            ScopeItemMeta {
                table: "zc_id_task-fix",
                concept: Some("任务-修复"),
            },
            ScopeItemMeta {
                table: "zc_id_task-storage",
                concept: Some("任务-储元"),
            },
            ScopeItemMeta {
                table: "zc_id_task-testing",
                concept: Some("任务-测试"),
            },
        ],
    },
    ScopeDomainMeta {
        key: "event",
        items: &[
            ScopeItemMeta {
                table: "zc_id_even-accident",
                concept: Some("事件-事故"),
            },
            ScopeItemMeta {
                table: "zc_id_even-alert",
                concept: Some("事件-提示"),
            },
            ScopeItemMeta {
                table: "zc_id_even-counting",
                concept: Some("事件-盘点"),
            },
            ScopeItemMeta {
                table: "zc_id_even-issue",
                concept: Some("事件-问题"),
            },
            ScopeItemMeta {
                table: "zc_id_even-log",
                concept: Some("事件-日志"),
            },
            ScopeItemMeta {
                table: "zc_id_even-modify",
                concept: Some("事件-变更"),
            },
            ScopeItemMeta {
                table: "zc_id_even-report",
                concept: Some("事件-汇报"),
            },
            ScopeItemMeta {
                table: "zc_id_even-tracking",
                concept: Some("事件-追踪"),
            },
        ],
    },
    ScopeDomainMeta {
        key: "approve",
        items: &[
            ScopeItemMeta {
                table: "zc_id_appr-authorization",
                concept: Some("审批-权限授予"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-bid-evaluation",
                concept: Some("审批-评标审定"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-code-review",
                concept: Some("审批-代码审查"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-damage",
                concept: Some("审批-损失申报"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-org-structure",
                concept: Some("审批-组织调整"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-payment",
                concept: Some("审批-付款申请"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-pricing",
                concept: Some("审批-价格调整"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-prj-initiation",
                concept: Some("审批-立项申请"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-prj_doc-push",
                concept: Some("审批-项目文档变更发布"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-prj_made-push",
                concept: Some("审批-项目内容变更发布"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-prj_request-push",
                concept: Some("审批-项目诉求变更发布"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-prj_sales-push",
                concept: Some("审批-项目需求变更发布"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-process",
                concept: Some("审批-处理流程"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-project-push",
                concept: Some("审批-项目基线变更发布"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-purchase",
                concept: Some("审批-采购申请"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-recruitment",
                concept: Some("审批-招聘申请"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-req-time_off",
                concept: Some("审批-休假申请"),
            },
            ScopeItemMeta {
                table: "zc_id_appr-user_verify",
                concept: Some("审批-用户认证"),
            },
        ],
    },
];

/// 上下文业务字段（编译期快照；全有物理列——junction-only/jsonb 已在生成期剔除）
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct ContextFieldMeta {
    /// 物理列名（条件 expr 引用名，含连字符如 act-group）
    pub name: &'static str,
    /// 业务展示名（生成期取自 meta title，缺失回退列名）
    pub label: &'static str,
    /// scalar / reference
    pub category: &'static str,
    /// 物理 DB 数据类型（bigint/text — RuleValueInput 输入形态依据）
    pub data_type: &'static str,
    /// 值域种类：""=标量/文本 | subject=主体成员徽章 | lookup=字典彩色徽章 |
    /// color=颜色值徽章（t_color_，值即颜色值）
    pub domain: &'static str,
}

/// 三域叶表 → 业务字段（叶表在册即白名单成员；字段可为空切片=该叶表未种子化）
pub static CONTEXT_FIELDS: &[(&str, &[ContextFieldMeta])] = &[
    (
        "zc_id_appr-authorization",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-bid-evaluation",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "model",
                label: "model",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-code-review",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-damage",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-org-structure",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "model",
                label: "model",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-payment",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-pricing",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "model",
                label: "model",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-prj-initiation",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "model",
                label: "model",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-prj_doc-push",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "model",
                label: "model",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-prj_made-push",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "model",
                label: "model",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-prj_request-push",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "model",
                label: "model",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-prj_sales-push",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "model",
                label: "model",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-process",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-project-push",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "model",
                label: "model",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-purchase",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "model",
                label: "model",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-recruitment",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "model",
                label: "model",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-req-time_off",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "model",
                label: "model",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_appr-user_verify",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_even-accident",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_even-alert",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_even-counting",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "summary",
                label: "summary",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_even-issue",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_even-log",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_even-modify",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_even-report",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_even-tracking",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_task-commission",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_task-design",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_task-develop",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_task-fix",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_task-storage",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
    (
        "zc_id_task-testing",
        &[
            ContextFieldMeta {
                name: "code",
                label: "code",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "comments",
                label: "comments",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "notice",
                label: "notice",
                category: "scalar",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "t_color_",
                label: "t_color_",
                category: "scalar",
                data_type: "text",
                domain: "color",
            },
        ],
    ),
];

/// 按叶表名查上下文字段；非三域叶表（白名单外）返回 None
pub fn context_fields_of(table: &str) -> Option<&'static [ContextFieldMeta]> {
    CONTEXT_FIELDS
        .iter()
        .find(|(t, _)| *t == table)
        .map(|(_, fields)| *fields)
}

/// 实体行加载 SQL（静态分发——sqlx 0.9 要求 &'static str，禁 format! 动态表名）。
pub fn entity_row_sql(leaf: &str) -> Option<&'static str> {
    match leaf {
        "zc_id_appr-authorization" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-authorization" t WHERE t.id = $1"#)
        }
        "zc_id_appr-bid-evaluation" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-bid-evaluation" t WHERE t.id = $1"#)
        }
        "zc_id_appr-code-review" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-code-review" t WHERE t.id = $1"#)
        }
        "zc_id_appr-damage" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-damage" t WHERE t.id = $1"#)
        }
        "zc_id_appr-org-structure" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-org-structure" t WHERE t.id = $1"#)
        }
        "zc_id_appr-payment" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-payment" t WHERE t.id = $1"#)
        }
        "zc_id_appr-pricing" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-pricing" t WHERE t.id = $1"#)
        }
        "zc_id_appr-prj-initiation" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-prj-initiation" t WHERE t.id = $1"#)
        }
        "zc_id_appr-prj_doc-push" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-prj_doc-push" t WHERE t.id = $1"#)
        }
        "zc_id_appr-prj_made-push" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-prj_made-push" t WHERE t.id = $1"#)
        }
        "zc_id_appr-prj_request-push" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-prj_request-push" t WHERE t.id = $1"#)
        }
        "zc_id_appr-prj_sales-push" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-prj_sales-push" t WHERE t.id = $1"#)
        }
        "zc_id_appr-process" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-process" t WHERE t.id = $1"#)
        }
        "zc_id_appr-project-push" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-project-push" t WHERE t.id = $1"#)
        }
        "zc_id_appr-purchase" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-purchase" t WHERE t.id = $1"#)
        }
        "zc_id_appr-recruitment" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-recruitment" t WHERE t.id = $1"#)
        }
        "zc_id_appr-req-time_off" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-req-time_off" t WHERE t.id = $1"#)
        }
        "zc_id_appr-user_verify" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_appr-user_verify" t WHERE t.id = $1"#)
        }
        "zc_id_even-accident" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_even-accident" t WHERE t.id = $1"#)
        }
        "zc_id_even-alert" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_even-alert" t WHERE t.id = $1"#)
        }
        "zc_id_even-counting" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_even-counting" t WHERE t.id = $1"#)
        }
        "zc_id_even-issue" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_even-issue" t WHERE t.id = $1"#)
        }
        "zc_id_even-log" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_even-log" t WHERE t.id = $1"#)
        }
        "zc_id_even-modify" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_even-modify" t WHERE t.id = $1"#)
        }
        "zc_id_even-report" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_even-report" t WHERE t.id = $1"#)
        }
        "zc_id_even-tracking" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_even-tracking" t WHERE t.id = $1"#)
        }
        "zc_id_task-commission" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_task-commission" t WHERE t.id = $1"#)
        }
        "zc_id_task-design" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_task-design" t WHERE t.id = $1"#)
        }
        "zc_id_task-develop" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_task-develop" t WHERE t.id = $1"#)
        }
        "zc_id_task-fix" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_task-fix" t WHERE t.id = $1"#)
        }
        "zc_id_task-storage" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_task-storage" t WHERE t.id = $1"#)
        }
        "zc_id_task-testing" => {
            Some(r#"SELECT to_jsonb(t) FROM isahl."zc_id_task-testing" t WHERE t.id = $1"#)
        }
        _ => None,
    }
}

/// 值域候选查询静态表（可视化值编辑器数据源；subject join auth_users 取姓名）
pub static DOMAIN_SQL: &[(&str, &str, &str)] = &[
    (
        "zc_id_appr-authorization",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-authorization",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-authorization",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-authorization",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-authorization",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-code-review",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-code-review",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-code-review",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-code-review",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-code-review",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-damage",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-damage",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-damage",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-damage",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-damage",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-org-structure",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-org-structure",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-org-structure",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-org-structure",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-org-structure",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-payment",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-payment",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-payment",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-payment",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-payment",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-pricing",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-pricing",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-pricing",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-pricing",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-pricing",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-process",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-process",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-process",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-process",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-process",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-project-push",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-project-push",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-project-push",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-project-push",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-project-push",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-purchase",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-purchase",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-purchase",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-purchase",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-purchase",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-recruitment",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-recruitment",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-recruitment",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-recruitment",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-recruitment",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-user_verify",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-user_verify",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-user_verify",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-user_verify",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_appr-user_verify",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-accident",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-accident",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-accident",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-accident",
        "lk_risk",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-risk" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-accident",
        "lk_severity",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-severity" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-accident",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-accident",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-alert",
        "ck_category",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-alert" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-alert",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-alert",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-alert",
        "lk_applicable",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-applicable" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-alert",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-alert",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-alert",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-counting",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-counting",
        "fk_storage",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_storage" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-counting",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-counting",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-counting",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-counting",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-issue",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-issue",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-issue",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-issue",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-issue",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-log",
        "ck_category",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-log" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-log",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-log",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-log",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-log",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-log",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-modify",
        "ck_category",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-modify" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-modify",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-modify",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-modify",
        "fk_ver-fork",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_version" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-modify",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-modify",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-modify",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-report",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-report",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-report",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-report",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-report",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-tracking",
        "ck_category",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-tracking" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-tracking",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-tracking",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-tracking",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-tracking",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_even-tracking",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-commission",
        "ck_branch",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-ver_branch" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-commission",
        "fk_previous",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_version" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-commission",
        "tk_batch_no",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-batch" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-commission",
        "tk_version",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-version" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-design",
        "ck_branch",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-ver_branch" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-design",
        "fk_previous",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_version" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-design",
        "tk_batch_no",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-batch" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-design",
        "tk_version",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-version" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-develop",
        "ck_branch",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-ver_branch" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-develop",
        "fk_previous",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_version" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-develop",
        "tk_batch_no",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-batch" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-develop",
        "tk_version",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-version" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-fix",
        "ck_branch",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-ver_branch" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-fix",
        "fk_previous",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_version" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-fix",
        "tk_batch_no",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-batch" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-fix",
        "tk_version",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-version" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-storage",
        "ck_branch",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-ver_branch" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-storage",
        "fk_previous",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_version" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-storage",
        "tk_batch_no",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-batch" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-storage",
        "tk_version",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-version" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-testing",
        "ck_branch",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-ver_branch" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-testing",
        "fk_previous",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_version" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-testing",
        "tk_batch_no",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-batch" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
    (
        "zc_id_task-testing",
        "tk_version",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-version" e
WHERE e.deleted_at IS NULL ORDER BY 2 LIMIT 200"#,
    ),
];

/// 字段值域候选查询 SQL 静态分发；（叶表, 物理列）无值域 → None
pub fn domain_sql(leaf: &str, column: &str) -> Option<&'static str> {
    DOMAIN_SQL
        .iter()
        .find(|(l, c, _)| *l == leaf && *c == column)
        .map(|(_, _, sql)| *sql)
}

/// 引用字段 `_refs` 解析静态表（模型设计规则：外键列不直接选入条件/计算，
/// 引用值经 `_refs` 模式访问——表达式求值上下文按本表解析目标行 id/label/color）
pub static CONTEXT_REFS: &[(&str, &str, &str)] = &[
    (
        "zc_id_appr-authorization",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-authorization",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-authorization",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-authorization",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-authorization",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-code-review",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-code-review",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-code-review",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-code-review",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-code-review",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-damage",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-damage",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-damage",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-damage",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-damage",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-org-structure",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-org-structure",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-org-structure",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-org-structure",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-org-structure",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-payment",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-payment",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-payment",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-payment",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-payment",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-pricing",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-pricing",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-pricing",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-pricing",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-pricing",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-process",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-process",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-process",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-process",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-process",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-project-push",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-project-push",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-project-push",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-project-push",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-project-push",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-purchase",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-purchase",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-purchase",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-purchase",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-purchase",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-recruitment",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-recruitment",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-recruitment",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-recruitment",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-recruitment",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-user_verify",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-user_verify",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-user_verify",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-user_verify",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_appr-user_verify",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-accident",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-accident",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-accident",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-accident",
        "lk_risk",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-risk" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-accident",
        "lk_severity",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-severity" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-accident",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-accident",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-alert",
        "ck_category",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-alert" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-alert",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-alert",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-alert",
        "lk_applicable",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-applicable" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-alert",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-alert",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-alert",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-counting",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-counting",
        "fk_storage",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_storage" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-counting",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-counting",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-counting",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-counting",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-issue",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-issue",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-issue",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-issue",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-issue",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-log",
        "ck_category",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-log" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-log",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-log",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-log",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-log",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-log",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-modify",
        "ck_category",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-modify" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-modify",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-modify",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-modify",
        "fk_ver-fork",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_version" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-modify",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-modify",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-modify",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-report",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-report",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-report",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-report",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-report",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-tracking",
        "ck_category",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-tracking" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-tracking",
        "fk_place",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_place" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-tracking",
        "fk_subject",
        r#"SELECT e.id, COALESCE(u.name, e.notice) AS label, e.t_color_ AS color
FROM isahl."zc_id_subjects" e
               LEFT JOIN isahl_auth.auth_users u ON u.id = e.fk_user
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-tracking",
        "lk_health",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-health" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-tracking",
        "lk_urgent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_leve-urgent" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_even-tracking",
        "qk_date",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_scal-date" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-commission",
        "ck_branch",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-ver_branch" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-commission",
        "fk_previous",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_version" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-commission",
        "tk_batch_no",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-batch" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-commission",
        "tk_version",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-version" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-design",
        "ck_branch",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-ver_branch" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-design",
        "fk_previous",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_version" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-design",
        "tk_batch_no",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-batch" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-design",
        "tk_version",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-version" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-develop",
        "ck_branch",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-ver_branch" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-develop",
        "fk_previous",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_version" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-develop",
        "tk_batch_no",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-batch" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-develop",
        "tk_version",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-version" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-fix",
        "ck_branch",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-ver_branch" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-fix",
        "fk_previous",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_version" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-fix",
        "tk_batch_no",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-batch" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-fix",
        "tk_version",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-version" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-storage",
        "ck_branch",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-ver_branch" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-storage",
        "fk_previous",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_version" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-storage",
        "tk_batch_no",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-batch" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-storage",
        "tk_version",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-version" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-testing",
        "ck_branch",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_cate-ver_branch" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-testing",
        "fk_previous",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_version" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-testing",
        "tk_batch_no",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-batch" e
WHERE e.id = $1"#,
    ),
    (
        "zc_id_task-testing",
        "tk_version",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_tags-version" e
WHERE e.id = $1"#,
    ),
];

/// 引用行解析 SQL 静态分发；（叶表, 物理列）无引用 → None
pub fn refs_sql(leaf: &str, column: &str) -> Option<&'static str> {
    CONTEXT_REFS
        .iter()
        .find(|(l, c, _)| *l == leaf && *c == column)
        .map(|(_, _, sql)| *sql)
}
