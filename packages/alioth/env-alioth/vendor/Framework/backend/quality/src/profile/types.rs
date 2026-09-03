use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 字段画像统计结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldProfile {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub field_id: i64,
    #[serde(with = "common::serde_zuid")]
    pub collection_id: i64,
    pub run_at: DateTime<Utc>,
    #[serde(with = "common::serde_zuid")]
    pub row_count: i64,
    #[serde(with = "common::serde_zuid")]
    pub null_count: i64,
    pub null_percentage: f64,
    #[serde(with = "common::serde_zuid")]
    pub unique_count: i64,
    pub cardinality_ratio: f64, // unique_count / non_null_count
    pub statistics: ProfileStatistics,
    pub top_values: Vec<ValueFrequency>,
    pub histogram: Option<HistogramData>,
}

/// 统计类型（根据数据类型变化）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProfileStatistics {
    Numeric(NumericStatistics),
    Text(TextStatistics),
    DateTime(DateTimeStatistics),
    Boolean(BooleanStatistics),
    Generic(GenericStatistics),
}

/// 数值统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumericStatistics {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
    pub median: Option<f64>,
    pub std_dev: Option<f64>,
    pub sum: Option<f64>,
    #[serde(with = "common::serde_zuid")]
    pub zeros_count: i64,
    #[serde(with = "common::serde_zuid")]
    pub negatives_count: i64,
}

/// 文本统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextStatistics {
    pub min_length: Option<i32>,
    pub max_length: Option<i32>,
    pub avg_length: Option<f64>,
    #[serde(with = "common::serde_zuid")]
    pub empty_string_count: i64,
    #[serde(with = "common::serde_zuid")]
    pub whitespace_only_count: i64,
}

/// 日期时间统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DateTimeStatistics {
    pub min_date: Option<DateTime<Utc>>,
    pub max_date: Option<DateTime<Utc>>,
    #[serde(with = "common::serde_zuid")]
    pub future_dates_count: i64,
    #[serde(with = "common::serde_zuid")]
    pub past_dates_count: i64,
}

/// 布尔统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BooleanStatistics {
    #[serde(with = "common::serde_zuid")]
    pub true_count: i64,
    #[serde(with = "common::serde_zuid")]
    pub false_count: i64,
    pub true_percentage: f64,
}

/// 通用统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenericStatistics {
    #[serde(with = "common::serde_zuid")]
    pub distinct_count: i64,
}

/// 值频率
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueFrequency {
    pub value: String,
    #[serde(with = "common::serde_zuid")]
    pub count: i64,
    pub percentage: f64,
}

/// 直方图数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramData {
    pub buckets: Vec<HistogramBucket>,
    pub bucket_type: BucketType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramBucket {
    pub range_start: String,
    pub range_end: String,
    #[serde(with = "common::serde_zuid")]
    pub count: i64,
    pub percentage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BucketType {
    Numeric,
    DateTime,
    TextLength,
}

/// 集合画像运行状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionProfileRun {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub collection_id: i64,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status: RunStatus,
    pub fields_count: i32,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
}

/// 画像趋势数据点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileTrendPoint {
    pub run_at: DateTime<Utc>,
    pub null_percentage: f64,
    #[serde(with = "common::serde_zuid")]
    pub unique_count: i64,
    #[serde(with = "common::serde_zuid")]
    pub row_count: i64,
}

/// 数据类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DataType {
    Integer,
    BigInt,
    SmallInt,
    Decimal,
    Numeric,
    Real,
    Double,
    VarChar,
    Char,
    Text,
    Boolean,
    Date,
    Time,
    Timestamp,
    Timestamptz,
    Json,
    Jsonb,
    Uuid,
    Bytea,
    Unknown,
}

impl DataType {
    /// 从 PostgreSQL 类型名称解析
    pub fn from_pg_type(type_name: &str) -> Self {
        match type_name.to_lowercase().as_str() {
            "integer" | "int" | "int4" => DataType::Integer,
            "bigint" | "int8" => DataType::BigInt,
            "smallint" | "int2" => DataType::SmallInt,
            "decimal" => DataType::Decimal,
            "numeric" => DataType::Numeric,
            "real" | "float4" => DataType::Real,
            "double precision" | "float8" => DataType::Double,
            "character varying" | "varchar" => DataType::VarChar,
            "character" | "char" => DataType::Char,
            "text" => DataType::Text,
            "boolean" | "bool" => DataType::Boolean,
            "date" => DataType::Date,
            "time" | "time without time zone" => DataType::Time,
            "timestamp" | "timestamp without time zone" => DataType::Timestamp,
            "timestamp with time zone" | "timestamptz" => DataType::Timestamptz,
            "json" => DataType::Json,
            "jsonb" => DataType::Jsonb,
            "uuid" => DataType::Uuid,
            "bytea" => DataType::Bytea,
            _ => DataType::Unknown,
        }
    }

    /// 判断是否为数值类型
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            DataType::Integer
                | DataType::BigInt
                | DataType::SmallInt
                | DataType::Decimal
                | DataType::Numeric
                | DataType::Real
                | DataType::Double
        )
    }

    /// 判断是否为文本类型
    pub fn is_text(&self) -> bool {
        matches!(self, DataType::VarChar | DataType::Char | DataType::Text)
    }

    /// 判断是否为日期时间类型
    pub fn is_datetime(&self) -> bool {
        matches!(
            self,
            DataType::Date | DataType::Time | DataType::Timestamp | DataType::Timestamptz
        )
    }

    /// 判断是否为布尔类型
    pub fn is_boolean(&self) -> bool {
        matches!(self, DataType::Boolean)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_type_from_pg() {
        assert!(matches!(
            DataType::from_pg_type("integer"),
            DataType::Integer
        ));
        assert!(matches!(
            DataType::from_pg_type("varchar"),
            DataType::VarChar
        ));
        assert!(matches!(
            DataType::from_pg_type("boolean"),
            DataType::Boolean
        ));
    }

    #[test]
    fn test_data_type_checks() {
        assert!(DataType::Integer.is_numeric());
        assert!(!DataType::Integer.is_text());
        assert!(DataType::VarChar.is_text());
        assert!(DataType::Timestamp.is_datetime());
        assert!(DataType::Boolean.is_boolean());
    }
}
