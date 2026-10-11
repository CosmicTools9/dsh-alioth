//! PEP 内部 seam — HTTP 方法/路径 → NGAC action/resource 映射（评审候选 5 分缝之一：resource_map）。
//!
//! 自 `pep/middleware.rs` 内部抽出；含平台默认注册表 helper。

use ngac_contract::ResourceRegistry;

/// 平台默认资源注册表（**不含任何 namespace 维度扩展**）。
///
/// ns 维度扩展归组合根持有：`main.rs` 以 `ns_wiring::resource_registry(base)` 构建本 ns 的
/// 注册表并经 `with_resource_registry` 显式覆盖（组合根归 ns 仓——openspec/specs/ns-isolation
/// :: platform-composition-root-ns-agnostic；门禁 `check-platform-ns-agnostic.ts`）。
/// 本 helper 仅是**未显式注入**时的平台缺省（测试 seam 与框架面一致）。
pub(super) fn platform_resource_registry() -> ResourceRegistry {
    ResourceRegistry::new().with_alioth_defaults()
}

/// Map HTTP method to action string
/// 方法 → NGAC action（**消费判定链唯一实现** `ngac_contract::action_for_method`）。
///
/// 此前本处自持第二份映射，默认值还与 SSO 侧相左（本处 `read` vs SSO `access`）——
/// 未识别方法被当成读权限存在放行风险；收敛后统一为 `access`（不匹配任何 association ⇒ deny）。
pub(super) fn method_to_action(method: &str) -> &'static str {
    ngac_contract::action_for_method(method)
}

/// Map HTTP request path to NGAC resource string in `type:id` format.
///
/// All resources are normalized to two segments: `{type}:{id}`.
/// Collection-level operations (list/create) use `id = 0`.
///
/// Examples:
/// - /api/schemas -> schema:0
/// - /api/schemas/123 -> schema:123
/// - /api/schemas/123/export -> schema:123
/// - /api/collections -> collection:0
/// - /api/collections/abc -> collection:abc
/// - /api/product -> product:0
/// - /api/product/42 -> product:42
pub(super) fn map_resource(path: &str) -> String {
    let stripped = path.trim_start_matches("/api/").trim_start_matches('/');
    let parts: Vec<&str> = stripped.split('/').collect();
    if parts.is_empty() || parts[0].is_empty() {
        return "api:0".to_string();
    }

    // /api/service/{svc}/{entity}[/{id}]：entity 段为资源类型（registry 未注册的
    // service 实体也按实体名推导，而非整路径 fallback 出 service_xxx 伪类型）。
    // 与 scope 推导（PEP scope 校验）一致，保证 PDP 决策与 scope 校验同实体。
    if parts.len() >= 3 && parts[0] == "service" {
        let entity = parts[2].replace('-', "_");
        let id = parts
            .get(3)
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        return format!("{}:{}", entity, id);
    }

    // OpenAPI 管理/统计面恒定资源（refactor-openapi-admin-ngac-pdp；OA 种子 =
    // Framework/seed/seed-ngac-07-openapi-admin-analytics-oas.sql）：
    // - usage 系列（统计聚合，全 GET）→ openapi_analytics:0
    // - scope-catalog / provider-catalog / outbound（管理面读+出向管理 CRUD）→ openapi_admin:0
    // 文档路径（openapi.json/openapi/openapi/）在 is_openapi_doc_path 提前豁免，
    // 不到这里。
    if parts[0] == "openapi" {
        match parts.get(1).copied() {
            Some("usage") => return "openapi_analytics:0".to_string(),
            Some("scope-catalog")
            | Some("provider-catalog")
            | Some("outbound")
            | Some("billing")
            | Some("projection-status") => {
                return "openapi_admin:0".to_string();
            }
            _ => {}
        }
    }

    let resource_type = parts[0].replace("-", "_");
    if parts.len() == 1 || parts[1].is_empty() {
        return format!("{}:0", resource_type);
    }

    // SSO PDP requires the resource ID to be an integer. If the second path
    // segment is numeric, use it as the resource ID (sub-paths like /export
    // are ignored because NGAC policies are object-centric). If it is not
    // numeric, treat it as a sub-resource qualifier so that
    // /api/schedule/overview becomes schedule_overview:0 instead of the
    // invalid schedule:overview.
    match parts[1].parse::<i64>() {
        Ok(id) => format!("{}:{}", resource_type, id),
        Err(_) => format!("{}_{}:0", resource_type, parts[1].replace("-", "_")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_method_to_action_mapping() {
        assert_eq!(method_to_action("GET"), "read");
        assert_eq!(method_to_action("POST"), "create");
        assert_eq!(method_to_action("PUT"), "update");
        assert_eq!(method_to_action("PATCH"), "update");
        assert_eq!(method_to_action("DELETE"), "delete");
        // 未识别方法 ⇒ `access`（deny-by-default；本断言原先期望旧默认值 `read`，
        // 属预存在过时断言——`action_for_method` 语义早已收敛为 `access`，2026-10-10 随分缝修正）。
        assert_eq!(method_to_action("OPTIONS"), "access");
    }

    #[test]
    fn test_map_resource_schemas_list() {
        let resource = map_resource("/api/schemas");
        assert_eq!(resource, "schemas:0");
    }

    #[test]
    fn test_map_resource_schema_get() {
        let resource = map_resource("/api/schemas/123e4567-e89b-12d3-a456-426614174000");
        assert_eq!(resource, "schemas_123e4567_e89b_12d3_a456_426614174000:0");
    }

    #[test]
    fn test_map_resource_schema_export() {
        let resource = map_resource("/api/schemas/123e4567-e89b-12d3-a456-426614174000/export");
        assert_eq!(resource, "schemas_123e4567_e89b_12d3_a456_426614174000:0");
    }

    #[test]
    fn test_map_resource_collections() {
        let resource = map_resource("/api/collections");
        assert_eq!(resource, "collections:0");
    }

    #[test]
    fn test_map_resource_collection_item() {
        let resource = map_resource("/api/collections/abc-123");
        assert_eq!(resource, "collections_abc_123:0");
    }

    #[test]
    fn test_map_resource_product() {
        let resource = map_resource("/api/product");
        assert_eq!(resource, "product:0");
    }

    #[test]
    fn test_map_resource_product_item() {
        let resource = map_resource("/api/product/42");
        assert_eq!(resource, "product:42");
    }

    #[test]
    fn test_map_resource_datasource() {
        let resource = map_resource("/api/data-sources");
        assert_eq!(resource, "data_sources:0");
    }

    #[test]
    fn test_map_resource_non_schema() {
        let resource = map_resource("/api/other/path");
        assert_eq!(resource, "other_path:0");
    }

    #[test]
    fn test_map_resource_workspace_overview() {
        let resource = map_resource("/api/schedule/overview");
        assert_eq!(resource, "schedule_overview:0");

        let resource = map_resource("/api/global/overview");
        assert_eq!(resource, "global_overview:0");
    }
}
