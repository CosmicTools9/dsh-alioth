/// 判断字符串是否是正则表达式查询
/// 约定：`/pattern/` — 以 `/` 开头和结尾，中间不含 `/`
pub fn is_regex_pattern(query: &str) -> bool {
    if query.len() <= 2 || !query.starts_with('/') || !query.ends_with('/') {
        return false;
    }
    let inner = &query[1..query.len() - 1];
    !inner.contains('/')
}

/// 预处理查询字符串为 ILIKE 模式：
/// - 普通查询：`%query%`
/// - 正则查询（`/pattern/`）：只提取 pattern 做宽松 ILIKE，前端 `matchesQuery` 精确过滤
pub fn prepare_ilike_value(query: &str) -> String {
    if is_regex_pattern(query) {
        format!("%{}%", &query[1..query.len() - 1])
    } else {
        format!("%{}%", query)
    }
}

/// 生成 name/search_col ILIKE 查询的绑定值（处理正则需要）
pub fn prepare_like_value(query: &str) -> String {
    if is_regex_pattern(query) {
        format!("%{}%", &query[1..query.len() - 1])
    } else {
        format!("%{}%", query)
    }
}
