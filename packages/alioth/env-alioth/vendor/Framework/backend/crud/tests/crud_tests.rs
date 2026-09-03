use crud::*;

#[test]
fn test_crud_error_display() {
    let err = CrudError::NotFound("entity 42".to_string());
    assert!(err.to_string().contains("entity 42"));

    let err = CrudError::BadRequest("invalid param".to_string());
    assert!(err.to_string().contains("invalid param"));
}

#[test]
fn test_list_query_deserialize() {
    let query: ListQuery = serde_json::from_str(r#"{"page": 2, "page_size": 50}"#).unwrap();
    assert_eq!(query.page, 2);
    assert_eq!(query.page_size, 50);
}

#[test]
fn test_list_query_defaults() {
    let query: ListQuery = serde_json::from_str(r#"{}"#).unwrap();
    assert_eq!(query.page, 1);
    assert_eq!(query.page_size, 20);
}

#[test]
fn test_list_query_with_filters() {
    let json = r#"{"page":1,"page_size":10,"filter_field":"status","filter_op":"eq","filter_value":"active","sort_field":"name","sort_order":"asc"}"#;
    let query: ListQuery = serde_json::from_str(json).unwrap();
    assert_eq!(query.filter_field.as_deref(), Some("status"));
    assert_eq!(query.sort_field.as_deref(), Some("name"));
}

#[test]
fn test_paginated_response_construction() {
    let resp: PaginatedResponse<String> = PaginatedResponse {
        items: vec!["a".to_string(), "b".to_string()],
        total: 10,
        page: 1,
        page_size: 20,
    };
    assert_eq!(resp.items.len(), 2);
    assert_eq!(resp.total, 10);
}

#[test]
fn test_paginated_response_new() {
    let resp: PaginatedResponse<i64> = PaginatedResponse::new(vec![1, 2, 3], 100, 1, 20);
    assert_eq!(resp.items.len(), 3);
    assert_eq!(resp.total, 100);
}

#[test]
fn test_batch_create_deserialize() {
    let json = r#"{"items": [{"name": "item1"}]}"#;
    let req: BatchCreateRequest<serde_json::Value> = serde_json::from_str(json).unwrap();
    assert_eq!(req.items.len(), 1);
}

#[test]
fn test_batch_response_construction() {
    let resp = BatchResponse::new(5, 0);
    // BatchResponse only impl Serialize, no deserialize - test construction only
    assert_eq!(resp.success, 5);
    assert_eq!(resp.failed, 0);
}

#[test]
fn test_batch_response_with_errors() {
    let resp = BatchResponse::with_errors(3, 2, vec!["error 1".to_string()]);
    assert_eq!(resp.success, 3);
    assert_eq!(resp.errors.len(), 1);
}

#[test]
fn test_identifiable_trait() {
    use crud::entity::Identifiable;

    struct TestEntity {
        id: i64,
    }
    impl Identifiable for TestEntity {
        fn id(&self) -> i64 {
            self.id
        }
    }

    let entity = TestEntity { id: 42 };
    assert_eq!(entity.id(), 42);
}

#[test]
fn test_sort_deserialize() {
    let sort: Sort = serde_json::from_str(r#"{"field": "name", "order": "asc"}"#).unwrap();
    assert_eq!(sort.field, "name");
    assert_eq!(sort.order, "asc");
}

#[test]
fn test_sort_default_order() {
    let sort: Sort = serde_json::from_str(r#"{"field": "name"}"#).unwrap();
    assert_eq!(sort.order, "desc");
}

#[test]
fn test_sort_validate_valid() {
    let sort = Sort {
        field: "name".to_string(),
        order: "asc".to_string(),
    };
    assert!(sort.validate().is_ok());
}

#[test]
fn test_sort_validate_invalid_field() {
    let sort = Sort {
        field: "".to_string(),
        order: "asc".to_string(),
    };
    assert!(sort.validate().is_err());
}

#[test]
fn test_sort_validate_invalid_order() {
    let sort = Sort {
        field: "name".to_string(),
        order: "invalid".to_string(),
    };
    assert!(sort.validate().is_err());
}

#[test]
fn test_sort_to_sql() {
    let sort = Sort {
        field: "name".to_string(),
        order: "ASC".to_string(),
    };
    assert_eq!(sort.to_sql(), "name asc");
}

#[test]
fn test_filter_validate() {
    use crud::filter::Filter;

    let filter = Filter {
        field: "status".to_string(),
        op: "eq".to_string(),
        value: "active".to_string(),
    };
    assert!(filter.validate().is_ok());

    let empty = Filter {
        field: "".to_string(),
        op: "eq".to_string(),
        value: "test".to_string(),
    };
    assert!(empty.validate().is_err());
}

#[test]
fn test_filter_to_sql() {
    use crud::filter::Filter;

    let filter = Filter {
        field: "status".to_string(),
        op: "eq".to_string(),
        value: "active".to_string(),
    };
    let sql = filter.to_sql(3);
    assert_eq!(sql, Some("status::text = $3".to_string()));

    let like_filter = Filter {
        field: "name".to_string(),
        op: "like".to_string(),
        value: "%test%".to_string(),
    };
    assert_eq!(
        like_filter.to_sql(1),
        Some("name::text LIKE $1".to_string())
    );
}

#[test]
fn test_batch_delete_deserialize() {
    let json = r#"{"ids": [1, 2, 3]}"#;
    let req: BatchDeleteRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.ids.len(), 3);
}
