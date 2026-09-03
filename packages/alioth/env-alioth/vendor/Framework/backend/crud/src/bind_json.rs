//! 写入绑定 — 按目标列类型强转 JSON 值
//!
//! `coerce` 把前端 JSON 值按 `information_schema.columns.data_type` 转成
//! 强类型绑定（bigint → i64、numeric → f64、boolean → bool、jsonb → Value），
//! 取代旧的一刀切 JSON 形状绑定（那会把字符串 "123" 绑到 bigint 列上直接报错，
//! 或把数组/对象 to_string 成文本塞进 jsonb 列）。
//!
//! 语义（与 Meta/backend/src/table_data/mod.rs 的 bind_json 对齐）：
//! **schema-soft** — 无法强转的值静默变为 NULL（`Option::None`），不抛 500。
//! 调用方不感知；NULL 与合法值一样走 DB 约束判定。
//!
//! `data_type = None`（未知表/列）时按 JSON 形状兜底绑定，保持历史行为。

use serde_json::Value;
use sqlx::postgres::PgArguments;
use sqlx::Postgres;

/// 强转后的绑定载荷
pub enum BoundValue {
    /// 整数列（bigint/integer/smallint/serial）— None → NULL
    OptInt(Option<i64>),
    /// 数值列（numeric/real/double precision）— None → NULL
    OptFloat(Option<f64>),
    /// 布尔列 — None → NULL
    OptBool(Option<bool>),
    /// 文本/时间戳/日期/未知列 — None → NULL
    OptText(Option<String>),
    /// JSON/JSONB 列 — 原样绑定（sqlx 直接编码为 jsonb）
    JsonValue(Value),
}

fn parse_bool_str(s: &str) -> Option<bool> {
    match s.trim().to_lowercase().as_str() {
        "true" | "1" | "yes" | "是" => Some(true),
        "false" | "0" | "no" | "否" => Some(false),
        _ => None,
    }
}

/// GeoJSON object → WKT（geometry-field-types）。
///
/// 支持 Point/LineString/Polygon（RFC 7946 coordinates 数组）；不支持的类型
/// 返回 None（schema-soft 语义：无法强转 → NULL，与 `coerce` 一致）。
/// 结构化解析（serde_json），非正则。
fn geojson_to_wkt(v: &Value) -> Option<String> {
    let obj = v.as_object()?;
    let ty = obj.get("type")?.as_str()?;
    let coords = obj.get("coordinates")?;
    match ty {
        "Point" => {
            let p = coords.as_array()?;
            Some(format!("POINT({} {})", num(p.first()?)?, num(p.get(1)?)?))
        }
        "LineString" => {
            let pts: Vec<String> = coords
                .as_array()?
                .iter()
                .map(|c| {
                    let p = c.as_array()?;
                    Some(format!("{} {}", num(p.first()?)?, num(p.get(1)?)?))
                })
                .collect::<Option<_>>()?;
            Some(format!("LINESTRING({})", pts.join(", ")))
        }
        "Polygon" => {
            let rings: Vec<String> = coords
                .as_array()?
                .iter()
                .map(|ring| {
                    let pts: Vec<String> = ring
                        .as_array()?
                        .iter()
                        .map(|c| {
                            let p = c.as_array()?;
                            Some(format!("{} {}", num(p.first()?)?, num(p.get(1)?)?))
                        })
                        .collect::<Option<_>>()?;
                    Some(format!("({})", pts.join(", ")))
                })
                .collect::<Option<_>>()?;
            Some(format!("POLYGON({})", rings.join(", ")))
        }
        _ => None,
    }
}

fn num(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// 把 JSON 值按目标列类型强转。无法强转 → `None`（绑定为 NULL）。
pub fn coerce(v: &Value, data_type: Option<&str>) -> BoundValue {
    match data_type {
        Some("bigint" | "integer" | "smallint" | "smallserial" | "serial" | "bigserial") => {
            BoundValue::OptInt(
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok())),
            )
        }
        Some("numeric" | "decimal") => BoundValue::OptFloat(
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok())),
        ),
        Some("real" | "float4" | "float8" | "double precision") => BoundValue::OptFloat(
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok())),
        ),
        // PostGIS 几何列（geometry-field-types）：WKT 字符串直通（text→geometry
        // implicit cast，pg_cast 'i'）；GeoJSON object 自动转 WKT 绑定。
        // 读取侧 to_jsonb 输出 GeoJSON，读写协议对称。
        Some("geometry" | "geography") => match v {
            Value::String(s) => BoundValue::OptText(Some(s.clone())),
            Value::Object(_) => BoundValue::OptText(geojson_to_wkt(v)),
            _ => BoundValue::OptText(None),
        },
        Some("boolean" | "bool") => {
            BoundValue::OptBool(v.as_bool().or_else(|| v.as_str().and_then(parse_bool_str)))
        }
        // 时间戳/日期：文本直通（PG 按目标列 input 函数解析）；无法取字符串 → NULL
        Some(
            "timestamp with time zone"
            | "timestamp without time zone"
            | "timestamp"
            | "timestamptz"
            | "date"
            | "time with time zone"
            | "time without time zone"
            | "time",
        ) => BoundValue::OptText(v.as_str().map(String::from)),
        Some("json" | "jsonb") => BoundValue::JsonValue(v.clone()),
        // 未知列类型：按 JSON 形状兜底（历史行为）
        _ => match v {
            Value::Null => BoundValue::OptText(None),
            Value::Bool(b) => BoundValue::OptBool(Some(*b)),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    BoundValue::OptInt(Some(i))
                } else {
                    BoundValue::OptFloat(n.as_f64())
                }
            }
            Value::String(s) => BoundValue::OptText(Some(s.clone())),
            // 数组/对象按 JSON 原样绑定（对应列若非 jsonb，PG 会报类型错误；
            // 避免 to_string 成文本塞进 jsonb 列的隐式 cast 问题）
            Value::Array(_) | Value::Object(_) => BoundValue::JsonValue(v.clone()),
        },
    }
}

/// 对 `QueryAs`（返回单列 id）应用绑定
pub fn apply_query_as<'q>(
    q: sqlx::query::QueryAs<'q, Postgres, (i64,), PgArguments>,
    b: BoundValue,
) -> sqlx::query::QueryAs<'q, Postgres, (i64,), PgArguments> {
    match b {
        BoundValue::OptInt(v) => q.bind(v),
        BoundValue::OptFloat(v) => q.bind(v),
        BoundValue::OptBool(v) => q.bind(v),
        BoundValue::OptText(v) => q.bind(v),
        BoundValue::JsonValue(v) => q.bind(v),
    }
}

/// 对 `Query`（无返回值映射）应用绑定
pub fn apply_query<'q>(
    q: sqlx::query::Query<'q, Postgres, PgArguments>,
    b: BoundValue,
) -> sqlx::query::Query<'q, Postgres, PgArguments> {
    match b {
        BoundValue::OptInt(v) => q.bind(v),
        BoundValue::OptFloat(v) => q.bind(v),
        BoundValue::OptBool(v) => q.bind(v),
        BoundValue::OptText(v) => q.bind(v),
        BoundValue::JsonValue(v) => q.bind(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn coerce_bigint_from_number_and_string() {
        assert!(matches!(
            coerce(&json!(42), Some("bigint")),
            BoundValue::OptInt(Some(42))
        ));
        assert!(matches!(
            coerce(&json!("42"), Some("bigint")),
            BoundValue::OptInt(Some(42))
        ));
        // 无法解析 → NULL（schema-soft）
        assert!(matches!(
            coerce(&json!("abc"), Some("bigint")),
            BoundValue::OptInt(None)
        ));
    }

    #[test]
    fn coerce_numeric() {
        assert!(matches!(
            coerce(&json!(12.5), Some("numeric")),
            BoundValue::OptFloat(Some(v)) if (v - 12.5).abs() < 1e-9
        ));
        assert!(matches!(
            coerce(&json!("12.5"), Some("double precision")),
            BoundValue::OptFloat(Some(_))
        ));
    }

    #[test]
    fn coerce_boolean() {
        assert!(matches!(
            coerce(&json!(true), Some("boolean")),
            BoundValue::OptBool(Some(true))
        ));
        assert!(matches!(
            coerce(&json!("是"), Some("boolean")),
            BoundValue::OptBool(Some(true))
        ));
        assert!(matches!(
            coerce(&json!("0"), Some("boolean")),
            BoundValue::OptBool(Some(false))
        ));
        assert!(matches!(
            coerce(&json!("maybe"), Some("boolean")),
            BoundValue::OptBool(None)
        ));
    }

    #[test]
    fn coerce_temporal_passes_text() {
        assert!(matches!(
            coerce(
                &json!("2026-01-15T00:00:00Z"),
                Some("timestamp with time zone")
            ),
            BoundValue::OptText(Some(_))
        ));
        assert!(matches!(
            coerce(&json!("2026-01-15"), Some("date")),
            BoundValue::OptText(Some(_))
        ));
        assert!(matches!(
            coerce(&Value::Null, Some("date")),
            BoundValue::OptText(None)
        ));
    }

    #[test]
    fn coerce_jsonb_binds_value_directly() {
        match coerce(&json!({"a": [1, 2]}), Some("jsonb")) {
            BoundValue::JsonValue(v) => assert_eq!(v, json!({"a": [1, 2]})),
            _ => panic!("jsonb must bind Value directly"),
        }
    }

    #[test]
    fn coerce_unknown_type_falls_back_to_shape() {
        assert!(matches!(
            coerce(&json!(7), None),
            BoundValue::OptInt(Some(7))
        ));
        assert!(matches!(
            coerce(&json!("x"), None),
            BoundValue::OptText(Some(s)) if s == "x"
        ));
        assert!(matches!(
            coerce(&Value::Null, None),
            BoundValue::OptText(None)
        ));
        assert!(matches!(
            coerce(&json!([1, 2]), None),
            BoundValue::JsonValue(_)
        ));
    }

    #[test]
    fn coerce_geometry_wkt_passthrough() {
        // WKT 字符串直通（text→geometry implicit cast）
        assert!(matches!(
            coerce(&json!("POINT(118.986 39.208)"), Some("geometry")),
            BoundValue::OptText(Some(s)) if s == "POINT(118.986 39.208)"
        ));
        assert!(matches!(
            coerce(&json!("POINT(1 2)"), Some("geography")),
            BoundValue::OptText(Some(_))
        ));
    }

    #[test]
    fn coerce_geometry_geojson_to_wkt() {
        // GeoJSON Point → WKT
        assert!(matches!(
            coerce(&json!({"type": "Point", "coordinates": [118.986, 39.208]}), Some("geometry")),
            BoundValue::OptText(Some(s)) if s == "POINT(118.986 39.208)"
        ));
        // GeoJSON LineString → WKT
        assert!(matches!(
            coerce(
                &json!({"type": "LineString", "coordinates": [[118.9, 39.2], [119.0, 39.3]]}),
                Some("geometry")
            ),
            BoundValue::OptText(Some(s)) if s == "LINESTRING(118.9 39.2, 119.0 39.3)"
        ));
        // GeoJSON Polygon → WKT（外环）
        assert!(matches!(
            coerce(
                &json!({"type": "Polygon", "coordinates": [[[0, 0], [1, 0], [1, 1], [0, 0]]]}),
                Some("geometry")
            ),
            BoundValue::OptText(Some(s)) if s == "POLYGON((0 0, 1 0, 1 1, 0 0))"
        ));
        // 不支持的形状 → NULL（schema-soft）
        assert!(matches!(
            coerce(
                &json!({"type": "MultiPoint", "coordinates": [[1, 2]]}),
                Some("geometry")
            ),
            BoundValue::OptText(None)
        ));
        // 数值/布尔 → NULL
        assert!(matches!(
            coerce(&json!(3.14), Some("geometry")),
            BoundValue::OptText(None)
        ));
    }
}
