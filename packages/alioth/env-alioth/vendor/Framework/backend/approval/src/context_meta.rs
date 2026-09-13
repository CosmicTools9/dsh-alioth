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
//! - 桥接引用（junction-only，无 local_key）不落物理列：其参数以「目标属性投影」
//!   候选 `_refs.<引用名>.label` / `_refs.<引用名>.<属性>` 呈现（按取目标属性表达，
//!   非集合成员）；运行时由 approval::build_expr_ctx 经桥表解析目标行后注入
//!   `_refs.<引用名>`（点路径求值）。

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
/// lifecycle 叶表坐标三元组（code 形态）。未声明表返回 None —— 调用方绑 NULL
/// （BACKEND_FRAMEWORK §7.3.3「查不到 → NULL」降级；新增声明请补 scripts/generate-context-fields.ts
/// 的 TABLE_COORDS 并重跑生成器，禁手改本文件）。
pub fn leaf_coords(table: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match table {
        "zc_id_task-commission" => Some(("JE", "FMA", "↓_CH")),
        "zc_id_task-design" => Some(("JE", "FMA", "↓_CH")),
        "zc_id_task-develop" => Some(("JE", "FMA", "↓_CH")),
        "zc_id_task-fix" => Some(("JE", "FMA", "↓_CH")),
        "zc_id_task-storage" => Some(("JE", "FMA", "↓_CH")),
        "zc_id_task-testing" => Some(("JE", "FMA", "↓_CH")),
        "zc_id_even-accident" => Some(("JE", "FRA", "↓_EZ")),
        "zc_id_even-alert" => Some(("JE", "FBB", "↓_EE")),
        "zc_id_even-log" => Some(("JE", "FRE", "↓_GG")),
        "zc_id_even-modify" => Some(("JE", "FBB", "↓_EE")),
        "zc_id_even-report" => Some(("JE", "FBB", "↑_BD")),
        "zc_id_even-tracking" => Some(("TX", "FJA", "↓_GG")),
        "zc_id_appr-code-review" => Some(("JE", "FRE", "↓_GG")),
        "zc_id_appr-damage" => Some(("TX", "FRA", "↓_FF")),
        "zc_id_stat-inspection" => Some(("JE", "FBB", "↓_EN")),
        "zc_id_stat-maintenance" => Some(("JE", "FBB", "↓_FF")),
        "zc_id_stat-training" => Some(("JE", "FBA", "↓_EE")),
        "zc_id_stat-slf-voucher" => Some(("GC", "FJA", "↓.BE")),
        "zc_id_stat-tsp-voucher" => Some(("GC", "FJA", "↓_BE")),
        "zc_id_stat-whs-voucher" => Some(("JC", "GID", "↓_LA")),
        "zc_id_stat-smt-bank" => Some(("TX", "FJA", "↓_EV")),
        "zc_id_stat-smt-cash" => Some(("TX", "FJA", "↓_EV")),
        "zc_id_stat-smt-channel" => Some(("TX", "FJA", "↓_EV")),
        "zc_id_orde-land" => Some(("TX", "FJA", "↓_EV")),
        _ => None,
    }
}

pub fn statement_leaf_insert_sql(leaf: &str) -> Option<&'static str> {
    match leaf {
        "zc_id_stat-appeal" => Some(
            r#"INSERT INTO isahl."zc_id_stat-appeal" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_stat-inspection" => Some(
            r#"INSERT INTO isahl."zc_id_stat-inspection" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_stat-maintenance" => Some(
            r#"INSERT INTO isahl."zc_id_stat-maintenance" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_stat-bok-voucher" => Some(
            r#"INSERT INTO isahl."zc_id_stat-bok-voucher" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_stat-com-voucher" => Some(
            r#"INSERT INTO isahl."zc_id_stat-com-voucher" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_stat-slf-voucher" => Some(
            r#"INSERT INTO isahl."zc_id_stat-slf-voucher" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_stat-smt-bank" => Some(
            r#"INSERT INTO isahl."zc_id_stat-smt-bank" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_stat-smt-cash" => Some(
            r#"INSERT INTO isahl."zc_id_stat-smt-cash" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_stat-smt-channel" => Some(
            r#"INSERT INTO isahl."zc_id_stat-smt-channel" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_stat-tsp-voucher" => Some(
            r#"INSERT INTO isahl."zc_id_stat-tsp-voucher" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_stat-whs-voucher" => Some(
            r#"INSERT INTO isahl."zc_id_stat-whs-voucher" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_stat-tsk-requisition" => Some(
            r#"INSERT INTO isahl."zc_id_stat-tsk-requisition" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_orde-consult" => Some(
            r#"INSERT INTO isahl."zc_id_orde-consult" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_orde-retail" => Some(
            r#"INSERT INTO isahl."zc_id_orde-retail" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_orde-storage" => Some(
            r#"INSERT INTO isahl."zc_id_orde-storage" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_orde-ahbl" => Some(
            r#"INSERT INTO isahl."zc_id_orde-ahbl" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_orde-airlift" => Some(
            r#"INSERT INTO isahl."zc_id_orde-airlift" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_orde-hbl" => Some(
            r#"INSERT INTO isahl."zc_id_orde-hbl" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_orde-land" => Some(
            r#"INSERT INTO isahl."zc_id_orde-land" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_orde-lbl" => Some(
            r#"INSERT INTO isahl."zc_id_orde-lbl" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_orde-multimodal" => Some(
            r#"INSERT INTO isahl."zc_id_orde-multimodal" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_orde-railway" => Some(
            r#"INSERT INTO isahl."zc_id_orde-railway" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_orde-rbl" => Some(
            r#"INSERT INTO isahl."zc_id_orde-rbl" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_orde-shipping" => Some(
            r#"INSERT INTO isahl."zc_id_orde-shipping" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_stat-training" => Some(
            r#"INSERT INTO isahl."zc_id_stat-training" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_stat-volume" => Some(
            r#"INSERT INTO isahl."zc_id_stat-volume" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_stat-weight" => Some(
            r#"INSERT INTO isahl."zc_id_stat-weight" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        _ => None,
    }
}

pub fn task_leaf_insert_sql(leaf: &str) -> Option<&'static str> {
    match leaf {
        "zc_id_task-commission" => Some(
            r#"INSERT INTO isahl."zc_id_task-commission" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_task-design" => Some(
            r#"INSERT INTO isahl."zc_id_task-design" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_task-develop" => Some(
            r#"INSERT INTO isahl."zc_id_task-develop" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_task-fix" => Some(
            r#"INSERT INTO isahl."zc_id_task-fix" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_task-storage" => Some(
            r#"INSERT INTO isahl."zc_id_task-storage" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_task-testing" => Some(
            r#"INSERT INTO isahl."zc_id_task-testing" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        _ => None,
    }
}

pub fn event_leaf_insert_sql(leaf: &str) -> Option<&'static str> {
    match leaf {
        "zc_id_even-accident" => Some(
            r#"INSERT INTO isahl."zc_id_even-accident" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_even-alert" => Some(
            r#"INSERT INTO isahl."zc_id_even-alert" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-authorization" => Some(
            r#"INSERT INTO isahl."zc_id_appr-authorization" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-bid-evaluation" => Some(
            r#"INSERT INTO isahl."zc_id_appr-bid-evaluation" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-code-review" => Some(
            r#"INSERT INTO isahl."zc_id_appr-code-review" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-damage" => Some(
            r#"INSERT INTO isahl."zc_id_appr-damage" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-org-structure" => Some(
            r#"INSERT INTO isahl."zc_id_appr-org-structure" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-payment" => Some(
            r#"INSERT INTO isahl."zc_id_appr-payment" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-pricing" => Some(
            r#"INSERT INTO isahl."zc_id_appr-pricing" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-prj-initiation" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj-initiation" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-prj_doc-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj_doc-push" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-prj_made-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj_made-push" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-prj_request-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj_request-push" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-prj_sales-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-prj_sales-push" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-process" => Some(
            r#"INSERT INTO isahl."zc_id_appr-process" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-project-push" => Some(
            r#"INSERT INTO isahl."zc_id_appr-project-push" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-purchase" => Some(
            r#"INSERT INTO isahl."zc_id_appr-purchase" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-recruitment" => Some(
            r#"INSERT INTO isahl."zc_id_appr-recruitment" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-req-time_off" => Some(
            r#"INSERT INTO isahl."zc_id_appr-req-time_off" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_appr-user_verify" => Some(
            r#"INSERT INTO isahl."zc_id_appr-user_verify" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_even-counting" => Some(
            r#"INSERT INTO isahl."zc_id_even-counting" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_even-issue" => Some(
            r#"INSERT INTO isahl."zc_id_even-issue" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_even-log" => Some(
            r#"INSERT INTO isahl."zc_id_even-log" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_even-modify" => Some(
            r#"INSERT INTO isahl."zc_id_even-modify" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_even-report" => Some(
            r#"INSERT INTO isahl."zc_id_even-report" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
        ),
        "zc_id_even-tracking" => Some(
            r#"INSERT INTO isahl."zc_id_even-tracking" (notice, code, tpl_id, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id"#,
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.invoice.code",
                label: "发票内容·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.invoice.comments",
                label: "发票内容·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.invoice.label",
                label: "发票内容·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.invoice.t_color_",
                label: "发票内容·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.smt-voucher.code",
                label: "结算凭证·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.smt-voucher.comments",
                label: "结算凭证·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.smt-voucher.counting-type",
                label: "结算凭证·counting-type",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.smt-voucher.label",
                label: "结算凭证·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.smt-voucher.t_color_",
                label: "结算凭证·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.reason.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.reason.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.reason.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.reason.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.cnt-status.code",
                label: "盘点状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.cnt-status.comments",
                label: "盘点状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.cnt-status.enable",
                label: "盘点状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.cnt-status.label",
                label: "盘点状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.cnt-status.t_color_",
                label: "盘点状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "盘点物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "盘点物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "盘点物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "盘点物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "盘点物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "引用单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "引用单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "引用单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "引用单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.container.code",
                label: "容器·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.comments",
                label: "容器·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.container.label",
                label: "容器·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.container.t_color_",
                label: "容器·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.matter.code",
                label: "物项·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.comments",
                label: "物项·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.label",
                label: "物项·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.matter.p_number",
                label: "物项·p_number",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.matter.t_color_",
                label: "物项·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.standard.code",
                label: "标准·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.comments",
                label: "标准·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.standard.label",
                label: "标准·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.standard.t_color_",
                label: "标准·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.dependency.code",
                label: "前序依赖·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.dependency.comments",
                label: "前序依赖·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.dependency.label",
                label: "前序依赖·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.dependency.t_color_",
                label: "前序依赖·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.dependency.code",
                label: "前序依赖·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.dependency.comments",
                label: "前序依赖·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.dependency.label",
                label: "前序依赖·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.dependency.t_color_",
                label: "前序依赖·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.dependency.code",
                label: "前序依赖·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.dependency.comments",
                label: "前序依赖·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.dependency.label",
                label: "前序依赖·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.dependency.t_color_",
                label: "前序依赖·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.dependency.code",
                label: "前序依赖·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.dependency.comments",
                label: "前序依赖·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.dependency.label",
                label: "前序依赖·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.dependency.t_color_",
                label: "前序依赖·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.dependency.code",
                label: "前序依赖·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.dependency.comments",
                label: "前序依赖·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.dependency.label",
                label: "前序依赖·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.dependency.t_color_",
                label: "前序依赖·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
            ContextFieldMeta {
                name: "_refs.bill.code",
                label: "单据·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.comments",
                label: "单据·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.bill.label",
                label: "单据·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.bill.t_color_",
                label: "单据·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.dependency.code",
                label: "前序依赖·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.dependency.comments",
                label: "前序依赖·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.dependency.label",
                label: "前序依赖·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.dependency.t_color_",
                label: "前序依赖·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.statement.code",
                label: "缘由·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.comments",
                label: "缘由·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.statement.label",
                label: "缘由·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.statement.t_color_",
                label: "缘由·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "_refs.status.code",
                label: "状态·code",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.comments",
                label: "状态·comments",
                category: "reference",
                data_type: "text",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.enable",
                label: "状态·enable",
                category: "reference",
                data_type: "boolean",
                domain: "",
            },
            ContextFieldMeta {
                name: "_refs.status.label",
                label: "状态·notice",
                category: "reference",
                data_type: "text",
                domain: "lookup",
            },
            ContextFieldMeta {
                name: "_refs.status.t_color_",
                label: "状态·t_color_",
                category: "reference",
                data_type: "text",
                domain: "color",
            },
            ContextFieldMeta {
                name: "now",
                label: "系统时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "arrived_at",
                label: "抵达时间",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_st",
                label: "区间起",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
            },
            ContextFieldMeta {
                name: "period_ed",
                label: "区间止",
                category: "runtime",
                data_type: "timestamptz",
                domain: "",
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-authorization",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-authorization",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-authorization",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-authorization",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-authorization",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-code-review",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-code-review",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-code-review",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-code-review",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-code-review",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-damage",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-damage",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-damage",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-damage",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-damage",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-org-structure",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-org-structure",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-org-structure",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-org-structure",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-org-structure",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-payment",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-payment",
        "_refs.invoice.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_deta-invoice" e
JOIN isahl."zc_id_appr-payment_rr_invoice" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-payment",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-payment",
        "_refs.smt-voucher.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stat-smt-voucher" e
JOIN isahl."zc_id_appr-payment_rr_smt-voucher" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-payment",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-payment",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-payment",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-pricing",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-pricing",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-pricing",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-pricing",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-pricing",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-process",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-process",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-process",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-process",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-process",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-project-push",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-project-push",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-project-push",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-project-push",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-project-push",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-purchase",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-purchase",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-purchase",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-purchase",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-purchase",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-recruitment",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-recruitment",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-recruitment",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-recruitment",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-recruitment",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-user_verify",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-user_verify",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-user_verify",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-user_verify",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_appr-user_verify",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-accident",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-accident",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-accident",
        "_refs.reason.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_lifecycle" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-accident",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-accident",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-accident",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-alert",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-alert",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-alert",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-alert",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-alert",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-counting",
        "_refs.cnt-status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stus-counting" e
JOIN isahl."zc_id_counting_r_cnt-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-counting",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-counting",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-counting",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-counting",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-counting",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stus-event" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-issue",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-issue",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-issue",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-issue",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-issue",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stus-event" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-log",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-log",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-log",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-log",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-log",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-modify",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-modify",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-modify",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-modify",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-modify",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-report",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-report",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-report",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-report",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-report",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-tracking",
        "_refs.container.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_stor-container" e
JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-tracking",
        "_refs.matter.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_production" e
JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-tracking",
        "_refs.standard.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_standard" e
JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-tracking",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_even-tracking",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_task_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-commission",
        "_refs.dependency.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
JOIN isahl."zc_id_task_rr_dependency" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-commission",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_task_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-commission",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "fk_parent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_task_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-design",
        "_refs.dependency.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
JOIN isahl."zc_id_task_rr_dependency" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-design",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_task_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-design",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "fk_parent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_task_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-develop",
        "_refs.dependency.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
JOIN isahl."zc_id_task_rr_dependency" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-develop",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_task_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-develop",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "fk_parent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_task_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-fix",
        "_refs.dependency.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
JOIN isahl."zc_id_task_rr_dependency" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-fix",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_task_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-fix",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "fk_parent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_task_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-storage",
        "_refs.dependency.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
JOIN isahl."zc_id_task_rr_dependency" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-storage",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_task_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-storage",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "fk_parent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
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
        "_refs.bill.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_bill" e
JOIN isahl."zc_id_task_rr_bill" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-testing",
        "_refs.dependency.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
JOIN isahl."zc_id_task_rr_dependency" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-testing",
        "_refs.statement.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_statement" e
JOIN isahl."zc_id_task_rr_reason" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
    ),
    (
        "zc_id_task-testing",
        "_refs.status.label",
        r#"SELECT DISTINCT e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_status" e
JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
WHERE j.deleted_at IS NULL AND e.deleted_at IS NULL ORDER BY 1 LIMIT 200"#,
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
        "fk_parent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
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
        "fk_parent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
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
        "fk_parent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
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
        "fk_parent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
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
        "fk_parent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
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
        "fk_parent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
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
        "fk_parent",
        r#"SELECT e.id, e.notice AS label, e.t_color_ AS color
FROM isahl."zc_id_task" e
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

/// 桥接引用（junction-only）目标属性解析静态表：`(叶表, 引用名, SQL)`。
/// SQL 绑定 $1 = 实体行 id，返回 `{id,label,color,labels}`（目标行首行属性 +
/// 全部目标行业务名列表）；无行/解析失败 → NULL（降级为缺该项）。运行时注入
/// `_refs.<引用名>`，参数以 `<引用名>.label` / `<引用名>.<属性>` 形式引用
/// （2026-09-10 用户裁定：桥接引用按「取目标属性」表达）。
pub static CONTEXT_BRIDGE_REFS: &[(&str, &str, &str)] = &[
    (
        "zc_id_appr-authorization",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-authorization",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-authorization",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-authorization",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-authorization",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-authorization",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-bid-evaluation",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-code-review",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-code-review",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-code-review",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-code-review",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-code-review",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-code-review",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-damage",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-damage",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-damage",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-damage",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-damage",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-damage",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-org-structure",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-org-structure",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-org-structure",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-org-structure",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-org-structure",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-org-structure",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-payment",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-payment",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-payment",
        "invoice",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_deta-invoice" e2
                  JOIN isahl."zc_id_appr-payment_rr_invoice" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_deta-invoice" e
        JOIN isahl."zc_id_appr-payment_rr_invoice" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-payment",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-payment",
        "smt-voucher",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stat-smt-voucher" e2
                  JOIN isahl."zc_id_appr-payment_rr_smt-voucher" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stat-smt-voucher" e
        JOIN isahl."zc_id_appr-payment_rr_smt-voucher" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-payment",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-payment",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-payment",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-pricing",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-pricing",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-pricing",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-pricing",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-pricing",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-pricing",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj-initiation",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_doc-push",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_made-push",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_request-push",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-prj_sales-push",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-process",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-process",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-process",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-process",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-process",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-process",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-project-push",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-project-push",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-project-push",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-project-push",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-project-push",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-project-push",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-purchase",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-purchase",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-purchase",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-purchase",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-purchase",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-purchase",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-recruitment",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-recruitment",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-recruitment",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-recruitment",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-recruitment",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-recruitment",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-req-time_off",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-user_verify",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-user_verify",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-user_verify",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-user_verify",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-user_verify",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_appr-user_verify",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-accident",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-accident",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-accident",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-accident",
        "reason",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_lifecycle" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_lifecycle" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-accident",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-accident",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-accident",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-alert",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-alert",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-alert",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-alert",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-alert",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-alert",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-counting",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-counting",
        "cnt-status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stus-counting" e2
                  JOIN isahl."zc_id_counting_r_cnt-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stus-counting" e
        JOIN isahl."zc_id_counting_r_cnt-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-counting",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-counting",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-counting",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-counting",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-counting",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stus-event" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stus-event" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-issue",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-issue",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-issue",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-issue",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-issue",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-issue",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stus-event" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stus-event" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-log",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-log",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-log",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-log",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-log",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-log",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-modify",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-modify",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-modify",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-modify",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-modify",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-modify",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-report",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-report",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-report",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-report",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-report",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-report",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-tracking",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_event_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_event_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-tracking",
        "container",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_stor-container" e2
                  JOIN isahl."zc_id_event_rr_container" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_stor-container" e
        JOIN isahl."zc_id_event_rr_container" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-tracking",
        "matter",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_production" e2
                  JOIN isahl."zc_id_event_rr_matter" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_production" e
        JOIN isahl."zc_id_event_rr_matter" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-tracking",
        "standard",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_standard" e2
                  JOIN isahl."zc_id_event_rr_standard" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_standard" e
        JOIN isahl."zc_id_event_rr_standard" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-tracking",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_event_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_event_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_even-tracking",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-commission",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_task_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_task_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-commission",
        "dependency",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_task" e2
                  JOIN isahl."zc_id_task_rr_dependency" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_task" e
        JOIN isahl."zc_id_task_rr_dependency" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-commission",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_task_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_task_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-commission",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-design",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_task_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_task_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-design",
        "dependency",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_task" e2
                  JOIN isahl."zc_id_task_rr_dependency" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_task" e
        JOIN isahl."zc_id_task_rr_dependency" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-design",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_task_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_task_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-design",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-develop",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_task_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_task_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-develop",
        "dependency",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_task" e2
                  JOIN isahl."zc_id_task_rr_dependency" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_task" e
        JOIN isahl."zc_id_task_rr_dependency" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-develop",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_task_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_task_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-develop",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-fix",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_task_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_task_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-fix",
        "dependency",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_task" e2
                  JOIN isahl."zc_id_task_rr_dependency" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_task" e
        JOIN isahl."zc_id_task_rr_dependency" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-fix",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_task_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_task_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-fix",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-storage",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_task_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_task_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-storage",
        "dependency",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_task" e2
                  JOIN isahl."zc_id_task_rr_dependency" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_task" e
        JOIN isahl."zc_id_task_rr_dependency" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-storage",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_task_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_task_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-storage",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-testing",
        "bill",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_bill" e2
                  JOIN isahl."zc_id_task_rr_bill" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_bill" e
        JOIN isahl."zc_id_task_rr_bill" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-testing",
        "dependency",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_task" e2
                  JOIN isahl."zc_id_task_rr_dependency" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_task" e
        JOIN isahl."zc_id_task_rr_dependency" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-testing",
        "statement",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_statement" e2
                  JOIN isahl."zc_id_task_rr_reason" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_statement" e
        JOIN isahl."zc_id_task_rr_reason" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
    (
        "zc_id_task-testing",
        "status",
        r#"SELECT to_jsonb(x) FROM (SELECT e.id, e.notice AS label, e.t_color_ AS color,
               (SELECT jsonb_agg(y.lbl) FROM (SELECT e2.notice AS lbl
                  FROM isahl."zc_id_status" e2
                  JOIN isahl."zc_id_lifecycle_r_primary-status" j2 ON j2.ref_right = e2.id
                  WHERE j2.ref_left = $1 AND j2.deleted_at IS NULL AND e2.deleted_at IS NULL
                  ORDER BY e2.id) y) AS labels
        FROM isahl."zc_id_status" e
        JOIN isahl."zc_id_lifecycle_r_primary-status" j ON j.ref_right = e.id
        WHERE j.ref_left = $1 AND j.deleted_at IS NULL AND e.deleted_at IS NULL
        ORDER BY e.id LIMIT 1) x"#,
    ),
];

/// 操作运行时时间上下文 SQL（绑定 $1 = 操作行 id）：`now`=求值时刻、
/// `arrived_at`=该节点抵达时刻（qk_arrived → zc_id_scal-date.date，缺失回退
/// created_at）、`period_st/period_ed`=绑定区间起止（qk_period → zc_id_segm-date）。
pub const OPERATION_TIME_CTX_SQL: &str = r#"SELECT jsonb_build_object(
       'now', now(),
       'arrived_at', COALESCE(sd.date, o.created_at),
       'period_st', pd.date_st,
       'period_ed', pd.date_ed)
FROM isahl.zc_id_operation o
LEFT JOIN isahl."zc_id_scal-date" sd ON sd.id = o.qk_arrived
LEFT JOIN isahl."zc_id_segm-date" pd ON pd.id = o.qk_period
WHERE o.id = $1"#;
