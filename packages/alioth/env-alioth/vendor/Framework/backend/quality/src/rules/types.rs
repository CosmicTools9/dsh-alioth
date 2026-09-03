use serde::{Deserialize, Serialize};

/// 规则类型枚举 - 20种内置规则
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE", type_name = "TEXT")]
pub enum RuleType {
    // 基础检查 (2种)
    NotNull, // 非空检查
    Unique,  // 唯一性检查

    // 数值范围 (5种)
    Range,          // 范围检查 (min, max)
    RangeInclusive, // 包含边界的范围
    GreaterThan,    // 大于指定值
    LessThan,       // 小于指定值
    EqualTo,        // 等于指定值

    // 文本检查 (4种)
    LengthMin,   // 最小长度
    LengthMax,   // 最大长度
    LengthRange, // 长度范围
    RegexMatch,  // 正则匹配

    // 格式验证 (3种)
    EmailFormat, // 邮箱格式
    UrlFormat,   // URL格式
    UuidFormat,  // UUID格式

    // 枚举与集合 (2种)
    Enum,  // 枚举值检查
    NotIn, // 排除值检查

    // 类型检查 (5种)
    IsNumeric,   // 是数字
    IsInteger,   // 是整数
    IsBoolean,   // 是布尔值
    IsDate,      // 是日期
    IsTimestamp, // 是时间戳

    // 统计检查 (2种)
    NullPercentage,   // 空值比例限制
    CardinalityRatio, // 基数比例限制

    // 高级 (1种)
    CustomSql, // 自定义SQL检查
}

impl RuleType {
    /// 获取规则类型的字符串表示
    pub fn as_str(&self) -> &'static str {
        match self {
            RuleType::NotNull => "NOT_NULL",
            RuleType::Unique => "UNIQUE",
            RuleType::Range => "RANGE",
            RuleType::RangeInclusive => "RANGE_INCLUSIVE",
            RuleType::GreaterThan => "GREATER_THAN",
            RuleType::LessThan => "LESS_THAN",
            RuleType::EqualTo => "EQUAL_TO",
            RuleType::LengthMin => "LENGTH_MIN",
            RuleType::LengthMax => "LENGTH_MAX",
            RuleType::LengthRange => "LENGTH_RANGE",
            RuleType::RegexMatch => "REGEX_MATCH",
            RuleType::EmailFormat => "EMAIL_FORMAT",
            RuleType::UrlFormat => "URL_FORMAT",
            RuleType::UuidFormat => "UUID_FORMAT",
            RuleType::Enum => "ENUM",
            RuleType::NotIn => "NOT_IN",
            RuleType::IsNumeric => "IS_NUMERIC",
            RuleType::IsInteger => "IS_INTEGER",
            RuleType::IsBoolean => "IS_BOOLEAN",
            RuleType::IsDate => "IS_DATE",
            RuleType::IsTimestamp => "IS_TIMESTAMP",
            RuleType::NullPercentage => "NULL_PERCENTAGE",
            RuleType::CardinalityRatio => "CARDINALITY_RATIO",
            RuleType::CustomSql => "CUSTOM_SQL",
        }
    }

    /// 获取规则类型的显示名称
    pub fn display_name(&self) -> &'static str {
        match self {
            RuleType::NotNull => "非空检查",
            RuleType::Unique => "唯一性检查",
            RuleType::Range => "数值范围",
            RuleType::RangeInclusive => "数值范围（含边界）",
            RuleType::GreaterThan => "大于",
            RuleType::LessThan => "小于",
            RuleType::EqualTo => "等于",
            RuleType::LengthMin => "最小长度",
            RuleType::LengthMax => "最大长度",
            RuleType::LengthRange => "长度范围",
            RuleType::RegexMatch => "正则匹配",
            RuleType::EmailFormat => "邮箱格式",
            RuleType::UrlFormat => "URL格式",
            RuleType::UuidFormat => "UUID格式",
            RuleType::Enum => "枚举值",
            RuleType::NotIn => "排除值",
            RuleType::IsNumeric => "数字类型",
            RuleType::IsInteger => "整数类型",
            RuleType::IsBoolean => "布尔类型",
            RuleType::IsDate => "日期类型",
            RuleType::IsTimestamp => "时间戳类型",
            RuleType::NullPercentage => "空值比例",
            RuleType::CardinalityRatio => "基数比例",
            RuleType::CustomSql => "自定义SQL",
        }
    }

    /// 获取规则类型的描述
    pub fn description(&self) -> &'static str {
        match self {
            RuleType::NotNull => "检查字段值不能为空",
            RuleType::Unique => "检查字段值在表中必须唯一",
            RuleType::Range => "检查数值是否在指定范围内（不含边界）",
            RuleType::RangeInclusive => "检查数值是否在指定范围内（含边界）",
            RuleType::GreaterThan => "检查数值必须大于指定值",
            RuleType::LessThan => "检查数值必须小于指定值",
            RuleType::EqualTo => "检查数值必须等于指定值",
            RuleType::LengthMin => "检查文本长度不小于指定值",
            RuleType::LengthMax => "检查文本长度不大于指定值",
            RuleType::LengthRange => "检查文本长度在指定范围内",
            RuleType::RegexMatch => "检查文本匹配指定正则表达式",
            RuleType::EmailFormat => "检查文本符合邮箱格式",
            RuleType::UrlFormat => "检查文本符合URL格式",
            RuleType::UuidFormat => "检查文本符合UUID格式",
            RuleType::Enum => "检查值在指定枚举列表中",
            RuleType::NotIn => "检查值不在指定排除列表中",
            RuleType::IsNumeric => "检查值可以解析为数字",
            RuleType::IsInteger => "检查值可以解析为整数",
            RuleType::IsBoolean => "检查值为布尔类型",
            RuleType::IsDate => "检查值为日期类型",
            RuleType::IsTimestamp => "检查值为时间戳类型",
            RuleType::NullPercentage => "检查空值比例不超过阈值",
            RuleType::CardinalityRatio => "检查基数比例满足要求",
            RuleType::CustomSql => "使用自定义SQL表达式验证数据",
        }
    }

    /// 获取规则支持的参数定义
    pub fn parameter_definitions(&self) -> Vec<RuleParameterDef> {
        match self {
            RuleType::NotNull | RuleType::Unique => vec![],

            RuleType::Range => vec![
                RuleParameterDef::new("min", RuleParameterType::Number, false, "最小值"),
                RuleParameterDef::new("max", RuleParameterType::Number, false, "最大值"),
            ],
            RuleType::RangeInclusive => vec![
                RuleParameterDef::new("min", RuleParameterType::Number, false, "最小值（含）"),
                RuleParameterDef::new("max", RuleParameterType::Number, false, "最大值（含）"),
            ],
            RuleType::GreaterThan => vec![RuleParameterDef::new(
                "value",
                RuleParameterType::Number,
                true,
                "阈值",
            )],
            RuleType::LessThan => vec![RuleParameterDef::new(
                "value",
                RuleParameterType::Number,
                true,
                "阈值",
            )],
            RuleType::EqualTo => vec![RuleParameterDef::new(
                "value",
                RuleParameterType::Number,
                true,
                "目标值",
            )],

            RuleType::LengthMin => vec![RuleParameterDef::new(
                "min",
                RuleParameterType::Integer,
                true,
                "最小长度",
            )],
            RuleType::LengthMax => vec![RuleParameterDef::new(
                "max",
                RuleParameterType::Integer,
                true,
                "最大长度",
            )],
            RuleType::LengthRange => vec![
                RuleParameterDef::new("min", RuleParameterType::Integer, true, "最小长度"),
                RuleParameterDef::new("max", RuleParameterType::Integer, true, "最大长度"),
            ],
            RuleType::RegexMatch => vec![RuleParameterDef::new(
                "pattern",
                RuleParameterType::String,
                true,
                "正则表达式",
            )],

            RuleType::EmailFormat | RuleType::UrlFormat | RuleType::UuidFormat => vec![],

            RuleType::Enum => vec![RuleParameterDef::new(
                "values",
                RuleParameterType::StringArray,
                true,
                "允许的值列表",
            )],
            RuleType::NotIn => vec![RuleParameterDef::new(
                "values",
                RuleParameterType::StringArray,
                true,
                "排除的值列表",
            )],

            RuleType::IsNumeric
            | RuleType::IsInteger
            | RuleType::IsBoolean
            | RuleType::IsDate
            | RuleType::IsTimestamp => vec![],

            RuleType::NullPercentage => vec![RuleParameterDef::new(
                "max_percentage",
                RuleParameterType::Decimal,
                true,
                "最大空值比例(0-100)",
            )],
            RuleType::CardinalityRatio => vec![
                RuleParameterDef::new(
                    "min_ratio",
                    RuleParameterType::Decimal,
                    false,
                    "最小基数比例(0-1)",
                ),
                RuleParameterDef::new(
                    "max_ratio",
                    RuleParameterType::Decimal,
                    false,
                    "最大基数比例(0-1)",
                ),
            ],

            RuleType::CustomSql => vec![
                RuleParameterDef::new("sql", RuleParameterType::String, true, "SQL表达式"),
                RuleParameterDef::new(
                    "expected_result",
                    RuleParameterType::String,
                    false,
                    "期望结果",
                ),
            ],
        }
    }

    /// 获取适用的数据类型分类
    pub fn applicable_data_types(&self) -> Vec<DataTypeCategory> {
        match self {
            RuleType::NotNull | RuleType::Unique => vec![DataTypeCategory::All],

            RuleType::Range
            | RuleType::RangeInclusive
            | RuleType::GreaterThan
            | RuleType::LessThan
            | RuleType::EqualTo => vec![DataTypeCategory::Numeric],

            RuleType::LengthMin
            | RuleType::LengthMax
            | RuleType::LengthRange
            | RuleType::RegexMatch
            | RuleType::EmailFormat
            | RuleType::UrlFormat
            | RuleType::UuidFormat
            | RuleType::Enum
            | RuleType::NotIn => vec![DataTypeCategory::Text],

            RuleType::IsNumeric
            | RuleType::IsInteger
            | RuleType::IsBoolean
            | RuleType::IsDate
            | RuleType::IsTimestamp => vec![DataTypeCategory::All],

            RuleType::NullPercentage | RuleType::CardinalityRatio => vec![DataTypeCategory::All],

            RuleType::CustomSql => vec![DataTypeCategory::All],
        }
    }

    /// 获取规则分类
    pub fn category(&self) -> RuleCategory {
        match self {
            RuleType::NotNull | RuleType::Unique => RuleCategory::Basic,
            RuleType::Range
            | RuleType::RangeInclusive
            | RuleType::GreaterThan
            | RuleType::LessThan
            | RuleType::EqualTo => RuleCategory::Range,
            RuleType::LengthMin | RuleType::LengthMax | RuleType::LengthRange => {
                RuleCategory::Length
            }
            RuleType::RegexMatch
            | RuleType::EmailFormat
            | RuleType::UrlFormat
            | RuleType::UuidFormat => RuleCategory::Format,
            RuleType::Enum | RuleType::NotIn => RuleCategory::Set,
            RuleType::IsNumeric
            | RuleType::IsInteger
            | RuleType::IsBoolean
            | RuleType::IsDate
            | RuleType::IsTimestamp => RuleCategory::Type,
            RuleType::NullPercentage | RuleType::CardinalityRatio => RuleCategory::Statistic,
            RuleType::CustomSql => RuleCategory::Advanced,
        }
    }
}

impl std::str::FromStr for RuleType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "NOT_NULL" => Ok(RuleType::NotNull),
            "UNIQUE" => Ok(RuleType::Unique),
            "RANGE" => Ok(RuleType::Range),
            "RANGE_INCLUSIVE" => Ok(RuleType::RangeInclusive),
            "GREATER_THAN" => Ok(RuleType::GreaterThan),
            "LESS_THAN" => Ok(RuleType::LessThan),
            "EQUAL_TO" => Ok(RuleType::EqualTo),
            "LENGTH_MIN" => Ok(RuleType::LengthMin),
            "LENGTH_MAX" => Ok(RuleType::LengthMax),
            "LENGTH_RANGE" => Ok(RuleType::LengthRange),
            "REGEX_MATCH" => Ok(RuleType::RegexMatch),
            "EMAIL_FORMAT" => Ok(RuleType::EmailFormat),
            "URL_FORMAT" => Ok(RuleType::UrlFormat),
            "UUID_FORMAT" => Ok(RuleType::UuidFormat),
            "ENUM" => Ok(RuleType::Enum),
            "NOT_IN" => Ok(RuleType::NotIn),
            "IS_NUMERIC" => Ok(RuleType::IsNumeric),
            "IS_INTEGER" => Ok(RuleType::IsInteger),
            "IS_BOOLEAN" => Ok(RuleType::IsBoolean),
            "IS_DATE" => Ok(RuleType::IsDate),
            "IS_TIMESTAMP" => Ok(RuleType::IsTimestamp),
            "NULL_PERCENTAGE" => Ok(RuleType::NullPercentage),
            "CARDINALITY_RATIO" => Ok(RuleType::CardinalityRatio),
            "CUSTOM_SQL" => Ok(RuleType::CustomSql),
            _ => Err(format!("Unknown rule type: {}", s)),
        }
    }
}

/// 规则参数定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleParameterDef {
    pub name: String,
    pub param_type: RuleParameterType,
    pub required: bool,
    pub description: String,
}

impl RuleParameterDef {
    pub fn new(
        name: &str,
        param_type: RuleParameterType,
        required: bool,
        description: &str,
    ) -> Self {
        Self {
            name: name.to_string(),
            param_type,
            required,
            description: description.to_string(),
        }
    }
}

/// 规则参数值类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum RuleParameterType {
    String,
    Integer,
    Number,
    Decimal,
    Boolean,
    StringArray,
    NumberArray,
}

/// 数据类型分类
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DataTypeCategory {
    All,
    Numeric,
    Text,
    Boolean,
    DateTime,
}

/// 规则分类
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RuleCategory {
    Basic,     // 基础
    Range,     // 范围
    Length,    // 长度
    Format,    // 格式
    Set,       // 集合
    Type,      // 类型
    Statistic, // 统计
    Advanced,  // 高级
}

impl RuleCategory {
    pub fn display_name(&self) -> &'static str {
        match self {
            RuleCategory::Basic => "基础检查",
            RuleCategory::Range => "数值范围",
            RuleCategory::Length => "文本长度",
            RuleCategory::Format => "格式验证",
            RuleCategory::Set => "枚举集合",
            RuleCategory::Type => "类型检查",
            RuleCategory::Statistic => "统计检查",
            RuleCategory::Advanced => "高级",
        }
    }
}

/// 质量规则配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityRule {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub rule_type: RuleType,
    pub enabled: bool,
    pub severity: RuleSeverity,
    pub parameters: serde_json::Value,

    // 关联
    #[serde(with = "common::serde_zuid::opt")]
    pub collection_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub field_id: Option<i64>,

    // 元数据
    #[serde(with = "common::serde_zuid::opt")]
    pub created_by: Option<i64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// 规则严重程度
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE", type_name = "TEXT")]
pub enum RuleSeverity {
    Info,     // 信息级别
    Warning,  // 警告级别
    Error,    // 错误级别
    Critical, // 严重级别
}

impl RuleSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuleSeverity::Info => "INFO",
            RuleSeverity::Warning => "WARNING",
            RuleSeverity::Error => "ERROR",
            RuleSeverity::Critical => "CRITICAL",
        }
    }
}

impl std::str::FromStr for RuleSeverity {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "INFO" => Ok(RuleSeverity::Info),
            "WARNING" => Ok(RuleSeverity::Warning),
            "ERROR" => Ok(RuleSeverity::Error),
            "CRITICAL" => Ok(RuleSeverity::Critical),
            _ => Err(format!("Unknown severity: {}", s)),
        }
    }
}

/// 规则执行状态
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE", type_name = "TEXT")]
pub enum RuleStatus {
    Passed,  // 通过
    Failed,  // 失败
    Errored, // 执行错误
    Skipped, // 跳过
}

impl RuleStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuleStatus::Passed => "PASSED",
            RuleStatus::Failed => "FAILED",
            RuleStatus::Errored => "ERRORED",
            RuleStatus::Skipped => "SKIPPED",
        }
    }
}

/// 规则执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleExecutionResult {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub rule_id: i64,
    pub rule_name: String,
    pub rule_type: RuleType,
    pub severity: RuleSeverity,
    pub status: RuleStatus,
    pub executed_at: chrono::DateTime<chrono::Utc>,
    #[serde(with = "common::serde_zuid")]
    pub duration_ms: i64,

    // 统计
    #[serde(with = "common::serde_zuid")]
    pub total_rows: i64,
    #[serde(with = "common::serde_zuid")]
    pub passed_rows: i64,
    #[serde(with = "common::serde_zuid")]
    pub failed_rows: i64,
    pub pass_percentage: f64,

    // 失败详情（采样）
    pub failed_samples: Vec<FailedRowSample>,

    // 错误信息
    pub error_message: Option<String>,
}

/// 失败行采样
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedRowSample {
    pub row_id: Option<String>,
    pub field_value: String,
    pub reason: String,
}

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for RuleExecutionResult {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;

        let failed_samples: Option<serde_json::Value> = row.try_get("sample_failed_values")?;
        let failed_samples = failed_samples
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        Ok(RuleExecutionResult {
            id: row.try_get("id")?,
            rule_id: row.try_get("rule_id")?,
            rule_name: row.try_get("rule_name")?,
            rule_type: row.try_get("rule_type")?,
            severity: row.try_get("severity")?,
            status: row.try_get("status")?,
            executed_at: row.try_get("executed_at")?,
            duration_ms: row.try_get("duration_ms")?,
            total_rows: row.try_get("total_rows")?,
            passed_rows: row.try_get("passed_rows")?,
            failed_rows: row.try_get("failed_rows")?,
            pass_percentage: row.try_get("pass_percentage")?,
            failed_samples,
            error_message: row.try_get("error_message")?,
        })
    }
}

/// 规则执行请求
#[derive(Debug, Clone, Deserialize)]
pub struct ExecuteRulesRequest {
    #[serde(with = "common::serde_zuid::opt")]
    pub collection_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub field_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt_seq")]
    pub rule_ids: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sample_limit: Option<i64>,
}

/// 规则定义（用于规则库展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleDefinition {
    pub rule_type: RuleType,
    pub display_name: String,
    pub description: String,
    pub category: RuleCategory,
    pub parameters: Vec<RuleParameterDef>,
    pub applicable_data_types: Vec<DataTypeCategory>,
}

impl RuleDefinition {
    pub fn from_rule_type(rule_type: RuleType) -> Self {
        Self {
            rule_type,
            display_name: rule_type.display_name().to_string(),
            description: rule_type.description().to_string(),
            category: rule_type.category(),
            parameters: rule_type.parameter_definitions(),
            applicable_data_types: rule_type.applicable_data_types(),
        }
    }
}

/// 获取所有内置规则定义
pub fn get_builtin_rules() -> Vec<RuleDefinition> {
    vec![
        RuleDefinition::from_rule_type(RuleType::NotNull),
        RuleDefinition::from_rule_type(RuleType::Unique),
        RuleDefinition::from_rule_type(RuleType::Range),
        RuleDefinition::from_rule_type(RuleType::RangeInclusive),
        RuleDefinition::from_rule_type(RuleType::GreaterThan),
        RuleDefinition::from_rule_type(RuleType::LessThan),
        RuleDefinition::from_rule_type(RuleType::EqualTo),
        RuleDefinition::from_rule_type(RuleType::LengthMin),
        RuleDefinition::from_rule_type(RuleType::LengthMax),
        RuleDefinition::from_rule_type(RuleType::LengthRange),
        RuleDefinition::from_rule_type(RuleType::RegexMatch),
        RuleDefinition::from_rule_type(RuleType::EmailFormat),
        RuleDefinition::from_rule_type(RuleType::UrlFormat),
        RuleDefinition::from_rule_type(RuleType::UuidFormat),
        RuleDefinition::from_rule_type(RuleType::Enum),
        RuleDefinition::from_rule_type(RuleType::NotIn),
        RuleDefinition::from_rule_type(RuleType::IsNumeric),
        RuleDefinition::from_rule_type(RuleType::IsInteger),
        RuleDefinition::from_rule_type(RuleType::IsBoolean),
        RuleDefinition::from_rule_type(RuleType::IsDate),
        RuleDefinition::from_rule_type(RuleType::IsTimestamp),
        RuleDefinition::from_rule_type(RuleType::NullPercentage),
        RuleDefinition::from_rule_type(RuleType::CardinalityRatio),
        RuleDefinition::from_rule_type(RuleType::CustomSql),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rule_type_display_name() {
        assert_eq!(RuleType::NotNull.display_name(), "非空检查");
        assert_eq!(RuleType::Range.display_name(), "数值范围");
    }

    #[test]
    fn test_rule_type_category() {
        assert_eq!(RuleType::NotNull.category(), RuleCategory::Basic);
        assert_eq!(RuleType::Range.category(), RuleCategory::Range);
        assert_eq!(RuleType::EmailFormat.category(), RuleCategory::Format);
    }

    #[test]
    fn test_get_builtin_rules() {
        let rules = get_builtin_rules();
        assert_eq!(rules.len(), 24); // 20种规则
    }

    #[test]
    fn test_rule_type_from_str() {
        assert!(matches!(
            "NOT_NULL".parse::<RuleType>().unwrap(),
            RuleType::NotNull
        ));
        assert!(matches!(
            "range".parse::<RuleType>().unwrap(),
            RuleType::Range
        ));
    }
}
