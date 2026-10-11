//! 关键词检索谓词单源。
//!
//! 声明面 = [`AliothDbEntity::SEARCH_COLUMNS`]；值预处理与转义 = `common::search`。
//! 本模块只负责把「列 + 关键词」组装成一段安全的 SQL 谓词——SQL 面实现只此一处，
//! 其他读径（含服务自建仓储）MUST 复用，MUST NOT 内联拼接 `'%' || $1 || '%'`
//! 或自行内联转义。

use crate::entity::AliothDbEntity;
use common::search::like_pattern;

/// 构造检索谓词：`(<col>::text ILIKE '<pat>' ESCAPE '\' OR …)`。
///
/// `columns` 为空或关键词 trim 后为空 ⇒ `None`（调用方不追加谓词，行为与历史一致）。
/// 列名来自实体 `&'static [&'static str]` 声明（编译期常量）⇒ 不作转义；
/// 关键词经 `like_pattern` 转义（`\` `%` `_`），另将 `'` 转义为 `''`（本处是字面量 SQL）。
pub fn search_predicate(columns: &[&str], raw: &str) -> Option<String> {
    let query = raw.trim();
    if columns.is_empty() || query.is_empty() {
        return None;
    }
    let pattern = like_pattern(query).replace('\'', "''");
    let conditions: Vec<String> = columns
        .iter()
        .map(|col| format!("{col}::text ILIKE '{pattern}' ESCAPE '\\'"))
        .collect();
    Some(format!("({})", conditions.join(" OR ")))
}

/// 实体声明（`E::SEARCH_COLUMNS`）驱动的检索谓词。
pub fn entity_search_predicate<E: AliothDbEntity>(raw: &str) -> Option<String> {
    search_predicate(E::SEARCH_COLUMNS, raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_inputs_yield_none() {
        assert!(search_predicate(&["notice"], "   ").is_none());
        assert!(search_predicate(&[], "abc").is_none());
    }

    #[test]
    fn predicate_ors_columns_and_escapes_wildcards() {
        let p = search_predicate(&["notice", "code"], "100%").unwrap();
        assert_eq!(
            p,
            "(notice::text ILIKE '%100\\%%' ESCAPE '\\' OR code::text ILIKE '%100\\%%' ESCAPE '\\')"
        );
    }

    #[test]
    fn underscore_is_literal_and_quote_is_escaped() {
        let p = search_predicate(&["notice"], "o'brien_1").unwrap();
        assert!(p.contains("'%o''brien\\_1%'"), "got {p}");
    }

    #[test]
    fn regex_wrapper_is_stripped_before_escaping() {
        let p = search_predicate(&["notice"], "/a_b/").unwrap();
        assert!(p.contains("'%a\\_b%'"), "got {p}");
    }
}
