//! Sort 模块单元测试
//!
//! 测试 `Sort::validate()` 和 `Sort::to_sql()` 的纯逻辑行为。

use crud::sort::Sort;

// ===================================================================
// validate()
// ===================================================================

#[test]
fn test_validate_valid_field_asc() {
    let s = Sort {
        field: "name".into(),
        order: "asc".into(),
    };
    assert!(s.validate().is_ok());
}

#[test]
fn test_validate_valid_field_desc() {
    let s = Sort {
        field: "name".into(),
        order: "desc".into(),
    };
    assert!(s.validate().is_ok());
}

#[test]
fn test_validate_case_insensitive_order() {
    let s = Sort {
        field: "name".into(),
        order: "ASC".into(),
    };
    assert!(s.validate().is_ok());
    let s = Sort {
        field: "name".into(),
        order: "Desc".into(),
    };
    assert!(s.validate().is_ok());
}

#[test]
fn test_validate_invalid_order() {
    let s = Sort {
        field: "name".into(),
        order: "random".into(),
    };
    assert!(s.validate().is_err());
}

#[test]
fn test_validate_empty_field() {
    let s = Sort {
        field: "".into(),
        order: "asc".into(),
    };
    assert!(s.validate().is_err());
}

#[test]
fn test_validate_field_too_long() {
    let long = "a".repeat(64);
    let s = Sort {
        field: long,
        order: "asc".into(),
    };
    assert!(s.validate().is_err());
}

#[test]
fn test_validate_field_with_sql_injection() {
    let cases = ["name\0", "name; DROP", "name--", "name/*"];
    for field in &cases {
        let s = Sort {
            field: field.to_string(),
            order: "asc".into(),
        };
        assert!(
            s.validate().is_err(),
            "field '{:?}' should be rejected",
            field
        );
    }
}

#[test]
fn test_validate_field_underscore_start() {
    let s = Sort {
        field: "_created_at".into(),
        order: "asc".into(),
    };
    assert!(s.validate().is_ok());
}

#[test]
fn test_validate_field_contains_digit() {
    let s = Sort {
        field: "field1".into(),
        order: "asc".into(),
    };
    assert!(s.validate().is_ok());
}

// ===================================================================
// to_sql()
// ===================================================================

#[test]
fn test_to_sql_asc() {
    let s = Sort {
        field: "name".into(),
        order: "asc".into(),
    };
    assert_eq!(s.to_sql(), "name asc");
}

#[test]
fn test_to_sql_desc() {
    let s = Sort {
        field: "created_at".into(),
        order: "desc".into(),
    };
    assert_eq!(s.to_sql(), "created_at desc");
}

#[test]
fn test_to_sql_case_insensitive() {
    let s = Sort {
        field: "name".into(),
        order: "ASC".into(),
    };
    assert_eq!(s.to_sql(), "name asc");

    let s = Sort {
        field: "name".into(),
        order: "Desc".into(),
    };
    assert_eq!(s.to_sql(), "name desc");
}

#[test]
fn test_to_sql_unknown_order_passed_through() {
    // to_sql 不校验 order 合法性
    let s = Sort {
        field: "name".into(),
        order: "INVALID".into(),
    };
    assert_eq!(s.to_sql(), "name invalid");
}
