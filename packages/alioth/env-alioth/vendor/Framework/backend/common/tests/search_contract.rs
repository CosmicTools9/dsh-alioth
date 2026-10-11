//! 跨语言契约：前端匹配器 ↔ 后端预筛（超集不变量 / 无假阴性）。
//!
//! 语料单源 = `Framework/frontend/components/src/components/search/search-contract-corpus.tsv`
//! （TS 侧断言见同目录 `corpus.contract.test.ts`，两侧读**同一份**文件）。
//!
//! 契约（design D2）：前端匹配器命中 ⇒ 后端预筛 MUST 接受该行。后端只做宽松预筛
//! （`/pattern/` 包裹剥离 + 按字面量转义后 ILIKE 子串），精确过滤在前端完成；
//! `rust_accepts=1 而 ts_matches=0` 合法（预筛更宽）。
//!
//! 本文件只读 `common::search`（`is_regex_pattern` / `strip_regex_wrapper` / `like_pattern`），
//! 不修改其实现。

use std::path::PathBuf;

/// 语料路径：与 TS 侧同一文件（相对 crate 根解析，运行时读取而非 include 嵌入）。
fn corpus_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../frontend/components/src/components/search/search-contract-corpus.tsv")
}

struct Row {
    line: usize,
    query: String,
    text: String,
    ts_matches: bool,
    rust_accepts: bool,
}

fn load_corpus() -> Vec<Row> {
    let path = corpus_path();
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "语料不可读（跨语言契约文件缺失则契约失守）: {} —— {e}",
            path.display()
        )
    });

    let mut rows = Vec::new();
    for (index, raw_line) in raw.split('\n').enumerate() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            4,
            "语料第 {} 行字段数={}（期望 4）: {line:?}",
            index + 1,
            fields.len()
        );
        assert!(
            fields[2] == "0" || fields[2] == "1",
            "语料第 {} 行 ts_matches 非 0/1: {:?}",
            index + 1,
            fields[2]
        );
        assert!(
            fields[3] == "0" || fields[3] == "1",
            "语料第 {} 行 rust_accepts 非 0/1: {:?}",
            index + 1,
            fields[3]
        );
        rows.push(Row {
            line: index + 1,
            query: fields[0].to_string(),
            text: fields[1].to_string(),
            ts_matches: fields[2] == "1",
            rust_accepts: fields[3] == "1",
        });
    }
    assert!(
        rows.len() >= 20,
        "语料规模异常（{} 行，期望 ≥ 20）——检查 {}",
        rows.len(),
        path.display()
    );
    rows
}

/// Postgres LIKE 默认转义符 `\` 的还原：`\%`→`%`、`\_`→`_`、`\\`→`\`。
fn unescape_like(escaped: &str) -> String {
    let mut out = String::with_capacity(escaped.len());
    let mut chars = escaped.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some(next) => out.push(next),
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// 后端预筛在 SQL 里的判定：`text ILIKE like_pattern(query)`（字面量子串，大小写不敏感）。
fn prefilter_accepts(text: &str, query: &str) -> bool {
    let pattern = common::search::like_pattern(query);
    let body = pattern
        .strip_prefix('%')
        .and_then(|p| p.strip_suffix('%'))
        .unwrap_or_else(|| panic!("like_pattern 未按 `%…%` 包裹: {pattern:?}"));
    let needle = unescape_like(body);
    // `like_pattern` 的剥离/转义必须与包裹语义单源一致（`is_regex_pattern` ⇒ 内层而非整串）。
    assert_eq!(
        needle,
        common::search::strip_regex_wrapper(query),
        "like_pattern 字面量与 strip_regex_wrapper 不一致（query={query:?}）"
    );
    text.to_lowercase().contains(&needle.to_lowercase())
}

#[test]
fn frontend_match_implies_backend_prefilter_accepts() {
    let canonical = std::fs::canonicalize(corpus_path()).unwrap_or_else(|_| corpus_path());
    println!("corpus = {}", canonical.display());

    let rows = load_corpus();
    let mut checked = 0usize;
    for row in &rows {
        let declared_offenders = row.ts_matches && !row.rust_accepts;
        assert!(
            !declared_offenders,
            "语料第 {} 行声明违反超集不变量（ts_matches=1 而 rust_accepts=0）: query={:?} text={:?}",
            row.line, row.query, row.text
        );

        let accepted = prefilter_accepts(&row.text, &row.query);
        assert_eq!(
            accepted, row.rust_accepts,
            "语料第 {} 行 rust_accepts 声明与真实预筛判定不符: query={:?} text={:?}",
            row.line, row.query, row.text
        );

        if row.ts_matches {
            assert!(
                accepted,
                "超集不变量失守（假阴性）：语料第 {} 行 query={:?} text={:?} 前端命中但后端预筛拒绝",
                row.line, row.query, row.text
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "语料无 ts_matches=1 行——契约未被实际行使");
}

/// 与 TS 侧同源证据：TS 侧读同一文件的断言由 vitest 套件执行
/// （`Framework/frontend/components` 的 `corpus.contract.test.ts`）。
#[test]
fn corpus_file_is_shared_with_ts_side() {
    let source = std::fs::read_to_string(corpus_path()).expect("语料可读");
    assert!(
        source.contains("# query\ttext\tts_matches\trust_accepts"),
        "语料表头缺失或列序变更——两侧解析契约已破"
    );
    // 反向锚点：TS 消费方的测试文件确实引用同一语料（防「两侧各写一份」回退）。
    let ts_test = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../frontend/components/src/components/search/corpus.contract.test.ts");
    let ts_source = std::fs::read_to_string(&ts_test).expect("TS 契约测试可读");
    assert!(
        ts_source.contains("search-contract-corpus.tsv?raw"),
        "TS 侧未读同一语料文件: {}",
        ts_test.display()
    );
}
