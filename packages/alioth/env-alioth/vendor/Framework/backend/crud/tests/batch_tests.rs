//! 批量操作模块单元测试
//!
//! 测试 `BatchResponse` 的构造方法。

use crud::batch::BatchResponse;

#[test]
fn test_batch_response_new() {
    let resp = BatchResponse::new(3, 0);
    assert_eq!(resp.success, 3);
    assert_eq!(resp.failed, 0);
    assert!(resp.errors.is_empty());
}

#[test]
fn test_batch_response_with_errors() {
    let errors = vec!["item 1: invalid".into(), "item 2: duplicate".into()];
    let resp = BatchResponse::with_errors(1, 2, errors);
    assert_eq!(resp.success, 1);
    assert_eq!(resp.failed, 2);
    assert_eq!(resp.errors.len(), 2);
    assert_eq!(resp.errors[0], "item 1: invalid");
    assert_eq!(resp.errors[1], "item 2: duplicate");
}

#[test]
fn test_batch_response_empty_errors_not_serialized() {
    // 确认空 errors 列表在序列化时被跳过
    let resp = BatchResponse::new(5, 0);
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["success"], 5);
    assert_eq!(json["failed"], 0);
    assert!(!json.as_object().unwrap().contains_key("errors"));
}

#[test]
fn test_batch_response_non_empty_errors_serialized() {
    let resp = BatchResponse::with_errors(0, 2, vec!["err1".into()]);
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["errors"], serde_json::json!(["err1"]));
}
