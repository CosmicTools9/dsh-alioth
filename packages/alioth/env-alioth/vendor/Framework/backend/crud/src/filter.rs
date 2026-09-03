//! 过滤查询模块
//!
//! 提供标准化的过滤条件和 WHERE 子句构建

use serde::Deserialize;

/// 单个过滤条件
#[derive(Debug, Clone, Deserialize)]
pub struct Filter {
    pub field: String,
    pub op: String,
    pub value: String,
}

impl Filter {
    /// 验证字段名和运算符是否合法
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.field.is_empty()
            || self.field.len() > 63
            || self.field.contains('\0')
            || self.field.contains(';')
            || self.field.contains("--")
            || self.field.contains("/*")
        {
            return Err("Invalid filter field");
        }
        // field must start with a-z or _
        let first = self.field.chars().next().unwrap();
        if !first.is_ascii_lowercase() && first != '_' {
            return Err("Invalid filter field");
        }
        // remaining chars: a-z, 0-9, _, -
        // 允许连字符以支持 Alioth 物理列名（如 _t_、_f_）
        if !self
            .field
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err("Invalid filter field");
        }
        let valid_ops = ["eq", "ne", "gt", "lt", "gte", "lte", "like"];
        if !valid_ops.contains(&self.op.as_str()) {
            return Err("Invalid filter op");
        }
        Ok(())
    }

    /// 生成 SQL 条件占位符字符串（如 `field = $3`）
    ///
    /// `param_idx` 是该条件在参数列表中的 1-based 索引。
    ///
    /// 无列类型信息时的保守默认：对列做 `::text` cast（与历史行为一致，
    /// 保证 `like`/`ilike` 模糊查询可用，未知列也不报错）。
    pub fn to_sql(&self, param_idx: usize) -> Option<String> {
        self.to_sql_with_type(param_idx, None)
    }

    /// 按 DB 列类型生成 SQL 条件（元数据驱动分派）。
    ///
    /// `column_type` 为 `information_schema.columns.data_type`（如 `bigint`、
    /// `timestamp with time zone`、`text`）。分派规则：
    ///
    /// - 数值列（bigint/integer/numeric/real…）→ 对**参数** cast（`field op $n::bigint`），
    ///   保持数值语义，避免 `::text` 把 `9 > 100` 变成字典序比较；
    /// - 时间列 → `$n::timestamptz`，日期/时间字符串由 PG 解析；
    /// - 布尔列 → `$n::boolean`；
    /// - 文本列 / 未知列 / `like` 操作 → 保留 `field::text op $n`（模糊查询依赖）。
    pub fn to_sql_with_type(&self, param_idx: usize, column_type: Option<&str>) -> Option<String> {
        let op = match self.op.as_str() {
            "eq" => "=",
            "ne" => "!=",
            "gt" => ">",
            "lt" => "<",
            "gte" => ">=",
            "lte" => "<=",
            "like" => "LIKE",
            _ => return None,
        };
        // Alioth 物理列名含连字符（如 _t_、_f_）须加双引号
        let field = if self.field.contains('-') {
            format!(r#""{}""#, self.field)
        } else {
            self.field.clone()
        };
        // like/ilike 只在文本语义下成立 → 一律走列 ::text
        if self.op == "like" {
            return Some(format!("{}::text LIKE ${}", field, param_idx));
        }
        match column_type.map(|t| t.to_ascii_lowercase()).as_deref() {
            Some(t)
                if t.contains("bigint")
                    || t.contains("integer")
                    || t.contains("smallint")
                    || t.contains("serial") =>
            {
                Some(format!("{} {} ${}::bigint", field, op, param_idx))
            }
            Some(t)
                if t.contains("numeric")
                    || t.contains("decimal")
                    || t.contains("real")
                    || t.contains("double") =>
            {
                Some(format!("{} {} ${}::numeric", field, op, param_idx))
            }
            Some(t) if t.contains("timestamp with time zone") || t.contains("timestamptz") => {
                Some(format!("{} {} ${}::timestamptz", field, op, param_idx))
            }
            Some(t) if t.contains("timestamp") => {
                Some(format!("{} {} ${}::timestamp", field, op, param_idx))
            }
            Some(t) if t.contains("time with time zone") || t.contains("timetz") => {
                Some(format!("{} {} ${}::timetz", field, op, param_idx))
            }
            Some(t) if t.contains("time") => Some(format!("{} {} ${}::time", field, op, param_idx)),
            Some(t) if t.contains("date") => Some(format!("{} {} ${}::date", field, op, param_idx)),
            Some(t) if t.contains("bool") => {
                Some(format!("{} {} ${}::boolean", field, op, param_idx))
            }
            // text/varchar/char/json/jsonb/uuid/array/未知 → 列 ::text（历史行为）
            _ => Some(format!("{}::text {} ${}", field, op, param_idx)),
        }
    }
}
