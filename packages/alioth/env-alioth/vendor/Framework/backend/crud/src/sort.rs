//! 排序查询模块
//!
//! 提供标准化的排序字段和 ORDER BY 子句构建

use serde::Deserialize;

/// 排序条件
#[derive(Debug, Clone, Deserialize)]
pub struct Sort {
    pub field: String,
    #[serde(default = "default_order")]
    pub order: String,
}

fn default_order() -> String {
    "desc".to_string()
}

impl Sort {
    /// 验证字段名和排序方向是否合法
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.field.is_empty()
            || self.field.len() > 63
            || self.field.contains('\0')
            || self.field.contains(';')
            || self.field.contains("--")
            || self.field.contains("/*")
        {
            return Err("Invalid sort field");
        }
        let first = self.field.chars().next().unwrap();
        if !first.is_ascii_lowercase() && first != '_' {
            return Err("Invalid sort field");
        }
        if !self
            .field
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err("Invalid sort field");
        }
        let order_lower = self.order.to_lowercase();
        if order_lower != "asc" && order_lower != "desc" {
            return Err("Invalid sort order");
        }
        Ok(())
    }

    /// 生成 SQL ORDER BY 子句
    pub fn to_sql(&self) -> String {
        let order = self.order.to_lowercase();
        format!("{} {}", self.field, order)
    }
}
