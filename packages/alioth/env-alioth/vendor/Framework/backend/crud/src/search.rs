//! KeywordSearchable trait — 关键词搜索支持
//!
//! 实体实现此 trait 可声明支持关键词搜索的文本列。
//! QueryBuilder 和 GenericRepository 据此提供通用的 search 方法。

use crate::entity::AliothDbEntity;

/// 关键词搜索 trait
pub trait KeywordSearchable: AliothDbEntity {
    /// 搜索列列表（如 `["notice", "code", "comments"]`）
    const SEARCH_COLUMNS: &'static [&'static str];
}
