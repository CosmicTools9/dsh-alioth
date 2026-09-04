//! OA 展示名解析层（change `add-ngac-oa-display-name`，NGAC_SPEC §2.2）。
//!
//! `o_name` 是 NGAC 图技术标识（UNIQUE、被 association/prohibition 引用），
//! `resource_identifier` 语义限定实例级业务标识——两者都不复用。`display_name`
//! 为读取侧派生字段（不落库、决策路径零影响），解析链（按序取首个非空）：
//!
//! 1. 实例级（`fk_resource != 0`）且 `resource_identifier` 非空 → `resource_identifier`
//! 2. `resource_type` 命中 `isahl_meta.meta_collections.table_name` → 该行 `name`
//!    （一次 `WHERE table_name = ANY($1)` 批量查询，禁止 N+1）
//! 3. `resource_type` 命中内置资源域映射（`BUILTIN_RESOURCE_TYPE_DISPLAY`）
//! 4. fallback → `o_name`
//!
//! `meta_collections` 查询失败（权限/连接）时静默降级至链 3/4（`log::warn`，
//! 管理面不报错）——best-effort，展示名非授权语义。

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use sqlx::PgPool;

/// 内置资源域映射（链 3）：`resource_type →` 人可读展示名。
///
/// 覆盖两层来源（SSO 常连独立 auth 库、`isahl_meta` 不可达，故固化于此）：
/// 1. 系统/跨域资源（sso_admin 等）——简短业务名；
/// 2. `ngac-contract` `resource_registry` 全量资源类型（`type_name → zc_id 表
///    → meta_collections.name`，2026-08 从 dev 库导出，本体表中文名）+ registry
///    未注册但存量的 WZ 域类型（contacts/departments/positions/receipts/
///    waybill/sap_classes/outgo_payment_payees）与元数据类型。
///
/// `builtin_display` 查询前做连字符归一（`-` → `_`，兼容 outgo-payments 等
/// 历史双命名 seed）。新类型未收录时由链 4 fallback 兜底。
pub const BUILTIN_RESOURCE_TYPE_DISPLAY: &[(&str, &str)] = &[
    // ── 系统域（简短业务名，发布语义保留） ──
    ("sso_admin", "SSO 管理"),
    ("sso_audit", "SSO 审计"),
    ("system_config", "系统配置"),
    ("module", "模块"),
    ("configs", "OpenAPI 配置"),
    ("sales", "OpenAPI 销售"),
    ("purchases", "OpenAPI 购买"),
    ("mades", "OpenAPI 制造"),
    ("openapi_admin", "OpenAPI 管理"),
    ("openapi_analytics", "OpenAPI 分析"),
    ("approvals", "审批"),
    ("approval_flows", "审批流"),
    ("approval_instances", "审批实例"),
    ("approval_actions", "审批操作"),
    ("delegation_rules", "委托规则"),
    // ── registry 全量 + 存量补充（本体表中文名，按 resource_type 排序） ──
    ("aeg_reviews", "事件-变更"),
    ("airworthiness_certificates", "销售-适航证书"),
    ("airworthiness_directives", "事件-变更"),
    ("amendments", "关联-合约↔协定"),
    ("approval_roles", "实现-类目"),
    ("attachments", "文件-档案"),
    ("bill_checks", "单据-清算账单"),
    ("boms", "实现-BOM"),
    ("ccb_members", "操作-核验审批"),
    ("certification_bases", "实现-计划"),
    ("certification_plans", "实现-计划"),
    ("change_requests", "事件-变更"),
    ("cm_gates", "操作-验证门禁"),
    ("collections", "元数据集合"),
    ("compliance_records", "事件-汇报"),
    ("consignments", "订单-陆运委托"),
    ("contacts", "实现-联系人"),
    ("continued_airworthiness_files", "产品-诉求"),
    ("contract_agreements", "关联-合约↔协定"),
    ("contract_matters", "关联-合约↔标的"),
    ("contract_partys", "关联-合约↔参与方"),
    ("contracts", "实现-合约"),
    ("counterparties", "主体-组织"),
    ("damage", "事件-事故"),
    ("damage_reports", "事件-汇报"),
    ("dashboard", "实现-合约"),
    ("defects", "事件-汇报"),
    ("departments", "组织-部门"),
    ("design_change_impacts", "事件-变更"),
    ("design_manuals", "产品-诉求"),
    ("dispatch", "操作-核验审批"),
    ("engineers", "主体-雇员"),
    ("event_accidents", "事件-事故"),
    ("event_trackings", "事件-跟踪"),
    ("exchange_rates", "比率-汇率"),
    ("execution", "订单-陆运委托"),
    ("factors", "实现-要素"),
    ("fields", "元数据字段"),
    ("files", "文件-档案"),
    ("fleet", "销售-公路运输"),
    ("flow_nodes", "事件-审批"),
    ("flows", "实现-流程"),
    ("functions", "实现-职能"),
    ("gantt_items", "实现-计划"),
    ("gate_audit_logs", "事件-审批"),
    ("gate_projects", "实现-项目"),
    ("gate_templates", "实现-流程"),
    ("global", "操作-核验审批"),
    ("identities", "实现-身份"),
    ("incoming_inspections", "事实-质检单"),
    ("inspection_batches", "事实-质检单"),
    ("investment_monthlies", "评估-定量计算"),
    ("invoice_applications", "实现-发票"),
    ("invoices_out", "实现-发票"),
    ("items", "关联-合约↔标的"),
    ("license_changes", "实现-合约"),
    ("life_limited_parts", "产品-诉求"),
    ("maintenance_plans", "计划-维护"),
    ("maintenance_records", "事实-质检单"),
    ("materials", "制造-加工物料"),
    ("measurements", "实现-单位"),
    ("measurement_units", "实现-单位"),
    ("ncrs", "事件-事故"),
    ("outgo_bills", "单据-清算账单"),
    ("outgo_invoices_in", "实现-发票"),
    ("outgo_payables", "操作-付款"),
    ("outgo_payment_matches", "单据-清算账单"),
    ("outgo_payment_payees", "付款收款方"),
    ("outgo_payments", "操作-付款"),
    ("outgo_waybills", "订单-陆运委托"),
    ("outgo_yecai_payables", "单据-清算账单"),
    ("party", "关联-合约↔参与方"),
    ("payment_plans", "计划-付款"),
    ("places", "实现-场所"),
    ("plans", "实现-计划"),
    ("positions", "主体-岗位"),
    ("post_certification_audits", "事件-审批"),
    ("production_certifications", "销售-许可执照"),
    ("production_schedules", "实现-计划"),
    ("products", "实现-产品"),
    ("project_budgets", "评估-定量计算"),
    ("projects", "项目-过程控制"),
    ("project_templates", "实现-项目"),
    ("quad_analyses", "评估-定量计算"),
    ("reassign", "操作-核验审批"),
    ("receipt_collections", "单据-清算账单"),
    ("receipt_matches", "单据-清算账单"),
    ("receipts", "收款单"),
    ("receivables", "单据-清算账单"),
    ("regulatory_assessments", "事件-变更"),
    ("release_tags", "产品-诉求"),
    ("requirements", "产品-诉求"),
    ("risk_items", "事件-事故"),
    ("safety_events", "事件-事故"),
    ("sap_classes", "SAP 分类"),
    ("scalar_prices", "计量-价格"),
    ("scenes", "实现-场景"),
    ("schedule", "实现-计划"),
    ("settlement_orders", "事实-结算凭证"),
    ("signing", "关联-合约↔协定"),
    ("skill_tags", "标签-技能"),
    ("status", "状态-合约"),
    ("structural_repairs", "事件-变更"),
    ("subjects", "主体-组织"),
    ("subsystems", "产品-制造"),
    ("supplier_change_notices", "事件-变更"),
    ("tasks", "实现-任务"),
    ("templates", "实现-合约"),
    ("test_runs", "事实-质检单"),
    ("tracking", "操作-运输追踪"),
    ("training_records", "事实-培训纪录"),
    ("transport_tracking", "操作-运输追踪"),
    ("type_certifications", "销售-型号证书"),
    ("vehicles", "容器-车辆"),
    ("ver_branch", "类目-版控分支"),
    ("verctrl", "文件-受控文件"),
    ("versions", "标签-版本"),
    ("waybill", "运单"),
    ("waybills", "订单-运输服务"),
];

/// 链 2：批量解析 `resource_type → meta_collections.name`。
///
/// 单条 `WHERE table_name = ANY($1)` 查询覆盖全部入参类型；查询失败
/// （权限/连接）`log::warn` 后返回空 map（静默降级，不 panic 不报错）。
pub async fn meta_display_names(
    pool: &PgPool,
    resource_types: &HashSet<String>,
) -> HashMap<String, String> {
    if resource_types.is_empty() {
        return HashMap::new();
    }
    let types: Vec<&str> = resource_types.iter().map(String::as_str).collect();
    let rows: Result<Vec<(String, String)>, _> = sqlx::query_as(
        "SELECT table_name, name FROM isahl_meta.meta_collections WHERE table_name = ANY($1)",
    )
    .bind(&types)
    .fetch_all(pool)
    .await;
    match rows {
        Ok(rows) => rows.into_iter().collect(),
        Err(e) => {
            log::warn!(
                "meta_display_names: meta_collections 查询失败，降级至内置映射/fallback: {}",
                e
            );
            HashMap::new()
        }
    }
}

/// 链 3：内置资源域映射命中。查询前做连字符归一（`-` → `_`）——
/// 存量 seed 存在 `outgo-payments` 与 `outgo_payments` 双命名，归一后同映射。
pub fn builtin_display(resource_type: &str) -> Option<&'static str> {
    let normalized = resource_type.replace('-', "_");
    BUILTIN_RESOURCE_TYPE_DISPLAY
        .iter()
        .find(|(k, _)| *k == normalized || *k == resource_type)
        .map(|(_, v)| *v)
}

/// OA 展示名完整解析链（D2，按序取首个非空）。
///
/// - 实例级 identifier 优先（与 §2.2.1 实例级语义衔接：collection 级 OA 的
///   `resource_identifier` 为空或无信息量，自然落入链 2/3/4）
/// - `meta_names`：链 2 结果（可为空 map——已降级）
/// - 保证非空：链 1-3 全 miss 时返回 `o_name`
pub fn resolve_display_name(
    fk_resource: Option<i64>,
    resource_identifier: Option<&str>,
    resource_type: &str,
    meta_names: &HashMap<String, String>,
    o_name: &str,
) -> String {
    // 链 1：实例级业务标识
    if fk_resource.unwrap_or(0) != 0 {
        if let Some(id) = resource_identifier.filter(|s| !s.is_empty()) {
            return id.to_string();
        }
    }
    // 链 2：meta_collections.name
    if let Some(name) = meta_names.get(resource_type) {
        if !name.is_empty() {
            return name.clone();
        }
    }
    // 链 3：内置资源域映射
    if let Some(name) = builtin_display(resource_type) {
        return name.to_string();
    }
    // 链 4：fallback o_name
    o_name.to_string()
}

/// 组头/徽章用的资源域展示名：链 2 → 链 3 → 原 `resource_type` 值。
pub fn resolve_resource_type_display(
    resource_type: &str,
    meta_names: &HashMap<String, String>,
) -> String {
    if let Some(name) = meta_names.get(resource_type) {
        if !name.is_empty() {
            return name.clone();
        }
    }
    builtin_display(resource_type)
        .map(String::from)
        .unwrap_or_else(|| resource_type.to_string())
}

/// 资源类型的前端模块归属（add-ngac-oa-module-observability）。
///
/// 由 `scripts/ts/generate-ngac-resource-module-map.ts` 扫描
/// `Pre-Proc/*/Sources/Modules/*/frontend/src` 对 registry type_name 的引用生成；
/// 共享资源归属首个命中模块（主使用方）。系统域类型无前端引用 → 端点输出 null。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceModuleRef {
    pub namespace: &'static str,
    pub module_id: &'static str,
    pub module_name: &'static str,
    /// Gateway 模块路由前缀（如 `/outgo-wz`），前端跳转用
    pub module_route: &'static str,
}

/** 模块映射行：(资源类型, (命名空间, 模块码, 显示名, 路由前缀)) */
pub type ResourceModuleEntry = (
    &'static str,
    (&'static str, &'static str, &'static str, &'static str),
);

pub const RESOURCE_MODULE_MAP: &[ResourceModuleEntry] = &[
    (
        "aeg_reviews",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "airworthiness_certificates",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "airworthiness_directives",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "amendments",
        ("WZ", "contract-wz", "合同管理", "/contract-wz"),
    ),
    // approval_actions 无前端字面引用（与 flows/instances 同源，Framework approval），
    // 生成器不产出——手工归并多 ns（2026-08-21 跨 ns 标签修复）
    (
        "approval_actions",
        (
            "AVIC-CAASEC",
            "system-approve",
            "审批流程",
            "/system-approve",
        ),
    ),
    (
        "approval_actions",
        ("WZ", "comprehensive-wz", "综合管理", "/comprehensive-wz"),
    ),
    (
        "approval_actions",
        (
            "Cosmic-Tools",
            "system-approve",
            "审批流管理",
            "/system-approve",
        ),
    ),
    (
        "approval_actions",
        ("Alioth", "system-approve", "审批流管理", "/system-approve"),
    ),
    (
        "approval_flows",
        (
            "AVIC-CAASEC",
            "system-approve",
            "审批流程",
            "/system-approve",
        ),
    ),
    (
        "approval_flows",
        ("WZ", "comprehensive-wz", "综合管理", "/comprehensive-wz"),
    ),
    (
        "approval_flows",
        ("Alioth", "system-approve", "审批流管理", "/system-approve"),
    ),
    (
        "approval_instances",
        (
            "AVIC-CAASEC",
            "system-approve",
            "审批流程",
            "/system-approve",
        ),
    ),
    (
        "approval_instances",
        (
            "Cosmic-Tools",
            "system-approve",
            "审批流管理",
            "/system-approve",
        ),
    ),
    (
        "approval_instances",
        ("WZ", "comprehensive-wz", "综合管理", "/comprehensive-wz"),
    ),
    (
        "approval_instances",
        ("Alioth", "system-approve", "审批流管理", "/system-approve"),
    ),
    (
        "approval_roles",
        (
            "AVIC-CAASEC",
            "system-approve",
            "审批流程",
            "/system-approve",
        ),
    ),
    (
        "approval_roles",
        (
            "Cosmic-Tools",
            "system-approve",
            "审批流管理",
            "/system-approve",
        ),
    ),
    (
        "approval_roles",
        ("Alioth", "system-approve", "审批流管理", "/system-approve"),
    ),
    (
        "approvals",
        (
            "AVIC-CAASEC",
            "system-approve",
            "审批流程",
            "/system-approve",
        ),
    ),
    (
        "approvals",
        (
            "Cosmic-Tools",
            "system-approve",
            "审批流管理",
            "/system-approve",
        ),
    ),
    // WZ 审批：前端经 approval-flows/instances API 使用，无 approvals 字面量，
    // 与 flows/instances 同源（Framework approval，comprehensive-wz 审批页）——手工归并
    (
        "approvals",
        ("WZ", "comprehensive-wz", "综合管理", "/comprehensive-wz"),
    ),
    (
        "approvals",
        ("Alioth", "system-approve", "审批流管理", "/system-approve"),
    ),
    (
        "attachments",
        ("WZ", "contract-wz", "合同管理", "/contract-wz"),
    ),
    ("bill_checks", ("WZ", "income-wz", "收入往来", "/income-wz")),
    (
        "certification_bases",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "certification_plans",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "change_requests",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "cm_gates",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "compliance_records",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "consignments",
        ("WZ", "transport-wz", "承运商运营", "/transport-wz"),
    ),
    (
        "contacts",
        ("WZ", "comprehensive-wz", "综合管理", "/comprehensive-wz"),
    ),
    (
        "continued_airworthiness_files",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "contracts",
        ("WZ", "comprehensive-wz", "综合管理", "/comprehensive-wz"),
    ),
    (
        "counterparties",
        ("WZ", "comprehensive-wz", "综合管理", "/comprehensive-wz"),
    ),
    (
        "damage",
        ("WZ", "transport-wz", "承运商运营", "/transport-wz"),
    ),
    (
        "dashboard",
        ("WZ", "contract-wz", "合同管理", "/contract-wz"),
    ),
    (
        "defects",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "delegation_rules",
        (
            "AVIC-CAASEC",
            "system-approve",
            "审批流程",
            "/system-approve",
        ),
    ),
    (
        "delegation_rules",
        (
            "Cosmic-Tools",
            "system-approve",
            "审批流管理",
            "/system-approve",
        ),
    ),
    // WZ 审批委托：与 approvals 同源（comprehensive-wz 审批模块），无字面引用——手工归并
    (
        "delegation_rules",
        ("WZ", "comprehensive-wz", "综合管理", "/comprehensive-wz"),
    ),
    (
        "delegation_rules",
        ("Alioth", "system-approve", "审批流管理", "/system-approve"),
    ),
    (
        "departments",
        ("WZ", "comprehensive-wz", "综合管理", "/comprehensive-wz"),
    ),
    (
        "design_change_impacts",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "design_manuals",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "dispatch",
        ("WZ", "transport-wz", "承运商运营", "/transport-wz"),
    ),
    (
        "engineers",
        (
            "AVIC-CAASEC",
            "system-approve",
            "审批流程",
            "/system-approve",
        ),
    ),
    (
        "exchange_rates",
        (
            "Cosmic-Tools",
            "system-settings",
            "系统设置",
            "/system-settings",
        ),
    ),
    (
        "exchange_rates",
        ("Alioth", "system-settings", "系统设置", "/system-settings"),
    ),
    (
        "execution",
        ("WZ", "contract-wz", "合同管理", "/contract-wz"),
    ),
    (
        "fields",
        (
            "AVIC-CAASEC",
            "system-approve",
            "审批流程",
            "/system-approve",
        ),
    ),
    (
        "fields",
        (
            "Cosmic-Tools",
            "system-approve",
            "审批流管理",
            "/system-approve",
        ),
    ),
    (
        "fields",
        ("Alioth", "system-approve", "审批流管理", "/system-approve"),
    ),
    (
        "files",
        ("Cosmic-Tools", "repositories", "仓库管理", "/repositories"),
    ),
    (
        "fleet",
        ("WZ", "transport-wz", "承运商运营", "/transport-wz"),
    ),
    (
        "flows",
        (
            "Cosmic-Tools",
            "system-approve",
            "审批流管理",
            "/system-approve",
        ),
    ),
    (
        "flows",
        ("WZ", "comprehensive-wz", "综合管理", "/comprehensive-wz"),
    ),
    (
        "flows",
        ("Alioth", "system-approve", "审批流管理", "/system-approve"),
    ),
    (
        "gantt_items",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "gate_projects",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "gate_templates",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "git",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "git",
        ("Cosmic-Tools", "repositories", "仓库管理", "/repositories"),
    ),
    (
        "identities",
        ("WZ", "comprehensive-wz", "综合管理", "/comprehensive-wz"),
    ),
    (
        "incoming_inspections",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "inspection_batches",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "invoice_applications",
        ("WZ", "income-wz", "收入往来", "/income-wz"),
    ),
    (
        "invoices_out",
        ("WZ", "income-wz", "收入往来", "/income-wz"),
    ),
    (
        "items",
        ("Cosmic-Tools", "pipelines", "智能 CI", "/pipelines"),
    ),
    ("items", ("WZ", "contract-wz", "合同管理", "/contract-wz")),
    (
        "license_changes",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "life_limited_parts",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "maintenance_plans",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "maintenance_records",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "materials",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "ncrs",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    ("outgo_bills", ("WZ", "outgo-wz", "应付往来", "/outgo-wz")),
    (
        "outgo_invoices_in",
        ("WZ", "outgo-wz", "应付往来", "/outgo-wz"),
    ),
    (
        "outgo_payables",
        ("WZ", "outgo-wz", "应付往来", "/outgo-wz"),
    ),
    (
        "outgo_payment_matches",
        ("WZ", "outgo-wz", "应付往来", "/outgo-wz"),
    ),
    (
        "outgo_payment_payees",
        ("WZ", "outgo-wz", "应付往来", "/outgo-wz"),
    ),
    (
        "outgo_payments",
        ("WZ", "outgo-wz", "应付往来", "/outgo-wz"),
    ),
    (
        "outgo_waybills",
        ("WZ", "outgo-wz", "应付往来", "/outgo-wz"),
    ),
    (
        "outgo_yecai_payables",
        ("WZ", "outgo-wz", "应付往来", "/outgo-wz"),
    ),
    ("party", ("WZ", "contract-wz", "合同管理", "/contract-wz")),
    (
        "payment_plans",
        ("WZ", "contract-wz", "合同管理", "/contract-wz"),
    ),
    (
        "places",
        ("WZ", "transport-wz", "承运商运营", "/transport-wz"),
    ),
    (
        "plans",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "positions",
        ("WZ", "comprehensive-wz", "综合管理", "/comprehensive-wz"),
    ),
    (
        "post_certification_audits",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "production_certifications",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "production_schedules",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "products",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "products",
        ("WZ", "transport-wz", "承运商运营", "/transport-wz"),
    ),
    (
        "project_templates",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "projects",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "reassign",
        ("WZ", "transport-wz", "承运商运营", "/transport-wz"),
    ),
    (
        "receipt_matches",
        ("WZ", "income-wz", "收入往来", "/income-wz"),
    ),
    ("receipts", ("WZ", "income-wz", "收入往来", "/income-wz")),
    ("receivables", ("WZ", "income-wz", "收入往来", "/income-wz")),
    (
        "regulatory_assessments",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "release_tags",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "requirements",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    ("requirements", ("Alioth", "demand", "demand", "/demand")),
    (
        "risk_items",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "safety_events",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    ("sap_classes", ("WZ", "outgo-wz", "应付往来", "/outgo-wz")),
    (
        "schedule",
        ("WZ", "contract-wz", "合同管理", "/contract-wz"),
    ),
    (
        "settlement_orders",
        ("WZ", "contract-wz", "合同管理", "/contract-wz"),
    ),
    ("signing", ("WZ", "contract-wz", "合同管理", "/contract-wz")),
    (
        "status",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "status",
        (
            "Cosmic-Tools",
            "system-settings",
            "系统设置",
            "/system-settings",
        ),
    ),
    ("status", ("WZ", "outgo-wz", "应付往来", "/outgo-wz")),
    (
        "status",
        ("Alioth", "system-settings", "系统设置", "/system-settings"),
    ),
    (
        "structural_repairs",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "subjects",
        ("WZ", "comprehensive-wz", "综合管理", "/comprehensive-wz"),
    ),
    (
        "subsystems",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "supplier_change_notices",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    ("tasks", ("WZ", "logistics-wz", "委托物流", "/logistics-wz")),
    (
        "templates",
        ("WZ", "contract-wz", "合同管理", "/contract-wz"),
    ),
    (
        "test_runs",
        ("AVIC-CAASEC", "system-dev", "系统研发", "/system-dev"),
    ),
    (
        "tracking",
        ("WZ", "transport-wz", "承运商运营", "/transport-wz"),
    ),
    (
        "training_records",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "transport_tracking",
        ("WZ", "transport-wz", "承运商运营", "/transport-wz"),
    ),
    (
        "type_certifications",
        ("AVIC-CAASEC", "airworthiness", "适航管理", "/airworthiness"),
    ),
    (
        "vehicles",
        ("WZ", "transport-wz", "承运商运营", "/transport-wz"),
    ),
    (
        "verctrl",
        ("Cosmic-Tools", "repositories", "仓库管理", "/repositories"),
    ),
    (
        "versions",
        (
            "AVIC-CAASEC",
            "system-approve",
            "审批流程",
            "/system-approve",
        ),
    ),
    (
        "versions",
        (
            "Cosmic-Tools",
            "system-approve",
            "审批流管理",
            "/system-approve",
        ),
    ),
    (
        "versions",
        ("WZ", "comprehensive-wz", "综合管理", "/comprehensive-wz"),
    ),
    (
        "versions",
        ("Alioth", "system-approve", "审批流管理", "/system-approve"),
    ),
    ("waybill", ("WZ", "outgo-wz", "应付往来", "/outgo-wz")),
    ("waybills", ("WZ", "outgo-wz", "应付往来", "/outgo-wz")),
];
/// 资源类型的模块归属（连字符归一后查表）。系统域/未知类型返回 None。
///
/// 2026-08-21 修复：映射表条目带 namespace 元数据（如 approval_* 归属
/// AVIC-CAASEC·system-approve），但同一 resource_type 名可能存在于多个
/// namespace（WZ 也有 approval_*/products/status/versions 资源）。
/// 此前全局 .find() 首个命中即返回——WZ 矩阵把审批列错误标成
/// 「AVIC-CAASEC·审批流程」（用户实证：NS:WZ 权限管理看到 AVIC 内容）。
/// 现按当前进程 NAMESPACE 过滤：仅命中当前 namespace 条目；
/// 当前 namespace 无条目 → None（不贴错标签）。
pub fn resolve_module(resource_type: &str) -> Option<ResourceModuleRef> {
    let normalized = resource_type.replace('-', "_");
    let current_ns = std::env::var("NAMESPACE").ok();
    RESOURCE_MODULE_MAP
        .iter()
        .find(|(k, v)| {
            (*k == normalized || *k == resource_type)
                && current_ns.as_deref().is_none_or(|ns| v.0 == ns)
        })
        .map(
            |(_, (namespace, module_id, module_name, module_route))| ResourceModuleRef {
                namespace,
                module_id,
                module_name,
                module_route,
            },
        )
}

/// 模块归属三字段（module_name/module_route/namespace）：无归属时全 None。
pub fn module_fields(
    resource_type: &str,
) -> (
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
) {
    match resolve_module(resource_type) {
        Some(m) => (Some(m.module_name), Some(m.module_route), Some(m.namespace)),
        None => (None, None, None),
    }
}

/// OA 页面预览信息（add-ngac-oa-preview，dev-only）——采集器产物合并输出。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OaPreviewInfo {
    /// 截图访问相对路径（SSO 静态服务：/api/admin/ngac/previews/{rt}.png）
    pub png_url: String,
    /// 截图内高亮矩形（相对视口像素）
    pub rect: serde_json::Value,
    /// 采集时的页面 URL
    pub url: String,
    /// 采集时间（UTC ISO8601）
    pub captured_at: String,
}

/// 加载采集器 manifest（{resource_type → OaPreviewInfo}）；文件缺失/解析失败
/// → 空 map（preview 字段为 null，不报错）。每次调用重读（采集是低频 dev 操作）。
pub fn load_preview_manifest(preview_dir: &str) -> HashMap<String, OaPreviewInfo> {
    let path = std::path::Path::new(preview_dir).join("manifest.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return HashMap::new(),
    };
    match serde_json::from_str::<HashMap<String, OaPreviewInfo>>(&text) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("load_preview_manifest: manifest 解析失败: {}", e);
            HashMap::new()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // 解析链四层（纯函数，无 DB）
    // ========================================================================

    #[test]
    fn chain1_instance_identifier_wins() {
        let map = HashMap::new();
        let got = resolve_display_name(
            Some(42),
            Some("NOTICE-001"),
            "notice",
            &map,
            "notice:obj:42",
        );
        assert_eq!(got, "NOTICE-001");
    }

    #[test]
    fn chain1_collection_level_ignores_identifier() {
        // collection 级（fk_resource = 0）即使 identifier 非空也不采用（§2.2.1 边界）
        let map = HashMap::new();
        let got = resolve_display_name(Some(0), Some("stale"), "sso_admin", &map, "sso_admin:obj");
        assert_eq!(got, "SSO 管理");
    }

    #[test]
    fn chain2_meta_collections_name() {
        let mut map = HashMap::new();
        map.insert("zc_id_agreement".to_string(), "实现-协定".to_string());
        let got = resolve_display_name(
            Some(0),
            None,
            "zc_id_agreement",
            &map,
            "zc_id_agreement:obj",
        );
        assert_eq!(got, "实现-协定");
    }

    #[test]
    fn chain3_builtin_mapping() {
        let map = HashMap::new();
        for (key, expect) in BUILTIN_RESOURCE_TYPE_DISPLAY.iter() {
            let got = resolve_display_name(Some(0), None, key, &map, "fallback");
            assert_eq!(got, *expect, "builtin mapping failed for {}", key);
        }
    }

    #[test]
    fn chain4_unknown_type_falls_back_to_o_name() {
        let map = HashMap::new();
        let got = resolve_display_name(
            Some(7),
            None,
            "brand_new_type",
            &map,
            "brand_new_type:obj:7",
        );
        assert_eq!(got, "brand_new_type:obj:7");
    }

    // ========================================================================
    // resolve_module：按 NAMESPACE 过滤资源归属（2026-08-21 跨 ns 标签修复）
    // ========================================================================

    #[test]
    fn resolve_module_wz_does_not_get_avic_label() {
        // WZ 内嵌 SSO：approval_*/products 映射表有多 ns 条目（生成器修复后），
        // WZ 下必须命中 WZ 条目而非 AVIC（实证缺陷：WZ 矩阵把审批列标成
        // 「AVIC-CAASEC·审批流程」）
        std::env::set_var("NAMESPACE", "WZ");
        let a = resolve_module("approval_actions").expect("WZ 下 approval_actions 应命中 WZ 条目");
        assert_eq!(a.namespace, "WZ");
        assert_eq!(a.module_name, "综合管理");
        let f = resolve_module("approval_flows").expect("WZ 下 approval_flows 应命中 WZ 条目");
        assert_eq!(f.namespace, "WZ");
        // WZ 审批族（无前端字面量，手工归并）：approvals/delegation_rules 同源 comprehensive-wz
        let ap = resolve_module("approvals").expect("WZ 下 approvals 应命中 WZ 条目");
        assert_eq!(ap.namespace, "WZ");
        assert_eq!(ap.module_name, "综合管理");
        let dr = resolve_module("delegation_rules").expect("WZ 下 delegation_rules 应命中 WZ 条目");
        assert_eq!(dr.namespace, "WZ");
        let p = resolve_module("products").expect("WZ 下 products 应命中 WZ 条目");
        assert_eq!(p.namespace, "WZ");
        assert_eq!(p.module_route, "/transport-wz");
        // WZ 自己的资源仍正确归属
        let wz = resolve_module("waybills").expect("waybills 应归属 WZ");
        assert_eq!(wz.namespace, "WZ");
        assert_eq!(wz.module_route, "/outgo-wz");
        std::env::remove_var("NAMESPACE");
    }

    #[test]
    fn resolve_module_avic_gets_avic_label() {
        std::env::set_var("NAMESPACE", "AVIC-CAASEC");
        let a = resolve_module("approval_actions").expect("AVIC 下 approval_actions 应归属");
        assert_eq!(a.namespace, "AVIC-CAASEC");
        assert_eq!(a.module_name, "审批流程");
        let p = resolve_module("products").expect("AVIC 下 products 应归属");
        assert_eq!(p.namespace, "AVIC-CAASEC");
        std::env::remove_var("NAMESPACE");
    }

    #[test]
    fn resolve_module_without_namespace_falls_back_to_first_match() {
        // 独立 SSO（无 NAMESPACE env）：回退旧行为（首个命中）
        std::env::remove_var("NAMESPACE");
        let a = resolve_module("approval_actions").expect("无 NAMESPACE 时应回退");
        assert_eq!(a.namespace, "AVIC-CAASEC");
    }
    #[test]
    fn module_resolution_hit_and_miss() {
        // 命中：业务资源带模块归属 + 连字符归一
        let m = resolve_module("outgo_payments").expect("outgo_payments 应有模块归属");
        assert_eq!(
            (m.namespace, m.module_name, m.module_route),
            ("WZ", "应付往来", "/outgo-wz")
        );
        assert_eq!(
            resolve_module("outgo-payments").map(|x| x.module_route),
            Some("/outgo-wz"),
            "连字符变体归一后同归属"
        );
        // 系统域：无归属
        assert!(resolve_module("sso_admin").is_none());
        assert_eq!(module_fields("sso_admin"), (None, None, None));
        let (name, route, ns) = module_fields("consignments");
        assert_eq!(
            (ns, name, route),
            (Some("WZ"), Some("承运商运营"), Some("/transport-wz"))
        );
    }

    #[test]
    fn resource_type_display_chain() {
        let mut map = HashMap::new();
        map.insert("zc_id_agreement".to_string(), "实现-协定".to_string());
        // 链 2 优先
        assert_eq!(
            resolve_resource_type_display("zc_id_agreement", &map),
            "实现-协定"
        );
        // 链 3 内置
        assert_eq!(resolve_resource_type_display("sso_admin", &map), "SSO 管理");
        // fallback 原值
        assert_eq!(resolve_resource_type_display("unknown", &map), "unknown");
    }

    // ========================================================================
    // meta_collections 批量查询 + 降级
    // ========================================================================

    async fn test_pool() -> sqlx::PgPool {
        let url = std::env::var("DATABASE_URL")
            .or_else(|_| std::env::var("SSO_TEST_DATABASE_URL"))
            .unwrap_or_else(|_| {
                let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_string());
                format!("postgres://{}@localhost:5432/aliothstudio_test", user)
            });
        sqlx::PgPool::connect(&url)
            .await
            .expect("无法连接测试库，请先运行 `bash scripts/db/reset-db.sh --test`")
    }

    #[tokio::test]
    async fn meta_display_names_batch_resolves_known_types() {
        let pool = test_pool().await;
        let types: HashSet<String> = [
            "zc_id_agreement".to_string(),
            "definitely_not_a_table_xyz".to_string(),
        ]
        .into_iter()
        .collect();
        let map = meta_display_names(&pool, &types).await;
        // 已知业务表命中（seed 视图存在时）；未知类型不出现
        if let Some(name) = map.get("zc_id_agreement") {
            assert!(!name.is_empty());
        }
        assert!(!map.contains_key("definitely_not_a_table_xyz"));
    }

    #[tokio::test]
    async fn meta_display_names_degrades_silently_on_db_failure() {
        // 不可达地址：查询失败必须返回空 map（log::warn 降级，不 panic）
        let pool = sqlx::PgPool::connect_lazy("postgres://127.0.0.1:1/none")
            .expect("lazy connect never dials");
        let types: HashSet<String> = ["sso_admin".to_string()].into_iter().collect();
        let map = meta_display_names(&pool, &types).await;
        assert!(map.is_empty());
    }
}
