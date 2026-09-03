//! PostgreSQL identifier validation utilities
//!
//! Prevents SQL injection in DDL commands where identifiers cannot be parameterized.

/// Validate a PostgreSQL identifier to prevent SQL injection.
///
/// Rules:
/// - Must not be empty
/// - Must not exceed 63 bytes (PostgreSQL NAMEDATALEN - 1)
/// - Must start with a letter or underscore
/// - Must contain only ASCII alphanumeric characters and underscores
///
/// # Examples
/// ```
/// use common::validate_pg_ident;
/// assert!(validate_pg_ident("my_table").is_ok());
/// assert!(validate_pg_ident("123_table").is_err());
/// assert!(validate_pg_ident("table; DROP").is_err());
/// ```
pub fn validate_pg_ident(ident: &str) -> Result<(), &'static str> {
    if ident.is_empty() {
        return Err("Identifier cannot be empty");
    }
    if ident.len() > 63 {
        return Err("Identifier exceeds 63 bytes");
    }
    let mut chars = ident.chars();
    if let Some(first) = chars.next() {
        if first.is_ascii_digit() {
            return Err("Identifier cannot start with a digit");
        }
        if !(first.is_ascii_alphabetic() || first == '_') {
            return Err("Identifier must start with a letter or underscore");
        }
    }
    if chars.any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
        return Err("Identifier contains invalid characters");
    }
    Ok(())
}

/// Validate a PostgreSQL schema-qualified identifier.
///
/// Accepts formats like `"schema"."table"` or `schema.table`.
/// Each component is validated individually.
pub fn validate_qualified_ident(ident: &str) -> Result<(), &'static str> {
    let parts: Vec<&str> = ident.split('.').collect();
    if parts.is_empty() || parts.len() > 2 {
        return Err("Invalid qualified identifier format");
    }
    for part in &parts {
        let part = part.trim_matches('"');
        validate_pg_ident(part)?;
    }
    Ok(())
}

/// PostgreSQL R 类（完全）保留字清单。
///
/// 来源：PG 18 `SELECT word FROM pg_get_keywords() WHERE catcode='R' ORDER BY word`
/// （2026-08-05 自 aliothstudio_dev 实查导出，78 词，按字典序排序）。
/// PG major 升级窗口需复核本清单。
const PG_RESERVED_WORDS: &[&str] = &[
    "all",
    "analyse",
    "analyze",
    "and",
    "any",
    "array",
    "as",
    "asc",
    "asymmetric",
    "both",
    "case",
    "cast",
    "check",
    "collate",
    "column",
    "constraint",
    "create",
    "current_catalog",
    "current_date",
    "current_role",
    "current_time",
    "current_timestamp",
    "current_user",
    "default",
    "deferrable",
    "desc",
    "distinct",
    "do",
    "else",
    "end",
    "except",
    "false",
    "fetch",
    "for",
    "foreign",
    "from",
    "grant",
    "group",
    "having",
    "in",
    "initially",
    "intersect",
    "into",
    "lateral",
    "leading",
    "limit",
    "localtime",
    "localtimestamp",
    "not",
    "null",
    "offset",
    "on",
    "only",
    "or",
    "order",
    "placing",
    "primary",
    "references",
    "returning",
    "select",
    "session_user",
    "some",
    "symmetric",
    "system_user",
    "table",
    "then",
    "to",
    "trailing",
    "true",
    "union",
    "unique",
    "user",
    "using",
    "variadic",
    "when",
    "where",
    "window",
    "with",
];

/// 判断标识符是否为 PostgreSQL R 类（完全）保留字（大小写不敏感）。
///
/// 仅覆盖 R 类；C 类（`time`/`position` 等）物理上可作表/列名，不在此列。
pub fn is_pg_reserved_word(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    PG_RESERVED_WORDS.binary_search(&lower.as_str()).is_ok()
}

/// 将标识符引用为 PG 双引号形式（fix-meta-quote-table-identifiers）。
///
/// 动态 SQL 插值的表名/列名必须经此函数引用：保留字（time/position/group）
/// 与连字符（zc_id_orga-legal）标识符未引号时是语法错误。对已合法的小写
/// 标识符零行为变化（引号引用小写名 ≡ 未引号折叠）。内部 `"` 转义为 `""`。
///
/// 注意：仅用于标识符；SQL 表达式片段/字面量 MUST NOT 经此函数。
pub fn quote_pg_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// 校验业务表标识符（collection `table_name` 新建契约，fix-meta-create-contract）。
///
/// 规则：`^[a-z][a-z0-9_-]*$`，≤63 字节。保留字判定由调用方另行执行
/// （`is_pg_reserved_word`），本函数只管 pattern 与长度。
/// 返回中文错误原因（面向终端用户的契约消息）。
pub fn validate_business_table_ident(ident: &str) -> Result<(), &'static str> {
    if ident.is_empty() {
        return Err("表名不能为空");
    }
    if ident.len() > 63 {
        return Err("表名超过 63 字节上限");
    }
    let mut chars = ident.chars();
    // 非空已校验，first 必存在
    let first = chars.next().unwrap_or_default();
    if !first.is_ascii_lowercase() {
        return Err("表名必须以小写字母开头");
    }
    if chars.any(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')) {
        return Err("表名只能包含小写字母、数字、下划线和连字符");
    }
    Ok(())
}

/// 校验业务字段标识符（field `name` 新建契约，fix-meta-create-contract）。
///
/// 规则：`^[a-zA-Z_][a-zA-Z0-9_-]*$`，≤63 字节。字段名**不拒**保留字
/// （存量 `group`/`time`/`position` 字段为合法用途，字段链 DDL 全部双引号引用）。
/// 返回中文错误原因（面向终端用户的契约消息）。
pub fn validate_business_field_ident(ident: &str) -> Result<(), &'static str> {
    if ident.is_empty() {
        return Err("字段名不能为空");
    }
    if ident.len() > 63 {
        return Err("字段名超过 63 字节上限");
    }
    let mut chars = ident.chars();
    let first = chars.next().unwrap_or_default();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err("字段名必须以字母或下划线开头");
    }
    if chars.any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
        return Err("字段名只能包含字母、数字、下划线和连字符");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_idents() {
        assert!(validate_pg_ident("my_table").is_ok());
        assert!(validate_pg_ident("_private").is_ok());
        assert!(validate_pg_ident("Table123").is_ok());
        assert!(validate_pg_ident("a").is_ok());
        assert!(validate_pg_ident("table-name").is_ok());
        assert!(validate_pg_ident("table-with-dash").is_ok());
    }

    #[test]
    fn test_invalid_idents() {
        assert!(validate_pg_ident("").is_err());
        assert!(validate_pg_ident("123_table").is_err());
        assert!(validate_pg_ident("table; DROP").is_err());
        assert!(validate_pg_ident("table name").is_err());
        assert!(validate_pg_ident(
            "a_very_long_identifier_that_exceeds_sixty_three_characters_limit"
        )
        .is_err());
    }

    #[test]
    fn test_is_pg_reserved_word() {
        assert!(is_pg_reserved_word("select"));
        assert!(is_pg_reserved_word("SELECT"));
        assert!(is_pg_reserved_word("Table"));
        assert!(is_pg_reserved_word("user"));
        assert!(!is_pg_reserved_word("fg"));
        assert!(!is_pg_reserved_word("ztable"));
        assert!(!is_pg_reserved_word("my_table"));
        // C 类保留字不在拒绝集合内
        assert!(!is_pg_reserved_word("time"));
        assert!(!is_pg_reserved_word("position"));
    }

    #[test]
    fn test_validate_business_table_ident() {
        assert!(validate_business_table_ident("my_table").is_ok());
        assert!(validate_business_table_ident("a").is_ok());
        assert!(validate_business_table_ident("order-items").is_ok());
        assert!(validate_business_table_ident("").is_err());
        assert!(validate_business_table_ident("1abc").is_err());
        assert!(validate_business_table_ident("Abc").is_err());
        assert!(validate_business_table_ident("_abc").is_err());
        assert!(validate_business_table_ident("has space").is_err());
        assert!(validate_business_table_ident("has.dot").is_err());
        assert!(validate_business_table_ident(
            "a_very_long_identifier_that_exceeds_sixty_three_characters_limit"
        )
        .is_err());
    }

    #[test]
    fn test_validate_business_field_ident() {
        assert!(validate_business_field_ident("name").is_ok());
        assert!(validate_business_field_ident("Name").is_ok());
        assert!(validate_business_field_ident("_private").is_ok());
        assert!(validate_business_field_ident("group").is_ok());
        assert!(validate_business_field_ident("").is_err());
        assert!(validate_business_field_ident("1x").is_err());
        assert!(validate_business_field_ident("has space").is_err());
        assert!(validate_business_field_ident(
            "a_very_long_identifier_that_exceeds_sixty_three_characters_limit"
        )
        .is_err());
    }

    #[test]
    fn test_quote_pg_ident() {
        assert_eq!(quote_pg_ident("name"), "\"name\"");
        // 保留字/连字符标识符：引号化后语法合法
        assert_eq!(quote_pg_ident("time"), "\"time\"");
        assert_eq!(quote_pg_ident("group"), "\"group\"");
        assert_eq!(quote_pg_ident("zc_id_orga-legal"), "\"zc_id_orga-legal\"");
        // 内部双引号转义
        assert_eq!(quote_pg_ident("a\"b"), "\"a\"\"b\"");
    }
}
