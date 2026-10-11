//! 跨语言一致性语料回放（change `add-form-contract-seam`，design D5 / task 2.2）
//!
//! 语料由**生成侧**（Zod 侧）产出：`bun scripts/gen-form-contract-corpus.ts`
//! → `tests/fixtures/form-contract-corpus.json`。本测试在 Rust 侧逐条复现同一 `(contract, values)`
//! 判定，与生成侧记录的 `expectedOk` 对等断言——**两解释器分歧即红**。
//!
//! 诚实边界：`z.toJSONSchema` 不表达 `.refine()`/`.superRefine()`，故这些用例在语料里以
//! `expectParity: false` 显式豁免对等，改为断言 Rust 侧把它们记入 `uncovered`（未受覆盖），
//! MUST NOT 被表述为「已通过」。

use ai_agent::tools::form_contract::{validate_against_contract, FormContract};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct Corpus {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    contract: Option<FormContract>,
    values: Value,
    #[serde(rename = "expectedOk")]
    expected_ok: bool,
    #[serde(rename = "expectParity")]
    expect_parity: bool,
    #[serde(rename = "expectedUncovered", default)]
    expected_uncovered: Vec<String>,
}

fn load() -> Corpus {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/form-contract-corpus.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读取语料失败 {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("语料 MUST 可解析")
}

#[test]
fn corpus_is_present_and_non_empty() {
    let corpus = load();
    assert!(
        corpus.cases.len() >= 10,
        "语料应覆盖类型/必填/枚举/范围/长度/pattern/refine/dto 等面，实际 {} 例",
        corpus.cases.len()
    );
}

#[test]
fn zod_and_rust_interpreters_agree_on_declared_parity_cases() {
    let corpus = load();
    let mut checked = 0usize;
    for case in &corpus.cases {
        if !case.expect_parity {
            continue;
        }
        let contract = case
            .contract
            .as_ref()
            .unwrap_or_else(|| panic!("[{}] 要求对等的用例 MUST 带契约", case.name));
        let out = validate_against_contract(contract, &case.values);
        let rust_ok = out.errors.is_empty();
        assert_eq!(
            rust_ok, case.expected_ok,
            "[{}] 两解释器判定分歧：Zod={} Rust={}（错误：{:?}）",
            case.name, case.expected_ok, rust_ok, out.errors
        );
        checked += 1;
    }
    assert!(checked >= 8, "对等用例过少（{checked}），语料可能退化");
}

#[test]
fn non_portable_cases_are_reported_as_uncovered_not_as_passed() {
    let corpus = load();
    let mut checked = 0usize;
    for case in &corpus.cases {
        if case.expect_parity {
            continue;
        }
        let contract = case.contract.as_ref().expect("豁免对等的用例 MUST 带契约");
        let out = validate_against_contract(contract, &case.values);
        assert!(
            out.errors.is_empty(),
            "[{}] 不可派生规则 MUST NOT 产生值错误（否则是猜规则）",
            case.name
        );
        assert_eq!(
            out.uncovered, case.expected_uncovered,
            "[{}] 未覆盖字段集合不符",
            case.name
        );
        assert!(
            !out.appliable,
            "[{}] 存在未受覆盖字段 ⇒ 结果 MUST NOT 可应用",
            case.name
        );
        checked += 1;
    }
    assert!(checked >= 1, "语料 MUST 含 refine（nonPortable）用例");
}

#[test]
fn dto_metadata_survives_the_wire() {
    let corpus = load();
    let case = corpus
        .cases
        .iter()
        .find(|c| c.name == "dto-metadata-carried")
        .expect("语料 MUST 含 dto 元数据用例");
    let contract = case.contract.as_ref().unwrap();
    let by_name = |n: &str| {
        contract
            .fields
            .iter()
            .find(|f| f.name == n)
            .expect("字段存在")
    };

    let cp = by_name("counterpartyId");
    let dto = cp.dto.as_ref().expect("m2o 字段 MUST 带 dto");
    assert_eq!(dto.kind.as_deref(), Some("m2o"));
    assert_eq!(dto.label_field.as_deref(), Some("notice"));

    let w = by_name("weight");
    assert_eq!(w.dto.as_ref().unwrap().kind.as_deref(), Some("scalar"));
}
