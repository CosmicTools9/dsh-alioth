//! 分页模块单元测试
//!
//! 测试 `ListQueryExt` 的 `to_filter()` / `to_sort()` 行为。

use crud::pagination::{ListQuery, ListQueryExt};

fn make_page(page: i64, page_size: i64) -> ListQuery {
    ListQuery {
        page,
        page_size,
        filter_field: None,
        filter_op: None,
        filter_value: None,
        sort_field: None,
        sort_order: None,
    }
}

#[test]
fn test_to_filter_all_fields_present() {
    let q = ListQuery {
        page: 1,
        page_size: 20,
        filter_field: Some("status".into()),
        filter_op: Some("eq".into()),
        filter_value: Some("active".into()),
        sort_field: Some("name".into()),
        sort_order: Some("asc".into()),
    };
    let f = q.to_filter();
    assert!(f.is_some());
    let f = f.unwrap();
    assert_eq!(f.field, "status");
    assert_eq!(f.op, "eq");
    assert_eq!(f.value, "active");
}

#[test]
fn test_to_filter_none_when_missing_fields() {
    let q = make_page(1, 20);
    assert!(q.to_filter().is_none());
}

#[test]
fn test_to_filter_none_when_partial_fields() {
    // 只提供了 field 但没提供 op/value
    let q = ListQuery {
        page: 1,
        page_size: 20,
        filter_field: Some("name".into()),
        filter_op: None,
        filter_value: Some("test".into()),
        sort_field: None,
        sort_order: None,
    };
    assert!(q.to_filter().is_none());
}

#[test]
fn test_to_sort_present() {
    let q = ListQuery {
        page: 1,
        page_size: 20,
        filter_field: None,
        filter_op: None,
        filter_value: None,
        sort_field: Some("created_at".into()),
        sort_order: Some("desc".into()),
    };
    let s = q.to_sort();
    assert!(s.is_some());
    let s = s.unwrap();
    assert_eq!(s.field, "created_at");
    assert_eq!(s.order, "desc");
}

#[test]
fn test_to_sort_default_order() {
    let q = ListQuery {
        page: 1,
        page_size: 20,
        filter_field: None,
        filter_op: None,
        filter_value: None,
        sort_field: Some("name".into()),
        sort_order: None,
    };
    let s = q.to_sort();
    assert!(s.is_some());
    let s = s.unwrap();
    assert_eq!(s.field, "name");
    assert_eq!(s.order, "desc");
}

#[test]
fn test_to_sort_none_when_no_sort_field() {
    let q = ListQuery {
        page: 1,
        page_size: 20,
        filter_field: None,
        filter_op: None,
        filter_value: None,
        sort_field: None,
        sort_order: Some("asc".into()),
    };
    assert!(q.to_sort().is_none());
}
