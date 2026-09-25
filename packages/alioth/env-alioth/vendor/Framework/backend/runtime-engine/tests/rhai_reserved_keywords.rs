//! Rhai 保留字分类**钉住测试**（判据正本 = `scripts/check/check-expression-dialect.ts` 的 R4
//! `RHAI_RESERVED` 集）。
//!
//! 分类由**生产同款引擎**实测得出（`RhaiExpressionEngine`，rhai 1.26.1）：
//! - `RESERVED_KEYWORD`：裸名求值报 `'<word>' is a reserved keyword` ⇒ 不可作字段裸名；
//! - `RESERVED_SYNTAX`：裸名为语法错误（非 reserved 文案），但同样**不可作字段裸名**；
//! - `USABLE`：裸名报 `Variable not found` ⇒ 是可用标识符（如 `exit`），MUST NOT 入 R4 集。
//!
//! 目的：rhai 版本升级/引擎配置变更导致分类漂移时**立即失败**（R4 集与引擎行为不得脱节）。

use std::collections::HashMap;

/// 裸名报 "reserved keyword" 的词（rhai 1.26.1 实测 36 个）
const RESERVED_KEYWORD: &[&str] = &[
    "Fn",
    "async",
    "await",
    "call",
    "case",
    "curry",
    "debug",
    "default",
    "eval",
    "go",
    "goto",
    "is",
    "is_def_fn",
    "is_def_var",
    "is_shared",
    "match",
    "module",
    "new",
    "nil",
    "null",
    "package",
    "print",
    "protected",
    "public",
    "shared",
    "spawn",
    "static",
    "super",
    "sync",
    "thread",
    "type_of",
    "use",
    "var",
    "void",
    "with",
    "yield",
];

/// 裸名为语法错误、同样不可作字段裸名的词（rhai 1.26.1 实测 7 个）
const RESERVED_SYNTAX: &[&str] = &["as", "export", "fn", "import", "private", "this", "switch"];

/// 裸名为 `Variable not found` ⇒ 可用标识符（MUST NOT 入 R4 集）
const USABLE: &[&str] = &["exit", "quantity", "orderNo"];

fn evaluate_bare(word: &str) -> String {
    let engine = runtime_engine::RhaiExpressionEngine::new();
    let payload: HashMap<String, serde_json::Value> = HashMap::new();
    match engine.evaluate(word, &payload) {
        Ok(v) => format!("OK({v})"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn reserved_keyword_classification_is_pinned() {
    for word in RESERVED_KEYWORD {
        let outcome = evaluate_bare(word);
        assert!(
            outcome.contains("is a reserved keyword"),
            "'{word}' 实测应为保留字文案，实际: {outcome}——rhai 分类已漂移，R4 集需同步"
        );
    }
}

#[test]
fn reserved_syntax_classification_is_pinned() {
    for word in RESERVED_SYNTAX {
        let outcome = evaluate_bare(word);
        assert!(
            outcome.contains("Syntax error"),
            "'{word}' 实测应为语法错误（不可作字段裸名），实际: {outcome}——rhai 分类已漂移"
        );
        assert!(
            !outcome.contains("reserved keyword"),
            "'{word}' 若转为 reserved 文案，应移入 RESERVED_KEYWORD 组"
        );
    }
}

#[test]
fn usable_identifiers_are_not_in_reserved_sets() {
    for word in USABLE {
        let outcome = evaluate_bare(word);
        assert!(
            outcome.contains("Variable not found"),
            "'{word}' 实测应为可用标识符（Variable not found），实际: {outcome}——不得入 R4 集"
        );
    }
}
