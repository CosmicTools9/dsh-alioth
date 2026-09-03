use i18n::*;
use std::collections::HashMap;

#[test]
fn test_locale_new_and_display() {
    let loc = Locale::new("en-US");
    assert_eq!(loc.as_str(), "en-US");
    assert_eq!(format!("{}", loc), "en-US");
}

#[test]
fn test_locale_default() {
    let loc = Locale::default();
    assert_eq!(loc.as_str(), "zh-CN");
}

#[test]
fn test_locale_constants() {
    assert_eq!(Locale::ZH_CN, "zh-CN");
    assert_eq!(Locale::EN, "en");
}

#[test]
fn test_locale_equality() {
    assert_eq!(Locale::new("zh-CN"), Locale::new("zh-CN"));
    assert_ne!(Locale::new("zh-CN"), Locale::new("en"));
}

#[test]
fn test_parse_accept_language_empty() {
    let result = parse_accept_language("");
    assert!(result.is_empty());
}

#[test]
fn test_parse_accept_language_single() {
    let result = parse_accept_language("zh-CN");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0.as_str(), "zh-CN");
    assert!((result[0].1 - 1.0).abs() < 0.001);
}

#[test]
fn test_parse_accept_language_multiple_with_quality() {
    let result = parse_accept_language("zh-CN, en;q=0.5");
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].0.as_str(), "zh-CN");
    assert!((result[0].1 - 1.0).abs() < 0.001);
    assert_eq!(result[1].0.as_str(), "en");
    assert!((result[1].1 - 0.5).abs() < 0.001);
}

#[test]
fn test_parse_accept_language_quality_clamping() {
    let result = parse_accept_language("en;q=2.0");
    assert!((result[0].1 - 1.0).abs() < 0.001);
}

#[test]
fn test_resolve_locale_exact_match() {
    let result = resolve_locale("en", &["zh-CN", "en"], "zh-CN");
    assert_eq!(result.as_str(), "en");
}

#[test]
fn test_resolve_locale_primary_match() {
    // "en-US" should fallback to "en"
    let result = resolve_locale("en-US", &["en", "zh-CN"], "zh-CN");
    assert_eq!(result.as_str(), "en");
}

#[test]
fn test_resolve_locale_fallback() {
    let result = resolve_locale("fr", &["zh-CN", "en"], "zh-CN");
    assert_eq!(result.as_str(), "zh-CN");
}

#[test]
fn test_dictionary_from_json() {
    let json = serde_json::json!({
        "hello": "你好",
        "nested": {
            "greeting": "早上好"
        }
    });
    let dict = Dictionary::from_json(json);
    assert_eq!(dict.get("hello"), Some("你好"));
    assert_eq!(dict.get("nested.greeting"), Some("早上好"));
}

#[test]
fn test_dictionary_load() {
    let json_str = r#"{"key": "value", "nested": {"deep": "深"}}"#;
    let dict = Dictionary::load(json_str).unwrap();
    assert_eq!(dict.get("key"), Some("value"));
    assert_eq!(dict.get("nested.deep"), Some("深"));
}

#[test]
fn test_dictionary_load_invalid_json() {
    let result = Dictionary::load("not json");
    assert!(result.is_err());
}

#[test]
fn test_dictionary_get_missing_key() {
    let dict = Dictionary::from_json(serde_json::json!({"a": "1"}));
    assert_eq!(dict.get("nonexistent"), None);
}

#[test]
fn test_dictionary_merge() {
    let mut dict1 = Dictionary::from_json(serde_json::json!({"a": "1"}));
    let dict2 = Dictionary::from_json(serde_json::json!({"b": "2"}));
    dict1.merge(dict2);
    assert_eq!(dict1.get("a"), Some("1"));
    assert_eq!(dict1.get("b"), Some("2"));
}

#[test]
fn test_dictionary_merge_overwrite() {
    let mut dict1 = Dictionary::from_json(serde_json::json!({"key": "old"}));
    let dict2 = Dictionary::from_json(serde_json::json!({"key": "new"}));
    dict1.merge(dict2);
    assert_eq!(dict1.get("key"), Some("new"));
}

#[test]
fn test_i18n_manager_new() {
    let manager = I18nManager::new("zh-CN");
    // Verify we can create a manager (no getter available for default_locale)
    let _ = format!("{:?}", manager);
}

#[test]
fn test_i18n_manager_load_and_get() {
    let mut manager = I18nManager::new("en");
    let dict = Dictionary::from_json(serde_json::json!({"hello": "Hello"}));
    manager.load_locale(&Locale::new("en"), dict);
    assert_eq!(manager.get(&Locale::new("en"), "hello"), Some("Hello"));
}

#[test]
fn test_i18n_manager_get_with_locale() {
    let mut manager = I18nManager::new("en");
    manager.load_locale(
        &Locale::new("en"),
        Dictionary::from_json(serde_json::json!({"greet": "Hello"})),
    );
    manager.load_locale(
        &Locale::new("zh-CN"),
        Dictionary::from_json(serde_json::json!({"greet": "你好"})),
    );
    assert_eq!(manager.get(&Locale::new("zh-CN"), "greet"), Some("你好"));
}

#[test]
fn test_i18n_manager_get_missing_key() {
    let manager = I18nManager::new("en");
    assert_eq!(manager.get(&Locale::new("en"), "nonexistent"), None);
}

#[test]
fn test_i18n_manager_format() {
    let mut manager = I18nManager::new("en");
    manager.load_locale(
        &Locale::new("en"),
        Dictionary::from_json(serde_json::json!({"greeting": "Hello, {name}"})),
    );
    let mut params = HashMap::new();
    params.insert("name".to_string(), "World".to_string());
    assert_eq!(
        manager.format(&Locale::new("en"), "greeting", &params),
        "Hello, World"
    );
}

#[test]
fn test_i18n_manager_format_fallback() {
    let mut manager = I18nManager::new("en");
    manager.load_locale(
        &Locale::new("en"),
        Dictionary::from_json(serde_json::json!({"key": "value"})),
    );
    // Request nonexistent locale, should fallback to default
    assert_eq!(manager.get(&Locale::new("fr"), "key"), Some("value"));
}

#[test]
fn test_i18n_manager_load_from_json() {
    let mut manager = I18nManager::new("zh-CN");
    manager
        .load_locale_from_json(&Locale::new("zh-CN"), r#"{"hello": "你好"}"#)
        .unwrap();
    assert_eq!(manager.get(&Locale::new("zh-CN"), "hello"), Some("你好"));
}

#[test]
fn test_i18n_manager_load_from_json_invalid() {
    let mut manager = I18nManager::new("en");
    let result = manager.load_locale_from_json(&Locale::new("en"), "not json");
    assert!(result.is_err());
}
