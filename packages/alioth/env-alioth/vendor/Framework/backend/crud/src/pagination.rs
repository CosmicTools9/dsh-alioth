//! 分页模块
//!
//! `ListQuery` 与 `PaginatedResponse<T>` 已上移至 `alioth-common`。
//! 此模块提供兼容性重新导出，以及 CRUD 特有的过滤/排序转换工具。

use crate::filter::Filter;
use crate::sort::Sort;

// 从 alioth-common 重新导出分页与查询类型
pub use common::{ListQuery, PaginatedResponse};

/// `ListQuery` 的 CRUD 扩展 trait
///
/// 为 `common::ListQuery` 补充 `to_filter` / `to_sort` 方法，
/// 使现有代码 `query.to_filter()` 无需修改即可继续工作。
pub trait ListQueryExt {
    fn to_filter(&self) -> Option<Filter>;
    fn to_sort(&self) -> Option<Sort>;
}

impl ListQueryExt for ListQuery {
    fn to_filter(&self) -> Option<Filter> {
        match (&self.filter_field, &self.filter_op, &self.filter_value) {
            (Some(f), Some(o), Some(v)) => Some(Filter {
                field: f.clone(),
                op: o.clone(),
                value: v.clone(),
            }),
            _ => None,
        }
    }

    fn to_sort(&self) -> Option<Sort> {
        self.sort_field.as_ref().map(|f| Sort {
            field: f.clone(),
            order: self
                .sort_order
                .clone()
                .unwrap_or_else(|| "desc".to_string()),
        })
    }
}
