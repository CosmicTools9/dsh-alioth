//! OpenAPI scope 文法（openapi-external-access :: scope-grammar-single-source）。
//!
//! scope 字符串宇宙的**唯一文法实现**：
//! - 端点所需 scope = `{action}:{entity}`（action ∈ [`ACTIONS`]，entity 为
//!   PEP `map_resource` 对 `/api/service/{svc}/{segment}` 路径段 `-`→`_` 后的形态）；
//! - 通配：`*`（全通配，仅管理面经 NGAC `openapi_admin:0` 可授）与
//!   `*:{entity}`（按实体全动作）。
//!
//! 消费方（三方同源，MUST NOT 另起第二实现）：
//! - Gateway PEP scope 覆盖判定（`Gateway/backend/src/pep/middleware.rs`）——
//!   路径→实体名派生 = [`service_entity_from_path`]（与文档对账测试共用）；
//! - SSO 签发面创建/更新校验（`api_clients_common` / `portal` / `admin`）；
//! - OAS3 文档 conformance 测试（`Gateway/backend/tests/openapi_scope_conformance_test.rs`，
//!   断言文档逐端点标注 == PEP 推导；同源消费 [`service_entity_from_path`] 与
//!   `ngac_contract::action_for_method`）。

/// 动作全集（与 `ngac_contract::action_for_method` 映射一致：GET=read、
/// POST=create、PUT|PATCH=update、DELETE=delete）。
pub const ACTIONS: [&str; 4] = ["read", "create", "update", "delete"];

fn valid_entity(e: &str) -> bool {
    !e.is_empty()
        && e.len() <= 128
        && e.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// 合法 scope 形：`{action}:{entity}` / `*:{entity}` / `*`。
///
/// entity 限定 `[a-z0-9_]+`（与 PEP 路径段推导产物一致；连字符/大写/
/// 空串均为文法外值——签发面 MUST 拒绝）。
pub fn is_valid_scope(s: &str) -> bool {
    let s = s.trim();
    if s == "*" {
        return true;
    }
    match s.split_once(':') {
        Some((action, entity)) => {
            (action == "*" || ACTIONS.contains(&action)) && valid_entity(entity)
        }
        None => false,
    }
}

/// 通配形态（`*` / `*:{entity}`）——门户自助面 MUST NOT 自授（纵深防御：
/// 自助键 + 自声明通配 + bootstrap fail-open 的三链叠加封口）。
pub fn is_wildcard_scope(s: &str) -> bool {
    let s = s.trim();
    s == "*" || s.starts_with("*:")
}

/// 端点所需 scope：`{action}:{resource_type}`（resource_type 已是下划线形态）。
pub fn required_scope(action: &str, resource_type: &str) -> String {
    format!("{}:{}", action, resource_type)
}

/// service 路径 → scope 实体名派生（**PEP 路径 fallback 与文档对账测试的单源**，
/// change `fix-openapi-review-findings` §3——MUST NOT 各自复刻规则）。
///
/// - `/api/service/{svc}/{entity}[/...]` ⇒ `{entity}`（`-` → `_`）；
/// - `/api/service/{svc}`（服务根）⇒ `service_{svc}`（`-` → `_`）；
/// - 非 service 路径 / 结构不足 ⇒ `None`（调用方自行处理：PEP 继续走
///   `map_resource` fallback；对账测试对产物路径有 service 形前置断言）。
pub fn service_entity_from_path(path: &str) -> Option<String> {
    let stripped = path.trim_start_matches("/api/").trim_start_matches('/');
    let parts: Vec<&str> = stripped.split('/').collect();
    if parts.first() != Some(&"service") {
        return None;
    }
    if let Some(entity) = parts.get(2).filter(|s| !s.is_empty()) {
        return Some(entity.replace('-', "_"));
    }
    parts
        .get(1)
        .filter(|s| !s.is_empty())
        .map(|svc| format!("service_{}", svc.replace('-', "_")))
}

/// 单条已授 scope 是否覆盖所需 scope（PEP 判定等价实现：
/// `s == required || s == "*:{rt}" || s == "*"`）。
pub fn covers(granted: &str, required: &str) -> bool {
    let g = granted.trim();
    if g.is_empty() {
        return false;
    }
    if g == "*" {
        return true;
    }
    let (g_action, g_entity) = match g.split_once(':') {
        Some(parts) => parts,
        None => return false,
    };
    if g_action != "*" {
        return g == required.trim();
    }
    // `*:{entity}`：按实体全动作覆盖。
    required
        .trim()
        .split_once(':')
        .map(|(_, r_entity)| r_entity == g_entity)
        .unwrap_or(false)
}

/// 多 scope（空白分隔，JWT `scope` claim 形态）是否覆盖所需 scope。
pub fn covers_any(granted_scopes: &str, required: &str) -> bool {
    granted_scopes
        .split_whitespace()
        .any(|s| covers(s, required))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_scope_forms() {
        for s in [
            "read:contract",
            "create:stock_ledgers",
            "update:transport_tracking",
            "delete:configs",
            "*:contract",
            "*",
        ] {
            assert!(is_valid_scope(s), "应合法: {s}");
        }
    }

    #[test]
    fn invalid_scope_forms() {
        for s in [
            "read",                // 缺实体
            "",                    // 空
            "READ:X",              // 动作大写
            "write:contract",      // 动作不在全集
            "read:contract-x",     // 实体含连字符（PEP 推导产物是下划线）
            "read:Contract",       // 实体大写
            "read:approval-flows", // 连字符实体（seed 实测样本）
            "read:",               // 空实体
            " :contract",          // 空动作
            "read:contract:extra", // 三段
            "*:",                  // 空实体通配
            "read:contract ",      // 尾随空格按 trim 后合法——见下
        ] {
            if s == "read:contract " {
                assert!(is_valid_scope(s), "尾随空白应 trim 后合法");
            } else {
                assert!(!is_valid_scope(s), "应非法: {s:?}");
            }
        }
    }

    #[test]
    fn wildcard_detection() {
        assert!(is_wildcard_scope("*"));
        assert!(is_wildcard_scope("*:contract"));
        assert!(!is_wildcard_scope("read:contract"));
        assert!(!is_wildcard_scope("create:*"));
    }

    #[test]
    fn covers_semantics() {
        assert!(covers("read:contract", "read:contract"));
        assert!(covers("*:contract", "read:contract"));
        assert!(covers("*", "delete:anything"));
        assert!(!covers("read:contract", "create:contract"));
        assert!(!covers("read:contract", "read:contracts"));
        assert!(!covers("*:contract", "read:contracts"));
        assert!(!covers("", "read:contract"));
        assert!(!covers("read:contract:extra", "read:contract"));
    }

    #[test]
    fn covers_any_matches_pep_inline_semantics() {
        // 旧 PEP 内联：any(s == required || s == format!("*:{rt}") || s == "*")
        let required = "read:contract";
        assert!(covers_any("read:invoice read:contract", required));
        assert!(covers_any("*:contract", required));
        assert!(covers_any("read:invoice *", required));
        assert!(!covers_any("read:invoice write:contract", required));
        assert!(!covers_any("", required));
        assert!(!covers_any("   ", required));
    }

    #[test]
    fn required_scope_format() {
        assert_eq!(
            required_scope("read", "stock_ledgers"),
            "read:stock_ledgers"
        );
    }
}
