//! Resource Registry — maps URL path patterns to isahl resource tables.
//!
//! This solves the "route-level ID vs data-level ID" gap (Gap 1/2):
//! every frontend-accessible resource is registered here with its isahl table
//! mapping so that the PEP can produce correct `{resource_type}:{id}` pairs.
//!
//! # Resolution rules
//!
//! |URL pattern|Registry entry|Result|
//! |---|---|---|
//! |`/api/service/identity/engineers`|`"engineers" → {table: "zc_id_identity"}`|`engineers:0`|
//! |`/api/service/identity/engineers/42`|same|`engineers:42`|
//! |`/api/service/inventory/products`|`"products" → {table: "zc_id_production"}`|`products:0`|
//! |`/api/service/inventory/products/99`|same|`products:99`|
//!
//! When `id = 0` (list endpoint), the PDP returns a `visible_ids` set
//! for row-level filtering (Phase 3).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A resource type bound to an isahl table with optional ownership fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceTypeDef {
    /// NGAC resource_type (e.g. `"engineers"`, `"products"`)
    pub type_name: String,

    /// isahl table name (e.g. `"zc_id_identity"`, `"zc_id_production"`)
    pub table_name: String,

    /// Column that stores the primary key (default: `"id"`)
    pub id_column: String,

    /// Column for the record owner (default: `"created_by_id"`)
    pub owner_column: String,

    /// Whether this resource type has string-based IDs instead of bigint
    pub string_id: bool,
}

impl ResourceTypeDef {
    pub fn new(type_name: impl Into<String>, table_name: impl Into<String>) -> Self {
        Self {
            type_name: type_name.into(),
            table_name: table_name.into(),
            id_column: "id".to_string(),
            owner_column: "created_by_id".to_string(),
            string_id: false,
        }
    }

    pub fn with_string_id(mut self) -> Self {
        self.string_id = true;
        self
    }
}

/// Resolved resource — produced by `ResourceRegistry::resolve`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedResource {
    /// NGAC resource string: `"engineers:42"` or `"products:0"`
    pub resource: String,
    /// The resource type name (e.g. `"engineers"`)
    pub type_name: String,
    /// The resource ID (0 for list endpoints)
    #[serde(with = "common::serde_zuid")]
    pub resource_id: i64,
    /// The isahl table name
    pub table_name: String,
}

/// 路径前缀规则（**PEP 映射单源**）：命中即映射到固定资源，不解析实体/id。
///
/// 用途：`/api/admin/*` → `sso_admin:0`、`/api/audit/*` → `sso_audit:0`、
/// OpenAPI 管理面四族 → `openapi_admin:0` 等「整段路径共享一个恒定资源」的映射。
/// 前缀更长者优先（如 `/api/admin/api-clients` 先于 `/api/admin/`）。
#[derive(Debug, Clone)]
pub struct PrefixRule {
    pub prefix: String,
    pub type_name: String,
    pub resource_id: i64,
}

/// 方法 → NGAC action（**唯一实现**：SSO 中间件与 Gateway PEP 共用）。
///
/// 未知方法 ⇒ `access`（不映射为 `read`：避免把「未识别的写方法」当成读权限放行）。
pub fn action_for_method(method: &str) -> &'static str {
    match method.to_uppercase().as_str() {
        "GET" => "read",
        "POST" => "create",
        "PUT" | "PATCH" => "update",
        "DELETE" => "delete",
        _ => "access",
    }
}

/// Static resource registry that maps URL path prefixes to isahl table mappings.
///
/// Built once at startup and shared across the Gateway.
/// Factor routes are registered via `build.rs` and pulled in through
/// `factor_registry.rs`; internal Gateway routes are registered here.
#[derive(Debug, Clone)]
pub struct ResourceRegistry {
    /// Map from URL path segment (the entity name in `/api/{prefix}/{entity}`)
    /// to its isahl resource type.
    entities: HashMap<String, ResourceTypeDef>,

    /// 路径前缀规则（最长前缀优先）；解析顺序 = 前缀规则 → 实体解析。
    prefix_rules: Vec<PrefixRule>,

    /// Path prefix for Gateway internal routes that bypass NGAC.
    /// These are still authenticated (JWT required).
    internal_prefixes: Vec<String>,
}

impl Default for ResourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            entities: HashMap::new(),
            prefix_rules: Vec::new(),
            internal_prefixes: vec![
                "/api/auth/".to_string(),
                "/api/ngac/".to_string(),
                "/health".to_string(),
            ],
        }
    }

    /// 注册路径前缀规则（**PEP 映射单源**入口；长前缀优先由 `resolve_path` 保证）。
    pub fn with_prefix_rule(mut self, prefix: &str, type_name: &str, resource_id: i64) -> Self {
        self.prefix_rules.push(PrefixRule {
            prefix: prefix.to_string(),
            type_name: type_name.to_string(),
            resource_id,
        });
        self
    }

    /// SSO 管理/审计面声明（`/api/admin/*` → `sso_admin:0`、`/api/audit/*` → `sso_audit:0`；
    /// OpenAPI 四族为其更长的前缀 → `openapi_admin:0`）。SSO 中间件消费本构造，不再手写映射。
    pub fn with_sso_face_defaults(mut self) -> Self {
        self = self.with_prefix_rule("/api/admin/api-clients", "openapi_admin", 0);
        self = self.with_prefix_rule("/api/admin/api-plans", "openapi_admin", 0);
        self = self.with_prefix_rule("/api/admin/api-subscriptions", "openapi_admin", 0);
        self = self.with_prefix_rule("/api/admin/ngac/org-policy", "org_policy_admin", 0);
        self = self.with_prefix_rule("/api/admin/", "sso_admin", 0);
        self.with_prefix_rule("/api/audit/", "sso_audit", 0)
    }

    /// 解析 `(type_name, resource_id)`：先按**最长前缀规则**，未命中再落实体内核解析。
    ///
    /// 与 [`Self::resolve`] 的差别：后者只做实体段解析（Gateway 面），本函数是 PEP 映射单源入口。
    pub fn resolve_path(&self, path: &str) -> Option<ResolvedResource> {
        let mut best: Option<&PrefixRule> = None;
        for rule in &self.prefix_rules {
            if path.starts_with(rule.prefix.as_str())
                && best
                    .map(|b| rule.prefix.len() > b.prefix.len())
                    .unwrap_or(true)
            {
                best = Some(rule);
            }
        }
        if let Some(rule) = best {
            return Some(ResolvedResource {
                resource: format!("{}:{}", rule.type_name, rule.resource_id),
                type_name: rule.type_name.clone(),
                resource_id: rule.resource_id,
                table_name: String::new(),
            });
        }
        self.resolve(path)
    }

    /// Register a resource type mapping.
    pub fn register(&mut self, def: ResourceTypeDef) {
        self.entities.insert(def.type_name.clone(), def);
    }

    /// Register multiple resource types at once.
    pub fn register_all(&mut self, defs: Vec<ResourceTypeDef>) {
        for def in defs {
            self.register(def);
        }
    }

    /// 已注册 resource_type 有序清单（cover-org-resources-ngac / B-2）。
    ///
    /// 集合 OA 覆盖 ensure 的输入面：平台默认集（`with_alioth_defaults`）与调用方
    /// （ns 组合根 `ns_wiring::resource_registry`）扩展的并集——注册面是唯一真相源，
    /// 新增注册实体自动进入覆盖面（design D1）。
    pub fn registered_resource_types(&self) -> Vec<&str> {
        let mut types: Vec<&str> = self.entities.keys().map(String::as_str).collect();
        types.sort_unstable();
        types
    }

    /// Resolve an API path to an NGAC resource string.
    ///
    /// Returns `None` if the path doesn't match any registered resource
    /// (the PEP should then fall back to the old `map_resource` or deny).
    ///
    /// # Examples
    ///
    /// ```ignore
    /// registry.resolve("/api/service/identity/engineers")?
    ///   → ResolvedResource { resource: "engineers:0", type_name: "engineers", resource_id: 0, .. }
    /// registry.resolve("/api/service/identity/engineers/42")?
    ///   → ResolvedResource { resource: "engineers:42", type_name: "engineers", resource_id: 42, .. }
    /// ```
    pub fn resolve(&self, path: &str) -> Option<ResolvedResource> {
        let stripped = path.trim_start_matches("/api/").trim_start_matches('/');
        let parts: Vec<&str> = stripped.split('/').collect();
        if parts.is_empty() || parts[0].is_empty() {
            return None;
        }

        // Determine entity name:
        // For service routes:
        //   AVIC 3 段风格 `/api/service/{svc}/{entity}[/{id}]` → entity at index 2
        //   WZ 2 段风格 `/api/service/{entity}[/{id}|/sub...]` → entity at index 1
        //   （WZ 实体直接挂 service 根：/service/contracts/create 的 parts[2] 是动作/子路径）
        //   service 根（len==2：/api/service/invoice-sync）→ entity at index 1
        // 判定：parts[2] 是已注册实体 → 按 3 段取 parts[2]；否则按 2 段取 parts[1]。
        // 注意 parts[2] 为数字 id（/service/contracts/123）时同样落入 2 段分支。
        let entity_start_idx = if parts.len() >= 2 && parts[0] == "service" {
            if parts.len() >= 3 && self.entities.contains_key(&parts[2].replace('-', "_")) {
                2_usize
            } else {
                1_usize
            }
        } else {
            // /api/collections → parts[0] = "collections"
            // /api/product/42 → parts[0] = "product"
            0_usize
        };

        if entity_start_idx >= parts.len() {
            return None;
        }

        let entity_name = parts[entity_start_idx].replace('-', "_");

        let def = self.entities.get(&entity_name)?;

        // Determine the resource ID from the next path segment
        let id_str = parts.get(entity_start_idx + 1);
        let resource_id: i64 = match id_str {
            Some(s) if !s.is_empty() => {
                if def.string_id {
                    // String IDs aren't directly usable as bigint — mark as 0
                    // (collection-level decision; the string→bigint mapping table was
                    // retired 2026-10-06, drop-ngac-dead-schema)
                    0i64
                } else {
                    s.parse::<i64>().unwrap_or(0i64)
                }
            }
            _ => 0i64, // list endpoint
        };

        Some(ResolvedResource {
            resource: format!("{}:{}", def.type_name, resource_id),
            type_name: def.type_name.clone(),
            resource_id,
            table_name: def.table_name.clone(),
        })
    }

    /// Check if a path is an internal (non-NGAC) route.
    pub fn is_internal_path(&self, path: &str) -> bool {
        self.internal_prefixes
            .iter()
            .any(|p| path.starts_with(p.as_str()))
    }

    /// Get the current resource type count.
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// List all registered resource types (for diagnostics / permission enumeration).
    pub fn list_types(&self) -> Vec<&ResourceTypeDef> {
        self.entities.values().collect()
    }

    /// Populate with Alioth standard resource types.
    /// Called at startup for "Alioth" and "AVIC-CAASEC" namespaces.
    pub fn with_alioth_defaults(mut self) -> Self {
        let defaults = vec![
            // Alioth core entities
            // 审批操作端点（fix-approval-endpoint-gates）：/api/approvals[/{id}/approve|reject]
            // 与 /api/approvals/apply 的 PEP 资源——此前未注册致 resolve fallback 派生
            // 伪类型且无 OA，PDP 恒 deny（fail-open 下掩盖）。
            // string_id 恒定资源模式（对齐 openapi_admin/system_config）：全部子路径
            // （列表/{id}/approve|reject/apply）统一判定 approvals:0——审批操作端点是
            // 聚合虚拟资源（非行级 CRUD），行级属主由 handler 内 fk_operator/状态门禁承载。
            ResourceTypeDef::new("approvals", "zc_id_oper-approve").with_string_id(),
            // CCB 投票操作端点（fix-avic-ccb-vote-pep-gate）：聚合虚拟资源——
            // 行级属主（岗位校验 zc_id_subj-position ck_category=ccb_member）由
            // monitor handler 承载；与 approvals 同型（string_id 恒定 ccb_votes:0）。
            ResourceTypeDef::new("ccb_votes", "zc_id_deta-opinion").with_string_id(),
            // 存量 OA 补注册（add-ngac-oa-module-observability）：seed 已建 collection
            // OA 但 registry 缺席的类型——contacts/departments/positions（identity-org）、
            // waybill/receipts（accounts-receivable 域，表名对齐各自实体）。
            ResourceTypeDef::new("contacts", "zc_id_contacts"),
            ResourceTypeDef::new("departments", "zc_id_orga-department"),
            ResourceTypeDef::new("positions", "zc_id_subj-position"),
            ResourceTypeDef::new("waybill", "zc_id_orde-land"),
            ResourceTypeDef::new("receipts", "zc_id_stat-smt-bank"),
            // 基础设施配置：string_id 使 /api/system-config/* 全部分子路径统一为
            // system_config:0（对齐 SSO /api/admin/* → sso_admin:0 恒定资源模式），
            // 避免为运行时生成的配置 id 预 seed 实例级 OA。
            ResourceTypeDef::new("system_config", "system_configs").with_string_id(),
            // OpenAPI 管理/统计面恒定资源（refactor-openapi-admin-ngac-pdp）：
            // string_id 使 PEP 决策统一为 openapi_admin:0 / openapi_analytics:0，
            // 与 seed-ngac-07-openapi-admin-analytics-oas.sql 的 OA 对应；table 仅为名义归属（string_id 不查表）。
            ResourceTypeDef::new("openapi_admin", "api_clients").with_string_id(),
            ResourceTypeDef::new("openapi_analytics", "api_usage").with_string_id(),
            ResourceTypeDef::new("engineers", "zc_id_subj-employee"),
            ResourceTypeDef::new("materials", "zc_id_prod-material-made"),
            ResourceTypeDef::new("products", "zc_id_production"),
            ResourceTypeDef::new("measurements", "zc_id_unit"),
            ResourceTypeDef::new("ncrs", "zc_id_even-accident"),
            ResourceTypeDef::new("defects", "zc_id_even-report"),
            ResourceTypeDef::new("requirements", "zc_id_prod-request"),
            ResourceTypeDef::new("subsystems", "zc_id_prod-made"),
            ResourceTypeDef::new("investment_monthlies", "zc_id_eval-calculable"),
            ResourceTypeDef::new("quad_analyses", "zc_id_eval-calculable"),
            ResourceTypeDef::new("scenes", "zc_id_scene"),
            ResourceTypeDef::new("factors", "zc_id_factor"),
            ResourceTypeDef::new("functions", "zc_id_function"),
            ResourceTypeDef::new("collections", "meta_collections"),
            ResourceTypeDef::new("fields", "meta_fields"),
            ResourceTypeDef::new("plans", "zc_id_plan"),
            ResourceTypeDef::new("tasks", "zc_id_task"),
            ResourceTypeDef::new("projects", "zc_id_prjt-proc_ctrl"),
            ResourceTypeDef::new("boms", "zc_id_bom"),
            // AVIC-CAASEC monitor
            ResourceTypeDef::new("gate_projects", "zc_id_project"),
            ResourceTypeDef::new("gate_templates", "zc_id_process"),
            ResourceTypeDef::new("gate_audit_logs", "zc_id_even-approve"),
            // AVIC-CAASEC documentary
            ResourceTypeDef::new("change_requests", "zc_id_even-modify"),
            ResourceTypeDef::new("test_runs", "zc_id_stat-inspection"),
            ResourceTypeDef::new("cm_gates", "zc_id_oper-gate"),
            // AVIC-CAASEC metrology
            ResourceTypeDef::new("inspection_batches", "zc_id_stat-inspection"),
            // AVIC-CAASEC airworthiness
            ResourceTypeDef::new("continued_airworthiness_files", "zc_id_prod-request"),
            ResourceTypeDef::new("airworthiness_certificates", "zc_id_prod-air_cert-sales"),
            ResourceTypeDef::new("airworthiness_directives", "zc_id_even-modify"),
            ResourceTypeDef::new("certification_plans", "zc_id_plan"),
            ResourceTypeDef::new("maintenance_records", "zc_id_stat-inspection"),
            ResourceTypeDef::new("post_certification_audits", "zc_id_even-approve"),
            ResourceTypeDef::new("certification_bases", "zc_id_plan"),
            ResourceTypeDef::new("incoming_inspections", "zc_id_stat-inspection"),
            ResourceTypeDef::new("license_changes", "zc_id_contract"),
            ResourceTypeDef::new("production_schedules", "zc_id_plan"),
            ResourceTypeDef::new("regulatory_assessments", "zc_id_even-modify"),
            ResourceTypeDef::new("supplier_change_notices", "zc_id_even-modify"),
            ResourceTypeDef::new("training_records", "zc_id_stat-training"),
            ResourceTypeDef::new("safety_events", "zc_id_even-accident"),
            ResourceTypeDef::new("compliance_records", "zc_id_even-report"),
            ResourceTypeDef::new("design_change_impacts", "zc_id_even-modify"),
            ResourceTypeDef::new("design_manuals", "zc_id_prod-request"),
            ResourceTypeDef::new("release_tags", "zc_id_prod-request"),
            ResourceTypeDef::new("life_limited_parts", "zc_id_prod-request"),
            ResourceTypeDef::new("type_certifications", "zc_id_prod-type_cert-sales"),
            ResourceTypeDef::new("production_certifications", "zc_id_prod-license-sales"),
            ResourceTypeDef::new("aeg_reviews", "zc_id_even-modify"),
            ResourceTypeDef::new("maintenance_plans", "zc_id_plan-maintain"),
            ResourceTypeDef::new("structural_repairs", "zc_id_even-modify"),
            // AVIC-CAASEC commitment (new entities)
            // 管理面恒定资源（fix-approval-endpoint-gates 复查）：approval_flows 经自定义
            // repository 落子类（不触发 crud 行级 OA 自动注册）、delegation_rules 集合 OA
            // 缺失——行级解析恒 deny。string_id 统一集合判定（admin 全权关联覆盖），
            // 对齐 approvals/system_config/openapi_admin 恒定资源模式。
            ResourceTypeDef::new("approval_flows", "zc_id_process").with_string_id(),
            ResourceTypeDef::new("flow_nodes", "zc_id_even-approve").with_string_id(),
            ResourceTypeDef::new("approval_instances", "zc_id_oper-approve").with_string_id(),
            ResourceTypeDef::new("approval_actions", "zc_id_deta-opinion").with_string_id(),
            ResourceTypeDef::new("delegation_rules", "zc_id_operation").with_string_id(),
            // AVIC-CAASEC valuation
            ResourceTypeDef::new("project_budgets", "zc_id_eval-calculable"),
            // AVIC-CAASEC orchestration
            ResourceTypeDef::new("gantt_items", "zc_id_plan"),
            ResourceTypeDef::new("risk_items", "zc_id_even-accident"),
            ResourceTypeDef::new("project_templates", "zc_id_project"),
            // AVIC-CAASEC interfaces
            ResourceTypeDef::new("versions", "zc_id_tags-version"),
            // AVIC-CAASEC identity (new entities)
            ResourceTypeDef::new("skill_tags", "zc_id_tags-skill"),
            ResourceTypeDef::new("approval_roles", "zc_id_category"),
            ResourceTypeDef::new("ccb_members", "zc_id_oper-approve"),
            // workspace-dock 聚合端点（P1-4 完成态）：注册使 PEP 对
            // /api/global/overview（global:0）、/api/schedule/overview|todos
            // （schedule:0）走 visible_ids RLS 注入。schedule 同时覆盖
            // /api/schedule/items/{id}（schedule:{id}）——同一资源语义统一控制。
            ResourceTypeDef::new("global", "zc_id_oper-approve"),
            ResourceTypeDef::new("schedule", "zc_id_plan"),
            // 文件存储公共服务（/api/files）：实例级 files:{id} 供 PDP 决策，
            // 行级授权（ak_* 列）在服务层执行（fix-file-storage-local-auth）。
            // 单一 `files` 资源覆盖全部 5 种 kind 表（document/image/avatar/
            // package/ver_ctrl）——URL `/api/files/{id}` 无 kind 维度，resolve 恒解析
            // 为 files:{id}；注册别名表永不可达。表名仅为名义归属（列级授权/审计用），
            // 行级授权自包含（不注入 X-Authorized-Columns）。
            ResourceTypeDef::new("files", "zc_id_file-document"),
            // ── 存量未注册实体补注册（2026-10-08，NGAC_SPEC §2.1 注册义务收口）──
            // 来源 = resource-registry-coverage 门禁死分支修复后暴露的存量缺口（72 处）；
            // 本组 = 平台基线集（Alioth / AVIC-CAASEC 两 ns 共用 `with_alioth_defaults`）。
            // 表名取证 = 各 service.json ontology.entities + 后端 models.rs/repositories。
            // (Alioth) environment 服务：/environments CRUD
            ResourceTypeDef::new("environments", "zc_id_prot-env_config"),
            // (Alioth) inventory 服务：盘点/储元/凭证台账 + 物化库存统计
            ResourceTypeDef::new("countings", "zc_id_even-counting"),
            ResourceTypeDef::new("counting_details", "zc_id_deta-counting"),
            ResourceTypeDef::new("storage_nests", "zc_id_storage_rr_stock-in"),
            ResourceTypeDef::new("vouchers", "zc_id_stat-sto-voucher"),
            // /service/inventory/statistics（含 {id} 级）——StockStat 读物化储量
            // （stock_stat_repository.rs FROM zc_id_production_rr_storage）
            ResourceTypeDef::new("inventory", "zc_id_production_rr_storage"),
            // (Alioth) license 服务：/licenses CRUD（zc_id_prod-license-purchase）
            ResourceTypeDef::new("licenses", "zc_id_prod-license-purchase"),
            // (Alioth) todo 服务：/service/todos CRUD（todo-service/models.rs: table_name）
            ResourceTypeDef::new("todos", "zc_id_stus-task"),
            // (Alioth) todo-min 服务：/service/todo-mins CRUD（lib.rs:13 SQL 表）
            ResourceTypeDef::new("todo_mins", "zc_id_leve-task"),
            // (AVIC-CAASEC) airworthiness 服务：台账 CRUD（models.rs 各 impl table_name）
            ResourceTypeDef::new("airworthiness_limits", "zc_id_stan-air"),
            ResourceTypeDef::new(
                "certification_maintenance_requirements",
                "zc_id_stan-operation",
            ),
            ResourceTypeDef::new("master_minimum_equipment_items", "zc_id_bom-equipment"),
            ResourceTypeDef::new("structural_repair_manuals", "zc_id_file-srm"),
            ResourceTypeDef::new("weight_balance_manuals", "zc_id_file-wbm"),
            ResourceTypeDef::new("production_inspections", "zc_id_stat-inspection"),
            ResourceTypeDef::new("external_access_rules", "zc_id_form-access"),
            // (AVIC-CAASEC) interfaces 服务：/version-contexts CRUD
            ResourceTypeDef::new("version_contexts", "zc_id_vers-context"),
            // (AVIC-CAASEC) metrology 服务：/prod-files CRUD + 实例发布/内容读取子端点
            // （repositories/mod.rs INSERT/UPDATE isahl."zc_id_file-document"）
            ResourceTypeDef::new("prod_files", "zc_id_file-document"),
        ];
        self.register_all(defaults);
        self
    }

    /// WZ namespace 实体注册：与 `Pre-Proc/WZ/seed/seed-wz-ngac-resources.sql` 的
    /// collection OA 资源集对齐。WZ service 路由为 2 段风格 `/service/{entity}/...`
    /// （无 AVIC 的 svc 段），注册后 `resolve` 才能把 `/service/contracts/create`
    /// 正确解析为 `contracts:0`（parts[2] 动作段落入 2 段分支取 parts[1]）。
    /// 表名仅为名义归属（列级授权/审计用），PDP 决策只用 type_name。
    pub fn with_wz_defaults(mut self) -> Self {
        let wz = vec![
            // 合同管理（contract-wz 模块）
            ResourceTypeDef::new("contracts", "zc_id_contract"),
            ResourceTypeDef::new("contract_matters", "zc_id_contract_rr_matter"),
            ResourceTypeDef::new("contract_partys", "zc_id_contract_rr_party"),
            ResourceTypeDef::new("contract_agreements", "zc_id_contract_rr_agreement"),
            ResourceTypeDef::new("templates", "zc_id_contract"),
            ResourceTypeDef::new("amendments", "zc_id_contract_rr_agreement"),
            ResourceTypeDef::new("counterparties", "zc_id_subj-org"),
            ResourceTypeDef::new("bill_checks", "zc_id_bill-check"),
            ResourceTypeDef::new("attachments", "zc_id_file-document"),
            ResourceTypeDef::new("settlement_orders", "zc_id_stat-smt-voucher"),
            ResourceTypeDef::new("payment_plans", "zc_id_plan-payment"),
            ResourceTypeDef::new("signing", "zc_id_contract_rr_agreement"),
            ResourceTypeDef::new("execution", "zc_id_orde-land"),
            ResourceTypeDef::new("items", "zc_id_contract_rr_matter"),
            ResourceTypeDef::new("party", "zc_id_contract_rr_party"),
            ResourceTypeDef::new("dashboard", "zc_id_contract"),
            ResourceTypeDef::new("status", "zc_id_stus-contract"),
            // 审批/流（approval 模块）——approval_flows 已在 AVIC 段注册为恒定资源
            ResourceTypeDef::new("flows", "zc_id_process"),
            // 发票/收款/对账
            ResourceTypeDef::new("invoice_applications", "zc_id_invoice"),
            ResourceTypeDef::new("receipt_collections", "zc_id_bill-check"),
            ResourceTypeDef::new("invoices_out", "zc_id_invoice"),
            ResourceTypeDef::new("receivables", "zc_id_bill-check"),
            ResourceTypeDef::new("receipt_matches", "zc_id_bill-check"),
            ResourceTypeDef::new("outgo_waybills", "zc_id_orde-land"),
            ResourceTypeDef::new("outgo_payables", "zc_id_oper-payment"),
            ResourceTypeDef::new("outgo_payments", "zc_id_oper-payment"),
            ResourceTypeDef::new("outgo_payment_matches", "zc_id_bill-check"),
            ResourceTypeDef::new("outgo_invoices_in", "zc_id_invoice"),
            ResourceTypeDef::new("outgo_yecai_payables", "zc_id_bill-check"),
            ResourceTypeDef::new("outgo_bills", "zc_id_bill-check"),
            // 运输/物流
            ResourceTypeDef::new("consignments", "zc_id_orde-land"),
            ResourceTypeDef::new("waybills", "zc_id_orde-traffic"),
            ResourceTypeDef::new("vehicles", "zc_id_stor-ctn-vehicle"),
            ResourceTypeDef::new("transport_tracking", "zc_id_oper-transport_tracking"),
            ResourceTypeDef::new("tracking", "zc_id_oper-transport_tracking"),
            ResourceTypeDef::new("subjects", "zc_id_subj-org"),
            ResourceTypeDef::new("places", "zc_id_place"),
            ResourceTypeDef::new("tasks", "zc_id_task"),
            ResourceTypeDef::new("damage", "zc_id_even-accident"),
            ResourceTypeDef::new("damage_reports", "zc_id_even-report"),
            ResourceTypeDef::new("event_trackings", "zc_id_even-tracking"),
            ResourceTypeDef::new("event_accidents", "zc_id_even-accident"),
            ResourceTypeDef::new("reassign", "zc_id_oper-approve"),
            ResourceTypeDef::new("dispatch", "zc_id_oper-approve"),
            ResourceTypeDef::new("fleet", "zc_id_prod-freight_road-sales"),
            // 字典/计量
            ResourceTypeDef::new("measurement_units", "zc_id_unit"),
            ResourceTypeDef::new("exchange_rates", "zc_id_rate-exchange"),
            ResourceTypeDef::new("scalar_prices", "zc_id_scal-price"),
            // ── 存量未注册实体补注册（2026-10-08，NGAC_SPEC §2.1 收口）──
            // 来源 = resource-registry-coverage 死分支修复暴露的存量缺口（WZ 12 处）；
            // 表名取证 = 各 service.json ontology.entities。
            // wz-yy-wms-service（WMS 台账；URL 2 段风格 /batch-tags 等）
            ResourceTypeDef::new("batch_tags", "zc_id_tags-batch"),
            ResourceTypeDef::new("bin_titles", "zc_id_stor-plc-bin"),
            ResourceTypeDef::new("trustee_accounts", "zc_id_stor-acc-stock"),
            ResourceTypeDef::new("trustee_account_futures", "zc_id_stor-acc-futures"),
            ResourceTypeDef::new("deliveries", "zc_id_oper-delivery"),
            ResourceTypeDef::new("trade_orders", "zc_id_orde-consult"),
            // zc-id-stus-inventory-service（库存单据台账）
            ResourceTypeDef::new("inbound_orders", "zc_id_oper-put_in_stock"),
            ResourceTypeDef::new("outbound_orders", "zc_id_even-issue"),
            ResourceTypeDef::new("ownership_records", "zc_id_even-log"),
            ResourceTypeDef::new("stock_ledgers", "zc_id_docu-accounting"),
            ResourceTypeDef::new("stock_movements", "zc_id_oper-move"),
            ResourceTypeDef::new("stocktake_orders", "zc_id_even-counting"),
        ];
        self.register_all(wz);
        // 服务实体直接挂 service 根（2 段风格 /api/service/{service}/list，如 invoice-sync/
        // receipt-sync）：URL 段是服务名而资源类型是实体名（invoice_applications/
        // receipt_collections），register 以 type_name 为键无法表达——直接插入别名键，
        // 否则 PEP resolve 返回 None 回退 map_resource → 集合级判定恒 Deny（403）。
        self.entities.insert(
            "invoice_sync".to_string(),
            ResourceTypeDef::new("invoice_applications", "zc_id_invoice"),
        );
        self.entities.insert(
            "receipt_sync".to_string(),
            ResourceTypeDef::new("receipt_collections", "zc_id_bill-check"),
        );
        // WZ 服务根别名补注册（2026-10-10，`check-resource-registry-coverage` 报出 5 处
        // `resolve()=None`）：上述 2 段风格规则下，实体名取 `parts[1]`（服务名），而这 5 个
        // 服务名此前只注册了其**子实体**（见 ns 组合根 `Pre-Proc/WZ/gateway-host/src/lib.rs`
        // 的 `register_all`，对判据面不可见）⇒ PEP resolve 仍返回 None、回退 map_resource
        // 派生出 DB 中不存在的伪资源类型（阻断性缺陷）。
        // 表名取证 = 各 service.json `ontology.entities` 的首个实体（行内注释给出原始表名）。
        self.entities.insert(
            "accounts_payable".to_string(),
            ResourceTypeDef::new("outgo_payable_items", "zc_id_deta-bill-check"),
        );
        self.entities.insert(
            "accounts_receivable".to_string(),
            ResourceTypeDef::new("receivable_items", "zc_id_bill-check"),
        );
        self.entities.insert(
            "isahl_db".to_string(),
            ResourceTypeDef::new("identities", "zc_id_lifecycle"),
        );
        self.entities.insert(
            "transport_dispatch".to_string(),
            ResourceTypeDef::new("consignments", "zc_id_orde-traffic"),
        );
        self.entities.insert(
            "transport_operations".to_string(),
            ResourceTypeDef::new("kanban_cards", "zc_id_orde-land"),
        );
        // fix-gw-pep-handler-resource-drift：URL 实体段 → **handler 实际校验的资源名**别名。
        // 这些 URL 实体此前未注册（resolve 回退 map_resource 派生出 DB 中不存在的伪资源 →
        // 强制态恒 Deny）或注册名与 handler 校验名不一致（PEP 与 handler 落在不同 OA 上）。
        // 注册成 handler 同名资源后两侧判定同源；**不弱化校验**（校验点、动作、行级属主不变，
        // 仅让 PEP 的资源名与 handler 对齐）。表名仅为审计/列级归属（PDP 只用 type_name）。
        // 每个键的证据见本文件注释中的 `文件:行`（handler require_resource_access）。
        let handler_aliases: Vec<(&str, ResourceTypeDef)> = vec![
            // 采购询盘：/procure/*（transport-dispatch/handlers/procure.rs:180/206/278/301/420/446/539/574）
            (
                "procure",
                ResourceTypeDef::new("procure-inquiries", "zc_id_prod-request"),
            ),
            // 车型字典：/service/isahl-db/vehicle-types（identity-org/handlers/tracking.rs:77）
            (
                "vehicle_types",
                ResourceTypeDef::new("identities", "zc_id_subj-org"),
            ),
            // 线路站点：/traffic-lines/{id}/stops（identity-org/handlers/traffic_line_stops.rs:70/136）
            (
                "traffic_lines",
                ResourceTypeDef::new("traffic_line", "zc_id_place"),
            ),
            // 费用科目：/fee-subjects（accounts-receivable/handlers/fee_subject.rs:77/115/136）
            (
                "fee_subjects",
                ResourceTypeDef::new("receivables", "zc_id_bill-check"),
            ),
            // 关务拆单：/external/customs/split-bill（transport-operations/handlers/customs_split.rs:86）
            (
                "external",
                ResourceTypeDef::new("waybill", "zc_id_orde-land"),
            ),
            // 月度对账单推送：/monthly-statement/{month}/push-xingtai（transport-operations/handlers/settlement.rs:210）
            (
                "monthly_statement",
                ResourceTypeDef::new("settlements", "zc_id_plan-payment"),
            ),
            // FSSC 科目字典 / 回调：handler 判 outgo-yecai-payables
            // （accounts-payable/handlers/fssc.rs:260/432）
            (
                "sap_classes",
                ResourceTypeDef::new("outgo-yecai-payables", "zc_id_bill-check"),
            ),
            (
                "fssc_callbacks",
                ResourceTypeDef::new("outgo-yecai-payables", "zc_id_bill-check"),
            ),
            // 派车：/dispatch（transport-dispatch/handlers/dispatch.rs:23）
            (
                "dispatch",
                ResourceTypeDef::new("consignments", "zc_id_orde-land"),
            ),
            // 应付域 handler 使用连字符名（DB 两种命名并存）——PEP 对齐 handler 侧
            (
                "outgo_payables",
                ResourceTypeDef::new("outgo-payables", "zc_id_oper-payment"),
            ),
            (
                "outgo_payments",
                ResourceTypeDef::new("outgo-payments", "zc_id_oper-payment"),
            ),
            (
                "outgo_yecai_payables",
                ResourceTypeDef::new("outgo-yecai-payables", "zc_id_bill-check"),
            ),
        ];
        self.entities
            .extend(handler_aliases.into_iter().map(|(k, v)| (k.to_string(), v)));
        self
    }

    /// Cosmic-Tools namespace 实体注册：ct-git 版本控制（verctrl）资源。
    /// verctrl→zc_id_file-ver_ctrl（版本文件叶表）、ver_branch→zc_id_cate-ver_branch
    /// （版本分支维度表）。string_id 恒定资源模式：列表/详情/动作子路径
    /// （/service/ct-git/verctrl/branches|files|freezes 等）统一判定 verctrl:0 /
    /// ver_branch:0——集合级 OA 由 ngac_seed 幂等预置（admin 全权 / user 只读），
    /// 行级属主由 handler 内 fk_* / 服务层门禁承载（对齐 approvals/system_config
    /// 恒定资源模式，add-ct-git-vc-interop）。
    pub fn with_cosmic_tools_defaults(mut self) -> Self {
        let cosmic = vec![
            ResourceTypeDef::new("verctrl", "zc_id_file-ver_ctrl").with_string_id(),
            ResourceTypeDef::new("ver_branch", "zc_id_cate-ver_branch").with_string_id(),
            // add-ct-git-hosting-issue：Issue 跟踪 / task 面 / git 托管端点——
            // 均 string_id 恒定资源（集合级判定，OA 由 ngac_seed 幂等预置，
            // 行级属主由 handler 内门禁承载，对齐 verctrl 模式）。
            ResourceTypeDef::new("issues", "zc_id_even-issue").with_string_id(),
            ResourceTypeDef::new("tasks", "zc_id_task-fix").with_string_id(),
            ResourceTypeDef::new("git", "zc_id_stor-plc-repository").with_string_id(),
            // system-settings（平台级配置六面，service.json = Services/system-settings）：
            // 单位制 / 汇率 / 运行环境 / 主题与语言 / 许可证。均 string_id 恒定资源
            // （集合级 OA 由 ngac_seed 幂等预置，行级属主由 handler 内门禁承载，
            // 对齐 verctrl/issues 模式）。表归属见 service.json#ontology.entities。
            ResourceTypeDef::new("units", "zc_id_unit").with_string_id(),
            ResourceTypeDef::new("rates", "zc_id_rate").with_string_id(),
            ResourceTypeDef::new("environments", "zc_id_prot-env_config").with_string_id(),
            ResourceTypeDef::new("profiles", "zc_id_prot-profile_config").with_string_id(),
            ResourceTypeDef::new("licenses", "zc_id_prod-license-sales").with_string_id(),
            // ── 存量未注册实体补注册（2026-10-08，NGAC_SPEC §2.1 收口）──
            // 来源 = resource-registry-coverage 死分支修复暴露的存量缺口（Cosmic-Tools 42 处）；
            // 表名取证 = 各 service.json ontology.entities（后端 models/repositories 交叉复核）。
            // bv-local-*（本地服务门店线）
            ResourceTypeDef::new("appointments", "zc_id_stat-task"),
            ResourceTypeDef::new("service_categories", "zc_id_category"),
            ResourceTypeDef::new("service_items", "zc_id_prod-consulting"),
            ResourceTypeDef::new("service_orders", "zc_id_stat-trade_order"),
            ResourceTypeDef::new("service_order_items", "zc_id_deta-trade_order"),
            ResourceTypeDef::new("payment_transactions", "zc_id_stat-smt-channel"),
            ResourceTypeDef::new("settlement_vouchers", "zc_id_stat-smt-voucher"),
            // 微信支付集成回调（无行级身份 ⇒ 恒定资源；名义表 = 支付流水）
            ResourceTypeDef::new("wechat_pay", "zc_id_stat-smt-channel").with_string_id(),
            ResourceTypeDef::new("technicians", "zc_id_subj-employee"),
            ResourceTypeDef::new("technician_skills", "zc_id_tags-skill"),
            // 库存/仓储线
            ResourceTypeDef::new("bin_locations", "zc_id_stor-plc-bin"),
            ResourceTypeDef::new("storage_warehouses", "zc_id_stor-plc-warehouse"),
            ResourceTypeDef::new("inventory_records", "zc_id_stat-whs-voucher"),
            // 物化库存余额（mv_inventory 物化视图无行级身份 ⇒ 恒定资源）
            ResourceTypeDef::new("inventory_balance", "mv_inventory").with_string_id(),
            // 订单/履约/结算线
            ResourceTypeDef::new("trade_order_items", "zc_id_deta-trade_order"),
            ResourceTypeDef::new("fulfillment_tasks", "zc_id_plan-perform"),
            ResourceTypeDef::new("logistics_orders", "zc_id_orde-traffic"),
            ResourceTypeDef::new("logistics_traces", "zc_id_even-tracking"),
            ResourceTypeDef::new("regions", "zc_id_geog-area"),
            ResourceTypeDef::new("contractors", "zc_id_subj-org"),
            ResourceTypeDef::new("settlement_bills", "zc_id_bill-check"),
            // 订单计划聚合面（只读派生，集合级）
            ResourceTypeDef::new("order_plan", "zc_id_plan"),
            ResourceTypeDef::new("plan_change_sync", "zc_id_even-modify"),
            // 采购/拣货
            ResourceTypeDef::new("pick_requests", "zc_id_prod-stor_sorting-request"),
            ResourceTypeDef::new("pick_request_items", "zc_id_bom-output_rr_item"),
            // 生产/领退料线（production-order + zc-id-plan-material-service）
            ResourceTypeDef::new("production_orders", "zc_id_stat-task"),
            ResourceTypeDef::new("production_order_items", "zc_id_deta-tsk"),
            ResourceTypeDef::new("make_orders", "zc_id_stat-tsk-make"),
            ResourceTypeDef::new("material_plans", "zc_id_plan-material"),
            ResourceTypeDef::new("material_consumptions", "zc_id_bom-outbound"),
            ResourceTypeDef::new("material_consumption_items", "zc_id_bom-output_rr_item"),
            ResourceTypeDef::new("material_productions", "zc_id_bom-inbound"),
            ResourceTypeDef::new("material_production_items", "zc_id_bom-input_rr_item"),
            ResourceTypeDef::new("material_requisitions", "zc_id_bom-pickup"),
            ResourceTypeDef::new("material_requisition_items", "zc_id_bom-output_rr_item"),
            ResourceTypeDef::new("material_scraps", "zc_id_bom-outbound"),
            ResourceTypeDef::new("material_scrap_items", "zc_id_bom-output_rr_item"),
            // git-repo 分支/标签
            ResourceTypeDef::new("git_branches", "zc_id_cate-ver_branch"),
            ResourceTypeDef::new("git_tags", "zc_id_cons-date-tags"),
            // ct-git 托管面叶实体（issues/tasks/git/verctrl 已在既有注册面）：
            // 表名取证 = ct-git backend repositories/*.rs
            ResourceTypeDef::new("repositories", "zc_id_stor-plc-repository"),
            ResourceTypeDef::new("pipelines", "zc_id_proc-cicd"),
            ResourceTypeDef::new("pipeline_runs", "zc_id_even-log"),
            ResourceTypeDef::new("reviews", "zc_id_appr-code-review"),
            ResourceTypeDef::new("assist_logs", "zc_id_msgs-chat_ai"),
            ResourceTypeDef::new("milestones", "zc_id_tags-milestone"),
            // AI 助手端点（POST /assistant/*，无行级身份 ⇒ 恒定资源）
            ResourceTypeDef::new("assistant", "zc_id_msgs-chat_ai").with_string_id(),
            // approval 服务聚合面（approval-cc/analytics；流程/实例 CRUD 已在基线注册）
            ResourceTypeDef::new("approval", "zc_id_oper-approve").with_string_id(),
            // authority 服务选择器（positions 已在基线集）
            ResourceTypeDef::new("employees", "zc_id_subj-employee"),
            ResourceTypeDef::new("approvers", "zc_id_subj-position"),
        ];
        self.register_all(cosmic);
        self
    }

    /// SE namespace 资源（研发预算管理）——3 段风格 /service/{svc}/{entity}，
    /// 实体名取 parts[2]（resolve 器对注册实体自动按 3 段解析）。
    /// 表归属见 Pre-Proc/SE/domain-mapping.md（零建表定稿）。
    pub fn with_se_defaults(mut self) -> Self {
        let se = vec![
            // rd-project（研发项目主数据）
            ResourceTypeDef::new("projects", "zc_id_project"),
            // budget-core（预算/调整/结算/成本要素）
            ResourceTypeDef::new("budgets", "zc_id_plan-project"),
            ResourceTypeDef::new("adjustments", "zc_id_plan-project"),
            ResourceTypeDef::new("settlements", "zc_id_plan-payment"),
            ResourceTypeDef::new("cost_elements", "zc_id_tags-finance"),
            // sap-execution（导入作业/过账/聚合/对账）
            ResourceTypeDef::new("import_jobs", "zc_id_event"),
            ResourceTypeDef::new("postings", "zc_id_stat-smt-voucher"),
            ResourceTypeDef::new("aggregates", "zc_id_stat-smt-voucher"),
            ResourceTypeDef::new("reconciliation", "zc_id_plan-project"),
            // budget-analysis（差异/报表/BI——派生查询集合级）
            ResourceTypeDef::new("variance", "zc_id_plan-project"),
            ResourceTypeDef::new("reports", "zc_id_plan-project"),
            ResourceTypeDef::new("bi", "zc_id_plan-project"),
            // rd-milestone（节点目录/桥行）
            ResourceTypeDef::new("node_templates", "zc_id_tags-milestone"),
            ResourceTypeDef::new("milestones", "zc_id_project_r_milestone-tags"),
            // rd-project 契约切片补齐（close-se-functional-gaps；此前未注册 ⇒ resolve None ⇒ PDP 拒绝）
            ResourceTypeDef::new("categories", "zc_id_tags-project"),
            ResourceTypeDef::new("document_types", "zc_id_cate-file"),
            ResourceTypeDef::new("documents", "zc_id_document"),
            ResourceTypeDef::new("project_codes", "zc_id_project"),
            ResourceTypeDef::new("power_stations", "zc_id_project"),
            ResourceTypeDef::new("project_relations", "zc_id_project_rr_dependency"),
            // 主数据选项（组织/人员/维度标签）
            ResourceTypeDef::new("departments", "zc_id_subj-org"),
            ResourceTypeDef::new("employees", "zc_id_subj-employee"),
            ResourceTypeDef::new("dimension_tags", "zc_id_tags-project"),
        ];
        self.register_all(se);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_empty_path() {
        let reg = ResourceRegistry::new().with_alioth_defaults();
        assert!(reg.resolve("/api").is_none());
    }

    #[test]
    fn test_registered_resource_types_sorted_and_inclusive() {
        // cover-org-resources-ngac（B-2）：覆盖 ensure 的输入面——平台默认集 + 调用方扩展
        // 的并集、有序去重（HashMap 键 = type_name，天然去重）。
        let reg = ResourceRegistry::new().with_alioth_defaults();
        let types = reg.registered_resource_types();
        assert!(!types.is_empty(), "平台默认集非空");
        assert!(
            types.windows(2).all(|w| w[0] < w[1]),
            "返回有序清单（调用方可稳定 diff/日志）"
        );
        assert!(types.contains(&"approvals"), "含审批恒定资源");

        let mut extended = ResourceRegistry::new().with_alioth_defaults();
        extended.register(ResourceTypeDef::new(
            "z-test-cov-extra",
            "isahl.z_test_cov_extra",
        ));
        let extended_types = extended.registered_resource_types();
        assert!(
            extended_types.contains(&"z-test-cov-extra"),
            "调用方扩展进入覆盖面"
        );
        assert_eq!(
            extended_types.len(),
            types.len() + 1,
            "新增注册实体自动覆盖（design D1）"
        );
    }

    #[test]
    fn test_resolve_approvals_constant_resource() {
        // fix-approval-endpoint-gates：审批操作端点恒定资源——全部子路径统一 approvals:0
        let reg = ResourceRegistry::new().with_alioth_defaults();
        for path in [
            "/api/approvals",
            "/api/approvals/123",
            "/api/approvals/123/approve",
            "/api/approvals/apply",
        ] {
            let r = reg
                .resolve(path)
                .unwrap_or_else(|| panic!("resolve failed: {path}"));
            assert_eq!(r.resource, "approvals:0", "path={path}");
            assert_eq!(r.type_name, "approvals");
        }
    }

    #[test]
    fn test_resolve_ccb_votes_constant_resource() {
        // fix-avic-ccb-vote-pep-gate：CCB 投票操作端点恒定资源——统一 ccb_votes:0
        let reg = ResourceRegistry::new().with_alioth_defaults();
        for path in [
            "/api/service/monitor/ccb-votes",
            "/api/service/monitor/ccb-votes/by-change/338139418208796",
        ] {
            let r = reg
                .resolve(path)
                .unwrap_or_else(|| panic!("resolve failed: {path}"));
            assert_eq!(r.resource, "ccb_votes:0", "path={path}");
            assert_eq!(r.type_name, "ccb_votes");
        }
    }
    #[test]
    fn test_resolve_se_new_entities() {
        // close-se-functional-gaps：rd-project 契约切片新增端点 MUST 可解析（未注册 = PDP 拒绝）
        let reg = ResourceRegistry::new()
            .with_alioth_defaults()
            .with_se_defaults();
        for (path, want_type) in [
            ("/api/service/se-rd-project/categories", "categories"),
            ("/api/service/se-rd-project/categories/123", "categories"),
            (
                "/api/service/se-rd-project/document-types",
                "document_types",
            ),
            (
                "/api/service/se-rd-project/project-codes/import",
                "project_codes",
            ),
            (
                "/api/service/se-rd-project/power-stations",
                "power_stations",
            ),
            (
                "/api/service/se-rd-project/project-relations/9",
                "project_relations",
            ),
            ("/api/service/se-rd-project/departments", "departments"),
            ("/api/service/se-rd-project/employees", "employees"),
            (
                "/api/service/se-rd-project/dimension-tags",
                "dimension_tags",
            ),
            ("/api/service/se-rd-project/documents/7", "documents"),
            // 既有实体仍解析（3 段风格不受新增影响）
            ("/api/service/se-rd-project/projects/42", "projects"),
        ] {
            let r = reg
                .resolve(path)
                .unwrap_or_else(|| panic!("resolve failed: {path}"));
            assert_eq!(r.type_name, want_type, "path={path}");
        }
    }

    #[test]
    fn test_resolve_flat_list() {
        let reg = ResourceRegistry::new().with_alioth_defaults();
        let r = reg.resolve("/api/collections").unwrap();
        assert_eq!(r.resource, "collections:0");
        assert_eq!(r.type_name, "collections");
        assert_eq!(r.resource_id, 0);
        assert_eq!(r.table_name, "meta_collections");
    }

    #[test]
    fn test_resolve_flat_item() {
        let reg = ResourceRegistry::new().with_alioth_defaults();
        let r = reg.resolve("/api/collections/123").unwrap();
        assert_eq!(r.resource, "collections:123");
        assert_eq!(r.resource_id, 123);
    }

    #[test]
    fn test_resolve_workspace_aggregates() {
        let reg = ResourceRegistry::new().with_alioth_defaults();
        // /api/global/overview → global:0（列表）
        let g = reg.resolve("/api/global/overview").unwrap();
        assert_eq!(g.resource, "global:0");
        assert_eq!(g.type_name, "global");
        assert_eq!(g.resource_id, 0);
        // /api/schedule/overview → schedule:0
        let so = reg.resolve("/api/schedule/overview").unwrap();
        assert_eq!(so.resource, "schedule:0");
        // /api/schedule/items/123 → schedule:0（parts[1]="items" 非数字 id，列表级）
        let si = reg.resolve("/api/schedule/items/123").unwrap();
        assert_eq!(si.resource, "schedule:0");
        assert_eq!(si.resource_id, 0);
    }

    #[test]
    fn test_resolve_factor_list() {
        let reg = ResourceRegistry::new().with_alioth_defaults();
        let r = reg.resolve("/api/service/identity/engineers").unwrap();
        assert_eq!(r.resource, "engineers:0");
        assert_eq!(r.type_name, "engineers");
        assert_eq!(r.table_name, "zc_id_subj-employee");
    }

    #[test]
    fn test_resolve_factor_item() {
        let reg = ResourceRegistry::new().with_alioth_defaults();
        let r = reg.resolve("/api/service/identity/engineers/42").unwrap();
        assert_eq!(r.resource, "engineers:42");
        assert_eq!(r.resource_id, 42);
    }

    #[test]
    fn test_resolve_factor_nested_path() {
        let reg = ResourceRegistry::new().with_alioth_defaults();
        // Sub-paths like /export, /children still resolve to the parent entity
        let r = reg
            .resolve("/api/service/identity/engineers/42/children")
            .unwrap();
        assert_eq!(r.resource, "engineers:42");
        assert_eq!(r.resource_id, 42);
    }

    #[test]
    fn test_resolve_unknown_type() {
        let reg = ResourceRegistry::new().with_alioth_defaults();
        assert!(reg.resolve("/api/service/unknown/widgets").is_none());
    }

    #[test]
    fn test_resolve_wz_two_segment_action() {
        // WZ 2 段风格：/service/contracts/create → parts[2] 是动作，取 parts[1]=contracts
        let reg = ResourceRegistry::new()
            .with_alioth_defaults()
            .with_wz_defaults();
        let r = reg.resolve("/api/service/contracts/create").unwrap();
        assert_eq!(r.resource, "contracts:0");
        assert_eq!(r.type_name, "contracts");
    }

    #[test]
    fn test_resolve_wz_two_segment_item() {
        // /service/contracts/123 → parts[2] 是数字 id，取 parts[1]=contracts + id
        let reg = ResourceRegistry::new()
            .with_alioth_defaults()
            .with_wz_defaults();
        let r = reg.resolve("/api/service/contracts/123").unwrap();
        assert_eq!(r.resource, "contracts:123");
        assert_eq!(r.resource_id, 123);
    }

    #[test]
    fn test_resolve_wz_service_root_post() {
        // POST /api/service/invoice-sync（len==2 根路径）→ entity=parts[1] → 别名映射
        let reg = ResourceRegistry::new()
            .with_alioth_defaults()
            .with_wz_defaults();
        let r = reg.resolve("/api/service/invoice-sync").unwrap();
        assert_eq!(r.resource, "invoice_applications:0");
    }

    #[test]
    fn test_resolve_wz_service_root_alias() {
        // /service/invoice-sync/list → parts[2]='list' 非注册实体 → 2 段取 parts[1]，
        // URL 段转下划线后命中服务根别名键 → 映射到 invoice_applications 资源类型。
        let reg = ResourceRegistry::new()
            .with_alioth_defaults()
            .with_wz_defaults();
        let r = reg.resolve("/api/service/invoice-sync/list").unwrap();
        assert_eq!(r.resource, "invoice_applications:0");
        assert_eq!(r.type_name, "invoice_applications");
        let r2 = reg.resolve("/api/service/receipt-sync/list").unwrap();
        assert_eq!(r2.resource, "receipt_collections:0");
    }

    #[test]
    fn test_resolve_wz_two_segment_subresource() {
        // /service/contracts/templates → parts[2] 是已注册实体 templates，按 3 段取
        let reg = ResourceRegistry::new()
            .with_alioth_defaults()
            .with_wz_defaults();
        let r = reg.resolve("/api/service/contracts/templates").unwrap();
        assert_eq!(r.resource, "templates:0");
        assert_eq!(r.type_name, "templates");
    }

    #[test]
    fn test_resolve_wz_avic_three_segment_unchanged() {
        // AVIC 3 段风格不受影响：parts[2] 已注册实体仍按 3 段解析
        let reg = ResourceRegistry::new()
            .with_alioth_defaults()
            .with_wz_defaults();
        let r = reg.resolve("/api/service/identity/engineers/42").unwrap();
        assert_eq!(r.resource, "engineers:42");
    }

    #[test]
    fn test_resolve_wz_unknown_two_segment() {
        // 2 段路径实体未注册 → None（fallback map_resource 兜底）
        let reg = ResourceRegistry::new()
            .with_alioth_defaults()
            .with_wz_defaults();
        assert!(reg.resolve("/api/service/nonexistent/create").is_none());
    }

    #[test]
    fn test_resolve_wz_handler_resource_aliases() {
        // fix-gw-pep-handler-resource-drift：PEP 解析出的资源名必须 == handler 实际校验名
        let reg = ResourceRegistry::new()
            .with_alioth_defaults()
            .with_wz_defaults();
        let cases = [
            // 采购询盘（procure.rs）
            (
                "/api/service/transport-dispatch/procure/inquiries",
                "procure-inquiries:0",
            ),
            // 子路径 id 段（/procure/inquiries/{id}）——PEP 只认 entity 后第一段
            // （这里是 "inquiries"，非数字）→ id=0（残留：PEP id ≠ handler 行级 id，
            // 因 CTE 对具体行无 OA 时回落 fk_resource=0 集合 OA，判定结果不变）
            (
                "/api/service/transport-dispatch/procure/inquiries/42",
                "procure-inquiries:0",
            ),
            // 车型（identity-org/tracking.rs）
            ("/api/service/isahl-db/vehicle-types", "identities:0"),
            // 线路站点（identity-org/traffic_line_stops.rs）
            (
                "/api/service/isahl-db/traffic-lines/7/stops",
                "traffic_line:7",
            ),
            // 费用科目（accounts-receivable/fee_subject.rs）
            (
                "/api/service/accounts-receivable/fee-subjects",
                "receivables:0",
            ),
            // 关务拆单（transport-operations/customs_split.rs）
            (
                "/api/service/transport-operations/external/customs/split-bill",
                "waybill:0",
            ),
            // 月度对账推送（transport-operations/settlement.rs）
            (
                "/api/service/transport-operations/monthly-statement/2026-08/push-xingtai",
                "settlements:0",
            ),
            // FSSC 科目字典 / 回调（accounts-payable/fssc.rs）
            (
                "/api/service/accounts-payable/sap-classes",
                "outgo-yecai-payables:0",
            ),
            (
                "/api/service/accounts-payable/fssc-callbacks",
                "outgo-yecai-payables:0",
            ),
            // 派车（transport-dispatch/dispatch.rs）
            ("/api/service/transport-dispatch/dispatch", "consignments:0"),
            // 应付域连字符名对齐
            (
                "/api/service/accounts-payable/outgo-payables",
                "outgo-payables:0",
            ),
            (
                "/api/service/accounts-payable/outgo-payments/42",
                "outgo-payments:42",
            ),
            (
                "/api/service/accounts-payable/outgo-yecai-payables",
                "outgo-yecai-payables:0",
            ),
        ];
        for (path, expected) in cases {
            let r = reg
                .resolve(path)
                .unwrap_or_else(|| panic!("resolve failed: {path}"));
            assert_eq!(r.resource, expected, "path={path}");
        }
    }

    #[test]
    fn test_resolve_se_three_segment_list() {
        // SE 3 段风格：/service/se-rd-project/projects → parts[2]=projects → projects:0
        let reg = ResourceRegistry::new()
            .with_alioth_defaults()
            .with_se_defaults();
        let r = reg.resolve("/api/service/se-rd-project/projects").unwrap();
        assert_eq!(r.resource, "projects:0");
        assert_eq!(r.type_name, "projects");
        assert_eq!(r.table_name, "zc_id_project");
    }

    #[test]
    fn test_resolve_se_three_segment_item() {
        // /service/se-rd-project/projects/{id} → projects:{id}
        let reg = ResourceRegistry::new()
            .with_alioth_defaults()
            .with_se_defaults();
        let r = reg
            .resolve("/api/service/se-rd-project/projects/123")
            .unwrap();
        assert_eq!(r.resource, "projects:123");
        assert_eq!(r.resource_id, 123);
    }

    #[test]
    fn test_resolve_se_milestone_templates() {
        let reg = ResourceRegistry::new()
            .with_alioth_defaults()
            .with_se_defaults();
        let r = reg
            .resolve("/api/service/se-rd-milestone/node-templates")
            .unwrap();
        assert_eq!(r.type_name, "node_templates");
        assert_eq!(r.table_name, "zc_id_tags-milestone");
    }

    #[test]
    fn test_is_internal_path() {
        let reg = ResourceRegistry::new();
        assert!(reg.is_internal_path("/api/auth/login"));
        assert!(reg.is_internal_path("/api/ngac/decide"));
        assert!(reg.is_internal_path("/health"));
        assert!(!reg.is_internal_path("/api/service/identity/engineers"));
    }
}
