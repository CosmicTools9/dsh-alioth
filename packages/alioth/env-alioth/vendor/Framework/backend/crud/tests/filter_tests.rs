//! Filter 模块单元测试
//!
//! 测试 `Filter::validate()` 和 `Filter::to_sql()` 的纯逻辑行为。

use crud::filter::Filter;

// ===================================================================
// validate()
// ===================================================================

#[test]
fn test_validate_valid_field_and_op() {
    let f = Filter {
        field: "name".into(),
        op: "eq".into(),
        value: "test".into(),
    };
    assert!(f.validate().is_ok());
}

#[test]
fn test_validate_empty_field() {
    let f = Filter {
        field: "".into(),
        op: "eq".into(),
        value: "test".into(),
    };
    assert!(f.validate().is_err());
}

#[test]
fn test_validate_field_too_long() {
    let long = "a".repeat(64);
    let f = Filter {
        field: long,
        op: "eq".into(),
        value: "test".into(),
    };
    assert!(f.validate().is_err());
}

#[test]
fn test_validate_field_with_null_byte() {
    let f = Filter {
        field: "name\0abc".into(),
        op: "eq".into(),
        value: "test".into(),
    };
    assert!(f.validate().is_err());
}

#[test]
fn test_validate_field_with_semicolon() {
    let f = Filter {
        field: "name; DROP TABLE".into(),
        op: "eq".into(),
        value: "test".into(),
    };
    assert!(f.validate().is_err());
}

#[test]
fn test_validate_field_with_sql_comment() {
    let f = Filter {
        field: "name--".into(),
        op: "eq".into(),
        value: "test".into(),
    };
    assert!(f.validate().is_err());
}

#[test]
fn test_validate_field_with_block_comment() {
    let f = Filter {
        field: "name/*".into(),
        op: "eq".into(),
        value: "test".into(),
    };
    assert!(f.validate().is_err());
}

#[test]
fn test_validate_field_starts_with_digit() {
    let f = Filter {
        field: "1name".into(),
        op: "eq".into(),
        value: "test".into(),
    };
    assert!(f.validate().is_err());
}

#[test]
fn test_validate_field_starts_with_underscore() {
    let f = Filter {
        field: "_name".into(),
        op: "eq".into(),
        value: "test".into(),
    };
    assert!(f.validate().is_ok());
}

#[test]
fn test_validate_field_with_uppercase() {
    let f = Filter {
        field: "Name".into(),
        op: "eq".into(),
        value: "test".into(),
    };
    assert!(f.validate().is_err());
}

#[test]
fn test_validate_field_with_underbar() {
    let f = Filter {
        field: "user_id".into(),
        op: "eq".into(),
        value: "test".into(),
    };
    assert!(f.validate().is_ok());
}

#[test]
fn test_validate_invalid_op() {
    let f = Filter {
        field: "name".into(),
        op: "invalid".into(),
        value: "test".into(),
    };
    assert!(f.validate().is_err());
}

#[test]
fn test_validate_all_valid_ops() {
    let valid_ops = ["eq", "ne", "gt", "lt", "gte", "lte", "like"];
    for op in &valid_ops {
        let f = Filter {
            field: "name".into(),
            op: op.to_string(),
            value: "test".into(),
        };
        assert!(f.validate().is_ok(), "op '{}' should be valid", op);
    }
}

// ===================================================================
// to_sql()
// ===================================================================

#[test]
fn test_to_sql_eq() {
    let f = Filter {
        field: "name".into(),
        op: "eq".into(),
        value: "test".into(),
    };
    // 无列类型信息 → 列 ::text（历史默认，保证未知列不报错）
    assert_eq!(f.to_sql(1), Some("name::text = $1".into()));
}

#[test]
fn test_to_sql_ne() {
    let f = Filter {
        field: "status".into(),
        op: "ne".into(),
        value: "closed".into(),
    };
    assert_eq!(f.to_sql(2), Some("status::text != $2".into()));
}

#[test]
fn test_to_sql_gt() {
    let f = Filter {
        field: "price".into(),
        op: "gt".into(),
        value: "100".into(),
    };
    assert_eq!(f.to_sql(3), Some("price::text > $3".into()));
}

#[test]
fn test_to_sql_lt() {
    let f = Filter {
        field: "age".into(),
        op: "lt".into(),
        value: "18".into(),
    };
    assert_eq!(f.to_sql(1), Some("age::text < $1".into()));
}

#[test]
fn test_to_sql_gte() {
    let f = Filter {
        field: "quantity".into(),
        op: "gte".into(),
        value: "0".into(),
    };
    assert_eq!(f.to_sql(4), Some("quantity::text >= $4".into()));
}

#[test]
fn test_to_sql_lte() {
    let f = Filter {
        field: "score".into(),
        op: "lte".into(),
        value: "100".into(),
    };
    assert_eq!(f.to_sql(5), Some("score::text <= $5".into()));
}

#[test]
fn test_to_sql_like() {
    let f = Filter {
        field: "notice".into(),
        op: "like".into(),
        value: "%keyword%".into(),
    };
    assert_eq!(f.to_sql(1), Some("notice::text LIKE $1".into()));
}

// ===================================================================
// to_sql_with_type() — 元数据驱动分派
// ===================================================================

#[test]
fn test_to_sql_with_type_bigint() {
    let f = Filter {
        field: "qk_amount".into(),
        op: "gt".into(),
        value: "100".into(),
    };
    // 数值列 → 参数 cast，避免 ::text 字典序比较
    assert_eq!(
        f.to_sql_with_type(3, Some("bigint")),
        Some("qk_amount > $3::bigint".into())
    );
}

#[test]
fn test_to_sql_with_type_integer_serial() {
    let f = Filter {
        field: "qk_qty".into(),
        op: "lte".into(),
        value: "10".into(),
    };
    assert_eq!(
        f.to_sql_with_type(1, Some("integer")),
        Some("qk_qty <= $1::bigint".into())
    );
    assert_eq!(
        f.to_sql_with_type(1, Some("smallserial")),
        Some("qk_qty <= $1::bigint".into())
    );
}

#[test]
fn test_to_sql_with_type_numeric() {
    let f = Filter {
        field: "amount".into(),
        op: "gte".into(),
        value: "12.5".into(),
    };
    assert_eq!(
        f.to_sql_with_type(2, Some("numeric")),
        Some("amount >= $2::numeric".into())
    );
    assert_eq!(
        f.to_sql_with_type(2, Some("double precision")),
        Some("amount >= $2::numeric".into())
    );
}

#[test]
fn test_to_sql_with_type_timestamptz() {
    let f = Filter {
        field: "created_at".into(),
        op: "gte".into(),
        value: "2026-01-01T00:00:00Z".into(),
    };
    assert_eq!(
        f.to_sql_with_type(1, Some("timestamp with time zone")),
        Some("created_at >= $1::timestamptz".into())
    );
}

#[test]
fn test_to_sql_with_type_timestamp_and_date() {
    let f = Filter {
        field: "qk_date".into(),
        op: "eq".into(),
        value: "2026-01-15".into(),
    };
    assert_eq!(
        f.to_sql_with_type(1, Some("timestamp without time zone")),
        Some("qk_date = $1::timestamp".into())
    );
    assert_eq!(
        f.to_sql_with_type(1, Some("date")),
        Some("qk_date = $1::date".into())
    );
}

#[test]
fn test_to_sql_with_type_boolean() {
    let f = Filter {
        field: "retain_signal".into(),
        op: "eq".into(),
        value: "true".into(),
    };
    assert_eq!(
        f.to_sql_with_type(1, Some("boolean")),
        Some("retain_signal = $1::boolean".into())
    );
}

#[test]
fn test_to_sql_with_type_text_keeps_cast() {
    // 文本列 → 保持 ::text（like/ilike 模糊查询依赖）
    let f = Filter {
        field: "notice".into(),
        op: "eq".into(),
        value: "abc".into(),
    };
    assert_eq!(
        f.to_sql_with_type(1, Some("text")),
        Some("notice::text = $1".into())
    );
    assert_eq!(
        f.to_sql_with_type(1, Some("character varying")),
        Some("notice::text = $1".into())
    );
}

#[test]
fn test_to_sql_with_type_like_always_text() {
    // like 语义只对文本成立 → 即使列是数值也走 ::text
    let f = Filter {
        field: "qk_amount".into(),
        op: "like".into(),
        value: "%1%".into(),
    };
    assert_eq!(
        f.to_sql_with_type(1, Some("bigint")),
        Some("qk_amount::text LIKE $1".into())
    );
}

#[test]
fn test_to_sql_with_type_unknown_falls_back() {
    let f = Filter {
        field: "uuid_col".into(),
        op: "eq".into(),
        value: "x".into(),
    };
    assert_eq!(
        f.to_sql_with_type(1, Some("uuid")),
        Some("uuid_col::text = $1".into())
    );
    assert_eq!(
        f.to_sql_with_type(1, None),
        Some("uuid_col::text = $1".into())
    );
}

#[test]
fn test_to_sql_with_type_hyphenated_field() {
    let f = Filter {
        field: "ck_cate-biz".into(),
        op: "eq".into(),
        value: "3".into(),
    };
    assert_eq!(
        f.to_sql_with_type(1, Some("bigint")),
        Some(r#""ck_cate-biz" = $1::bigint"#.into())
    );
}

#[test]
fn test_to_sql_invalid_op_returns_none() {
    let f = Filter {
        field: "name".into(),
        op: "bad".into(),
        value: "x".into(),
    };
    assert!(f.to_sql(1).is_none());
}

#[test]
fn test_to_sql_multi_digit_param_index() {
    let f = Filter {
        field: "name".into(),
        op: "eq".into(),
        value: "x".into(),
    };
    assert_eq!(f.to_sql(10), Some("name::text = $10".into()));
    assert_eq!(f.to_sql(100), Some("name::text = $100".into()));
}

// ===================================================================
// validate + to_sql 组合：只有合法字段才生成 SQL
// ===================================================================

#[test]
fn test_validated_filter_produces_sql() {
    let f = Filter {
        field: "status".into(),
        op: "eq".into(),
        value: "active".into(),
    };
    assert!(f.validate().is_ok());
    assert_eq!(f.to_sql(1), Some("status::text = $1".into()));
}

#[test]
fn test_invalid_filter_validate_fails_but_to_sql_still_returns() {
    // to_sql 与 validate 独立 — to_sql 不校验字段名
    let f = Filter {
        field: "DROP;".into(),
        op: "eq".into(),
        value: "x".into(),
    };
    assert!(f.validate().is_err());
    assert_eq!(f.to_sql(1), Some("DROP;::text = $1".into()));
}
