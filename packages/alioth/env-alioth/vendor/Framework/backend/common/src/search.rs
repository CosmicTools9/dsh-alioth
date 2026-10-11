//! 关键词检索的值预处理与转义单源。
//!
//! 职责边界（唯一实现，其他读径 MUST 复用）：
//! - [`is_regex_pattern`] / [`strip_regex_wrapper`] —— `/pattern/` 正则包裹约定
//!   （与前端 `matchesQuery` 同约定：包裹剥离后按子串匹配，精确正则过滤在前端完成）；
//! - [`escape_like`] —— LIKE 元字符（`\` `%` `_`）转义原语，使输入按**字面量**匹配；
//! - [`like_pattern`] —— 关键词 → `%<转义>%` 绑定值（供 `ILIKE $n` 参数化路径）。
//!
//! SQL 面的谓词组装（列 OR 组合、`ESCAPE` 子句、`'` 转义）单源在
//! `crud::search::search_predicate`，其模式一律取自本模块。

/// 判断字符串是否是正则表达式查询
/// 约定：`/pattern/` — 以 `/` 开头和结尾，中间不含 `/`
pub fn is_regex_pattern(query: &str) -> bool {
    if query.len() <= 2 || !query.starts_with('/') || !query.ends_with('/') {
        return false;
    }
    let inner = &query[1..query.len() - 1];
    !inner.contains('/')
}

/// 去除正则包裹：`/pattern/` → `pattern`，其余原样。
pub fn strip_regex_wrapper(query: &str) -> &str {
    if is_regex_pattern(query) {
        &query[1..query.len() - 1]
    } else {
        query
    }
}

/// 转义 LIKE 元字符（`\` `%` `_`），使其按字面量参与匹配。
///
/// `\` 必须先行转义（否则会二次转义后续插入的反斜杠）。`'` 不在此处理——
/// 参数化路径无 SQL 字面量面；字面量 SQL 路径由调用方另行转义（见 `crud::search`）。
pub fn escape_like(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '%' => out.push_str("\\%"),
            '_' => out.push_str("\\_"),
            other => out.push(other),
        }
    }
    out
}

/// 关键词 → ILIKE 绑定值 `%<转义>%`（含正则包裹剥离）。
///
/// 调用方以 `ILIKE $n` 绑定该值即可；Postgres 的默认 LIKE 转义字符为 `\`，
/// 故 `\%` / `\_` 无需显式 `ESCAPE` 子句即按字面量匹配。
pub fn like_pattern(query: &str) -> String {
    format!("%{}%", escape_like(strip_regex_wrapper(query)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_wrapper_roundtrip() {
        assert!(is_regex_pattern("/abc/"));
        assert!(!is_regex_pattern("/a/b/"));
        assert!(!is_regex_pattern("abc"));
        assert_eq!(strip_regex_wrapper("/abc/"), "abc");
        assert_eq!(strip_regex_wrapper("abc"), "abc");
    }

    #[test]
    fn escape_like_neutralizes_metacharacters() {
        assert_eq!(escape_like("100%"), "100\\%");
        assert_eq!(escape_like("a_b"), "a\\_b");
        assert_eq!(escape_like("a\\b"), "a\\\\b");
        assert_eq!(escape_like("plain"), "plain");
    }

    #[test]
    fn like_pattern_wraps_and_escapes() {
        assert_eq!(like_pattern("abc"), "%abc%");
        assert_eq!(like_pattern("100%"), "%100\\%%");
        // 正则包裹剥离后再转义（内层文本按字面量匹配）
        assert_eq!(like_pattern("/a_b/"), "%a\\_b%");
    }
}
