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
            internal_prefixes: vec![
                "/api/auth/".to_string(),
                "/api/ngac/".to_string(),
                "/health".to_string(),
            ],
        }
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
                    // and rely on ngac_resource_identifier mapping
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
            // 与迁移 029 seed 的 OA 对应；table 仅为名义归属（string_id 不查表）。
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
        ];
        self.register_all(cosmic);
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
    fn test_is_internal_path() {
        let reg = ResourceRegistry::new();
        assert!(reg.is_internal_path("/api/auth/login"));
        assert!(reg.is_internal_path("/api/ngac/decide"));
        assert!(reg.is_internal_path("/health"));
        assert!(!reg.is_internal_path("/api/service/identity/engineers"));
    }
}
