//! OAS3 文档 scope 标注与 PEP 推导的一致性对账（scope-grammar-single-source）。
//!
//! 逐 ns 读取 `Pre-Proc/{ns}/openapi.json`，对每个 path × method 断言：
//! 文档标注 scope == PEP 所需 scope，两端口**同源消费**（MUST NOT 各自复刻规则）：
//! - 路径 → 实体名：`common::openapi_scope::service_entity_from_path`
//!   （与 `pep/middleware.rs` `resolved_for_scope` 的 service 路径 fallback 同函数）；
//! - 方法 → action：`ngac_contract::action_for_method`；
//! - scope 字符串构造：`common::openapi_scope::required_scope`。
//!
//! 文件驱动、无 DB；产物缺失完整性由 `check-openapi-doc-drift` 门禁负责，
//! 本测试只对已存在的产物逐条对账（至少一份数据面必须存在）。

use std::collections::BTreeMap;
use std::path::PathBuf;

/// PEP 所需 scope（单源消费：派生与字符串构造都走共享实现；产物路径须 service 形）。
fn pep_required_scope(path: &str, method: &str) -> String {
    let entity = common::openapi_scope::service_entity_from_path(path)
        .unwrap_or_else(|| panic!("产物路径 MUST 为 /api/service/* 形: {path}"));
    common::openapi_scope::required_scope(ngac_contract::action_for_method(method), &entity)
}

fn artifacts() -> Vec<PathBuf> {
    // 锚定 crate 根（编译期常量，免疫测试二进制 cwd 差异）
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut found: Vec<PathBuf> = Vec::new();
    for root in [
        crate_root.join("../../Pre-Proc"),
        crate_root.join("../../../Pre-Proc"),
    ] {
        let dir = root;
        if dir.join("Alioth").exists() || dir.join("WZ").exists() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    let p = e.path().join("openapi.json");
                    if p.is_file() {
                        found.push(p);
                    }
                }
            }
            break;
        }
    }
    found.sort();
    found
}

#[test]
fn openapi_doc_scopes_match_pep_derivation() {
    let files = artifacts();
    assert!(
        !files.is_empty(),
        "未找到任何 Pre-Proc/*/openapi.json（产物完整性由 check-openapi-doc-drift 门禁负责）"
    );
    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for file in &files {
        let ns = file
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let text = std::fs::read_to_string(file).unwrap_or_else(|e| {
            panic!("读取 {} 失败: {e}", file.display());
        });
        let doc: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("解析 {} 失败: {e}", file.display()));
        let empty = serde_json::Map::new();
        let paths = doc["paths"].as_object().unwrap_or(&empty);
        for (path, ops) in paths.iter() {
            let ops = ops.as_object().unwrap_or(&empty);
            for (method, op) in ops.iter() {
                if !matches!(method.as_str(), "get" | "post" | "put" | "patch" | "delete") {
                    continue;
                }
                let expected = pep_required_scope(path, method);
                let actual = op["security"][0]["ssoOAuth2"][0].as_str();
                match actual {
                    Some(s) if s == expected => checked += 1,
                    Some(s) => failures.push(format!(
                        "{ns} {method} {path}: 文档标注 {s:?} ≠ PEP 推导 {expected:?}"
                    )),
                    None => failures.push(format!(
                        "{ns} {method} {path}: 缺 security[0].ssoOAuth2[0] 标注（期望 {expected:?}）"
                    )),
                }
            }
        }
    }

    assert!(checked > 0, "对账面为空（paths 解析异常）");
    assert!(
        failures.is_empty(),
        "文档 scope 与 PEP 推导漂移（{} 处，前 20）：\n{}",
        failures.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// 无参数化：BTreeMap 引入仅为路径遍历的确定性排序（paths 对象键序在断言输出中稳定）。
#[test]
fn pep_derivation_reference_matrix() {
    let cases: BTreeMap<(&str, &str), &str> = [
        (
            ("/api/service/stock-ledgers", "get"),
            "read:service_stock_ledgers",
        ),
        (
            ("/api/service/stock-ledgers", "post"),
            "create:service_stock_ledgers",
        ),
        (
            ("/api/service/ns/contract-matters/42", "get"),
            "read:contract_matters",
        ),
        (
            ("/api/service/ns/contract-matters/42", "put"),
            "update:contract_matters",
        ),
        (
            ("/api/service/ns/contract-matters/42", "delete"),
            "delete:contract_matters",
        ),
        (
            ("/api/service/ns/contract-matters", "post"),
            "create:contract_matters",
        ),
        (
            (
                "/api/service/transport-dispatch/consignments/7/accept",
                "post",
            ),
            "create:consignments",
        ),
        (
            (
                "/api/service/transport-operations/quality/evaluations",
                "get",
            ),
            "read:quality",
        ),
        (
            (
                "/api/service/transport-operations/quality/evaluations",
                "post",
            ),
            "create:quality",
        ),
    ]
    .into_iter()
    .collect();
    for ((path, method), expected) in cases {
        assert_eq!(
            pep_required_scope(path, method),
            expected,
            "{method} {path}"
        );
    }
}
