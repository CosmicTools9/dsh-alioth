//! 维度/刻度表注册表（Dimension Table Registry）
//!
//! 提供元数据驱动的维度表查询基础设施。
//! 因子通过声明 `DimTable` 静态常量 + `DimRegistry::for_name()` 即可获得
//! 通用列表/详情 handler，无需为每张结构一致的表重复编写 handler + SQL。
//!
//! ## 使用场景
//!
//! 适用于结构简单（id + notice + code + 若干可选字段）的维度/刻度表，
//! 如：zc_id_scene、zc_id_unit、zc_id_status 等标量表。
//! 对于有复杂关联查询、标量引用或业务逻辑的实体，请使用 `crud` 框架的
//! `AliothRepository` + `crud_routes`。

use serde_json::{json, Value};
use sqlx::{AssertSqlSafe, Column, PgPool, Row, TypeInfo};

// ──────────── DimTable ─── 维度表元数据 ───────────────────────────────────────

/// 单张维度/刻度表的元数据
pub struct DimTable {
    /// SELECT 子句列名（不含 SELECT 关键字，用于传给 sqlx::query 后逐列提取）
    pub select_cols: &'static [&'static str],
    /// FROM 子句（含表名、别名、可选的 JOIN）
    pub from_clause: &'static str,
    /// ILIKE 搜索的目标列（带表别名前缀，如 `"notice"` 或 `"s.notice"`）
    pub search_cols: &'static [&'static str],
    /// 主键列（带别名前缀，如 `"id"` 或 `"s.id"`）
    pub pk_col: &'static str,
}

// ──────────── SQL 辅助函数 ─────────────────────────────────────────────────────

/// 构建 ILIKE 搜索条件的 OR 子句
pub fn build_search_conditions(cfg: &DimTable) -> String {
    cfg.search_cols
        .iter()
        .map(|c| format!("{}::text ILIKE $1", c))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// 列解码分类 — 纯函数，单测锁定分派表
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellKind {
    /// 整数列（含 id/qk_*/fk_*/ref_* → 字符串防 JS 精度丢失）
    IntRef,
    /// 其余整数列
    IntPlain,
    /// 定点数（NUMERIC/DECIMAL）— 保留精度为字符串
    NumericString,
    /// 浮点族
    Float,
    /// 布尔
    Bool,
    /// 时间戳/日期
    Temporal,
    /// JSON/JSONB — 原样解析
    JsonValue,
    /// 文本族 + 未知 — 字符串
    TextString,
}

/// 把 PostgreSQL OID 类型名（大写，如 `INT8`/`NUMERIC`/`_TEXT`）映射为分类。
fn classify_cell(ty: &str, name: &str) -> CellKind {
    match ty {
        "INT2" | "INT4" | "INT8" | "SERIAL" | "BIGSERIAL" | "SMALLSERIAL" => {
            // id / qk_* / fk_* / ref_left|ref_right 承载 zuid/标量引用，超出 JS 安全整数 → 字符串；
            // ref_count 等计数器是普通整数，按数值输出
            if name == "id"
                || name.starts_with("qk_")
                || name.starts_with("fk_")
                || name == "ref_left"
                || name == "ref_right"
            {
                CellKind::IntRef
            } else {
                CellKind::IntPlain
            }
        }
        "FLOAT4" | "FLOAT8" | "REAL" | "DOUBLE PRECISION" => CellKind::Float,
        "NUMERIC" | "DECIMAL" | "MONEY" => CellKind::NumericString,
        "BOOL" | "BOOLEAN" => CellKind::Bool,
        "TIMESTAMPTZ" | "TIMESTAMP" | "DATE" | "TIME" | "TIMETZ" => CellKind::Temporal,
        "JSON" | "JSONB" => CellKind::JsonValue,
        // VARCHAR/BPCHAR/TEXT/NAME/UNKNOWN 等一律文本
        _ => CellKind::TextString,
    }
}

/// 把 `PgRow` 第 `idx` 列解码为 JSON 值（按列真实类型分派）。
fn cell_to_json(row: &sqlx::postgres::PgRow, idx: usize, name: &str) -> Value {
    let ty = row.column(idx).type_info().name();
    match classify_cell(ty, name) {
        CellKind::IntRef => match row.try_get::<Option<i64>, _>(idx) {
            Ok(Some(v)) => json!(v.to_string()),
            _ => Value::Null,
        },
        CellKind::IntPlain => match row.try_get::<Option<i64>, _>(idx) {
            Ok(Some(v)) => json!(v),
            _ => Value::Null,
        },
        CellKind::Float => match row.try_get::<Option<f64>, _>(idx) {
            Ok(Some(v)) => json!(v),
            _ => Value::Null,
        },
        CellKind::NumericString => match row.try_get::<Option<rust_decimal::Decimal>, _>(idx) {
            Ok(Some(d)) => json!(d.to_string()),
            _ => Value::Null,
        },
        CellKind::Bool => match row.try_get::<Option<bool>, _>(idx) {
            Ok(Some(v)) => json!(v),
            _ => Value::Null,
        },
        CellKind::Temporal => {
            // TIMESTAMPTZ 优先；其余降级为文本（PG date/time 文本形态可直接消费）
            if let Ok(Some(t)) = row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(idx) {
                return json!(t.to_rfc3339());
            }
            match row.try_get::<Option<String>, _>(idx) {
                Ok(Some(s)) => json!(s),
                _ => Value::Null,
            }
        }
        CellKind::JsonValue => match row.try_get::<Option<Value>, _>(idx) {
            Ok(Some(v)) => v,
            _ => Value::Null,
        },
        CellKind::TextString => match row.try_get::<Option<String>, _>(idx) {
            Ok(Some(s)) => json!(s),
            _ => Value::Null,
        },
    }
}

/// 将 `PgRow` 按 `DimTable` 的 `select_cols` 转换为 JSON
pub fn row_to_json(row: &sqlx::postgres::PgRow, cfg: &DimTable) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (i, col_name) in cfg.select_cols.iter().enumerate() {
        let name = col_name.split('.').next_back().unwrap_or(col_name);
        let val = cell_to_json(row, i, name);
        map.insert(name.to_string(), val);
    }
    serde_json::Value::Object(map)
}

/// 批量转换
pub fn rows_to_json(rows: Vec<sqlx::postgres::PgRow>, cfg: &DimTable) -> Vec<serde_json::Value> {
    rows.iter().map(|r| row_to_json(r, cfg)).collect()
}

/// 通用维度 COUNT（支持关键词搜索）
pub async fn count_dimension(
    pool: &PgPool,
    cfg: &DimTable,
    keyword: &str,
) -> Result<i64, sqlx::Error> {
    if keyword.is_empty() {
        let sql = format!(
            "SELECT COUNT(*) FROM {} WHERE deleted_at IS NULL",
            cfg.from_clause
        );
        sqlx::query_scalar(AssertSqlSafe(sql.as_str()))
            .fetch_one(pool)
            .await
    } else {
        let conditions = build_search_conditions(cfg);
        let sql = format!(
            "SELECT COUNT(*) FROM {} WHERE deleted_at IS NULL AND ({})",
            cfg.from_clause, conditions,
        );
        sqlx::query_scalar(AssertSqlSafe(sql.as_str()))
            .bind(format!("%{}%", keyword))
            .fetch_one(pool)
            .await
    }
}

/// 通用维度分页列表查询
pub async fn list_dimension_rows(
    pool: &PgPool,
    cfg: &DimTable,
    keyword: &str,
    page_size: i64,
    offset: i64,
) -> Result<Vec<sqlx::postgres::PgRow>, sqlx::Error> {
    if keyword.is_empty() {
        let select_sql = cfg.select_cols.join(", ");
        let sql = format!(
            "SELECT {} FROM {} WHERE deleted_at IS NULL ORDER BY {} \
             LIMIT {} OFFSET {}",
            select_sql, cfg.from_clause, cfg.pk_col, page_size, offset,
        );
        sqlx::query(AssertSqlSafe(sql.as_str()))
            .fetch_all(pool)
            .await
    } else {
        let select_sql = cfg.select_cols.join(", ");
        let conditions = build_search_conditions(cfg);
        let sql = format!(
            "SELECT {} FROM {} WHERE deleted_at IS NULL AND ({}) \
             ORDER BY {} LIMIT {} OFFSET {}",
            select_sql, cfg.from_clause, conditions, cfg.pk_col, page_size, offset,
        );
        sqlx::query(AssertSqlSafe(sql.as_str()))
            .bind(format!("%{}%", keyword))
            .fetch_all(pool)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_int_ref_vs_plain() {
        assert_eq!(classify_cell("INT8", "id"), CellKind::IntRef);
        assert_eq!(classify_cell("INT8", "qk_amount"), CellKind::IntRef);
        assert_eq!(classify_cell("INT8", "fk_subject"), CellKind::IntRef);
        assert_eq!(classify_cell("INT8", "ref_left"), CellKind::IntRef);
        // 普通整数列（非引用语义）→ 数值
        assert_eq!(classify_cell("INT4", "precision_"), CellKind::IntPlain);
        assert_eq!(classify_cell("INT8", "ref_count"), CellKind::IntPlain);
        assert_eq!(classify_cell("BIGSERIAL", "seq"), CellKind::IntPlain);
    }

    #[test]
    fn classify_numeric_and_float() {
        assert_eq!(classify_cell("NUMERIC", "mark"), CellKind::NumericString);
        assert_eq!(classify_cell("DECIMAL", "rate"), CellKind::NumericString);
        assert_eq!(classify_cell("MONEY", "amount"), CellKind::NumericString);
        assert_eq!(classify_cell("FLOAT8", "ratio"), CellKind::Float);
        assert_eq!(classify_cell("DOUBLE PRECISION", "x"), CellKind::Float);
    }

    #[test]
    fn classify_bool_and_temporal() {
        assert_eq!(classify_cell("BOOL", "retain_signal"), CellKind::Bool);
        assert_eq!(classify_cell("BOOLEAN", "active"), CellKind::Bool);
        assert_eq!(
            classify_cell("TIMESTAMPTZ", "created_at"),
            CellKind::Temporal
        );
        assert_eq!(classify_cell("TIMESTAMP", "ts"), CellKind::Temporal);
        assert_eq!(classify_cell("DATE", "date"), CellKind::Temporal);
        assert_eq!(classify_cell("TIME", "t"), CellKind::Temporal);
    }

    #[test]
    fn classify_json_and_text() {
        assert_eq!(classify_cell("JSONB", "settings"), CellKind::JsonValue);
        assert_eq!(classify_cell("JSON", "meta"), CellKind::JsonValue);
        assert_eq!(classify_cell("TEXT", "notice"), CellKind::TextString);
        assert_eq!(classify_cell("VARCHAR", "code"), CellKind::TextString);
        assert_eq!(classify_cell("UUID", "uid"), CellKind::TextString);
        assert_eq!(classify_cell("UNKNOWN", "x"), CellKind::TextString);
    }
}
