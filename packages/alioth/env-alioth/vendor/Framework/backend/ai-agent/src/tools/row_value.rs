//! 行值 → JSON 类型分派解码
//!
//! LLM 工具执行 SQL 后，把 `PgRow` 按列的实际 DB 类型解码为 JSON，
//! 而非一刀切 `try_get::<Option<String>>`（那会把 INT8/NUMERIC/时间列全部
//! 变成 null 或字符串，LLM 拿到的结果失真）。
//!
//! 分派依据 `Column::type_info().name()`（PostgreSQL 内部类型名，大写 OID 名，
//! 如 `INT8` / `NUMERIC` / `TIMESTAMPTZ` / `_TEXT`），未知类型降级为字符串；
//! 解码失败返回 `Null`（与历史行为一致，不报错）。

use serde_json::{json, Value};
use sqlx::{postgres::PgRow, Column, Row, TypeInfo};

/// 列类型分类 — 驱动解码分派（纯函数，便于单测锁定分派表）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColKind {
    Int,
    Float,
    Decimal,
    Bool,
    Timestamptz,
    Timestamp,
    Date,
    Time,
    Json,
    Uuid,
    Array,
    Text,
}

/// 把 PostgreSQL OID 类型名（大写，如 `INT8`/`NUMERIC`/`_TEXT`）映射为分类。
///
/// 已知局限：sqlx 对部分列返回的 type name 未必是规范大写 OID 名
/// （例如个别环境下的 `double precision` 小写带空格）。无法识别的名字
/// 降级为 `ColKind::Text`，对应列会以字符串呈现——可接受的降级，
/// 不丢数据，仅丢类型。
fn classify(ty: &str) -> ColKind {
    match ty {
        "INT2" | "INT4" | "INT8" | "SERIAL" | "BIGSERIAL" | "SMALLSERIAL" => ColKind::Int,
        "FLOAT4" | "FLOAT8" | "REAL" | "DOUBLE PRECISION" => ColKind::Float,
        "NUMERIC" | "DECIMAL" | "MONEY" => ColKind::Decimal,
        "BOOL" | "BOOLEAN" => ColKind::Bool,
        "TIMESTAMPTZ" => ColKind::Timestamptz,
        "TIMESTAMP" => ColKind::Timestamp,
        "DATE" => ColKind::Date,
        "TIME" | "TIMETZ" => ColKind::Time,
        "JSON" | "JSONB" => ColKind::Json,
        "UUID" => ColKind::Uuid,
        _ if ty.starts_with('_') => ColKind::Array,
        // VARCHAR/BPCHAR/TEXT/NAME/CHAR/UNKNOWN 等一律文本
        _ => ColKind::Text,
    }
}

/// 把 `PgRow` 第 `idx` 列解码为 JSON 值。
pub fn row_value_to_json(row: &PgRow, idx: usize) -> Value {
    let ty = row.column(idx).type_info().name();

    // NULL 优先；类型不匹配则继续尝试下一个候选
    macro_rules! try_opt {
        ($t:ty) => {
            match row.try_get::<Option<$t>, _>(idx) {
                Ok(Some(v)) => return json!(v),
                Ok(None) => return Value::Null,
                Err(_) => {}
            }
        };
    }

    match classify(ty) {
        // 整数族 — 统一 i64
        ColKind::Int => try_opt!(i64),
        // 浮点族
        ColKind::Float => try_opt!(f64),
        // 定点数（NUMERIC/DECIMAL）— 保留精度，序列化为字符串
        ColKind::Decimal => match row.try_get::<Option<sqlx::types::Decimal>, _>(idx) {
            Ok(Some(d)) => return json!(d.to_string()),
            Ok(None) => return Value::Null,
            Err(_) => {}
        },
        // 布尔
        ColKind::Bool => try_opt!(bool),
        // 带时区时间戳 — RFC3339 字符串
        ColKind::Timestamptz => try_opt!(chrono::DateTime<chrono::Utc>),
        // 不带时区时间戳 — ISO 字符串
        ColKind::Timestamp => {
            if let Ok(v) = row.try_get::<Option<chrono::NaiveDateTime>, _>(idx) {
                return match v {
                    Some(dt) => json!(dt.format("%Y-%m-%dT%H:%M:%S").to_string()),
                    None => Value::Null,
                };
            }
        }
        ColKind::Date => try_opt!(chrono::NaiveDate),
        ColKind::Time => try_opt!(chrono::NaiveTime),
        // JSON/JSONB — 原样解析
        ColKind::Json => match row.try_get::<Option<Value>, _>(idx) {
            Ok(Some(v)) => return v,
            Ok(None) => return Value::Null,
            Err(_) => {}
        },
        // UUID — 字符串形态
        ColKind::Uuid => match row.try_get::<Option<sqlx::types::Uuid>, _>(idx) {
            Ok(Some(u)) => return json!(u.to_string()),
            Ok(None) => return Value::Null,
            Err(_) => {}
        },
        // 数组（_INT8 / _TEXT 等）— LLM 友好的 debug 形态
        ColKind::Array => {
            if let Ok(Some(v)) = row.try_get::<Option<Vec<i64>>, _>(idx) {
                return json!(format!("{:?}", v));
            }
            if let Ok(Some(v)) = row.try_get::<Option<Vec<String>>, _>(idx) {
                return json!(format!("{:?}", v));
            }
            if let Ok(Some(v)) = row.try_get::<Option<Vec<sqlx::types::Decimal>>, _>(idx) {
                return json!(format!("{:?}", v));
            }
        }
        // 文本族 + 未知类型 — 字符串
        ColKind::Text => try_opt!(String),
    }

    // 兜底：字符串；仍失败则 Null（与历史 `try_get(i).ok()` 一致）
    match row.try_get::<Option<String>, _>(idx) {
        Ok(Some(s)) => Value::String(s),
        Ok(None) => Value::Null,
        Err(_) => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_integer_family() {
        for ty in ["INT2", "INT4", "INT8", "SERIAL", "BIGSERIAL"] {
            assert_eq!(classify(ty), ColKind::Int, "{ty}");
        }
    }

    #[test]
    fn classify_float_family() {
        for ty in ["FLOAT4", "FLOAT8", "REAL", "DOUBLE PRECISION"] {
            assert_eq!(classify(ty), ColKind::Float, "{ty}");
        }
    }

    #[test]
    fn classify_decimal_family() {
        for ty in ["NUMERIC", "DECIMAL", "MONEY"] {
            assert_eq!(classify(ty), ColKind::Decimal, "{ty}");
        }
    }

    #[test]
    fn classify_bool() {
        assert_eq!(classify("BOOL"), ColKind::Bool);
        assert_eq!(classify("BOOLEAN"), ColKind::Bool);
    }

    #[test]
    fn classify_temporal_family() {
        assert_eq!(classify("TIMESTAMPTZ"), ColKind::Timestamptz);
        assert_eq!(classify("TIMESTAMP"), ColKind::Timestamp);
        assert_eq!(classify("DATE"), ColKind::Date);
        assert_eq!(classify("TIME"), ColKind::Time);
        assert_eq!(classify("TIMETZ"), ColKind::Time);
    }

    #[test]
    fn classify_json_and_uuid() {
        assert_eq!(classify("JSON"), ColKind::Json);
        assert_eq!(classify("JSONB"), ColKind::Json);
        assert_eq!(classify("UUID"), ColKind::Uuid);
    }

    #[test]
    fn classify_array_prefix() {
        assert_eq!(classify("_INT8"), ColKind::Array);
        assert_eq!(classify("_TEXT"), ColKind::Array);
        assert_eq!(classify("_NUMERIC"), ColKind::Array);
    }

    #[test]
    fn classify_text_family_and_unknown() {
        for ty in ["TEXT", "VARCHAR", "BPCHAR", "NAME", "CHAR", "UNKNOWN", ""] {
            assert_eq!(classify(ty), ColKind::Text, "{ty:?}");
        }
    }
}
