use base64::Engine;
use system_config::*;

#[test]
fn test_config_category_serde() {
    let category = ConfigCategory {
        code: "llm".to_string(),
        notice: "大语言模型".to_string(),
        description: Some("LLM providers".to_string()),
        icon: Some("Brain".to_string()),
        providers: vec![ConfigProvider {
            code: "openai".to_string(),
            notice: "OpenAI".to_string(),
            description: Some("OpenAI API".to_string()),
            schema: vec![ConfigFieldSchema {
                key: "api_key".to_string(),
                notice: "API Key".to_string(),
                field_type: "password".to_string(),
                required: true,
                placeholder: Some("sk-...".to_string()),
                default_value: None,
                options: None,
                sensitive: true,
                help_text: Some("你的 OpenAI API 密钥".to_string()),
            }],
            defaults: None,
        }],
    };
    let json = serde_json::to_string(&category).unwrap();
    let back: ConfigCategory = serde_json::from_str(&json).unwrap();
    assert_eq!(back.code, "llm");
    assert_eq!(back.providers.len(), 1);
    assert!(back.providers[0].schema[0].sensitive);
}

#[test]
fn test_config_field_schema_serde() {
    let field = ConfigFieldSchema {
        key: "api_key".to_string(),
        notice: "API Key".to_string(),
        field_type: "password".to_string(),
        required: true,
        placeholder: None,
        default_value: None,
        options: None,
        sensitive: true,
        help_text: None,
    };
    let json = serde_json::to_string(&field).unwrap();
    let back: ConfigFieldSchema = serde_json::from_str(&json).unwrap();
    assert!(back.required);
    assert!(back.sensitive);
}

#[test]
fn test_config_provider_serde() {
    let provider = ConfigProvider {
        code: "deepseek".to_string(),
        notice: "DeepSeek".to_string(),
        description: None,
        schema: vec![],
        defaults: None,
    };
    let json = serde_json::to_string(&provider).unwrap();
    let back: ConfigProvider = serde_json::from_str(&json).unwrap();
    assert_eq!(back.code, "deepseek");
}

#[test]
fn test_system_config_serde() {
    let config = SystemConfig {
        id: 1,
        notice: None,
        code: None,
        _f_: Some("llm".to_string()),
        _t_: Some("openai".to_string()),
        comments: None,
        credentials: Some(serde_json::json!({"api_key": "enc:xxx"})),
        settings: Some(serde_json::json!({"temperature": 0.7})),
        enabled: Some(true),
        is_default: Some(true),
        domain_: None,
        public: Some(false),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        created_by_id: None,
        updated_by_id: None,
        deleted_at: None,
    };
    let json = serde_json::to_string(&config).unwrap();
    let back: SystemConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, 1);
    assert_eq!(back._f_.as_deref(), Some("llm"));
    assert_eq!(back.enabled, Some(true));
}

#[test]
fn test_system_config_error_display() {
    let err = SystemConfigError::NotFound(42);
    assert!(err.to_string().contains("42"));

    let err = SystemConfigError::Validation("bad config".to_string());
    assert!(err.to_string().contains("bad config"));
}

#[test]
fn test_schema_functions() {
    let categories = schema::get_all_categories();
    assert!(!categories.is_empty());

    let llm = schema::get_category("llm");
    assert!(llm.is_some());
    assert_eq!(llm.unwrap().code, "llm");

    let fields = schema::get_provider_schema("llm", "openai");
    assert!(fields.is_some());
    assert!(!fields.unwrap().is_empty());
}

#[test]
fn test_sensitive_keys() {
    let keys = schema::get_sensitive_keys("llm", "openai");
    assert!(keys.contains(&"api_key".to_string()));
}

#[test]
fn test_crypto_generate_key() {
    let key = crypto::generate_key();
    assert!(!key.is_empty());
    let decoded = base64::engine::general_purpose::STANDARD.decode(&key);
    assert!(decoded.is_ok());
    assert_eq!(decoded.unwrap().len(), 32);
}

#[test]
fn test_crypto_encrypt_decrypt_roundtrip() {
    let key = crypto::generate_key();
    crypto::init_encryption(&key).unwrap();

    let plaintext = "my-secret-value";
    let encrypted = crypto::encrypt(plaintext).unwrap();
    assert_ne!(encrypted, plaintext);

    let decrypted = crypto::decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_crypto_encrypt_decrypt_unicode() {
    let key = crypto::generate_key();
    crypto::init_encryption(&key).unwrap();

    let plaintext = "你好，世界！こんにちは";
    let encrypted = crypto::encrypt(plaintext).unwrap();
    let decrypted = crypto::decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_crypto_encrypt_json_fields() {
    let key = crypto::generate_key();
    crypto::init_encryption(&key).unwrap();

    let mut value = serde_json::json!({
        "api_key": "sk-test",
        "name": "not-sensitive",
        "nested": {
            "secret": "hidden"
        }
    });

    crypto::encrypt_json_fields(&mut value, &["api_key", "secret"]).unwrap();
    let api_key = value["api_key"].as_str().unwrap();
    assert!(api_key.starts_with("enc:"));

    assert_eq!(value["name"], serde_json::json!("not-sensitive"));
}

#[test]
fn test_crypto_decrypt_json_fields() {
    let key = crypto::generate_key();
    crypto::init_encryption(&key).unwrap();

    let plain = "hello";
    let encrypted = format!("enc:{}", crypto::encrypt(plain).unwrap());

    let mut value = serde_json::json!({
        "api_key": encrypted,
    });

    crypto::decrypt_json_fields(&mut value, &["api_key"]).unwrap();
    assert_eq!(value["api_key"], serde_json::json!("hello"));
}

#[test]
fn test_select_option_serde() {
    let option = SelectOption {
        value: "gpt-4".to_string(),
        notice: "GPT-4".to_string(),
    };
    let json = serde_json::to_string(&option).unwrap();
    let back: SelectOption = serde_json::from_str(&json).unwrap();
    assert_eq!(back.value, "gpt-4");
}

#[test]
fn test_safe_response_fields() {
    let safe = SystemConfigSafeResponse {
        id: 1,
        notice: Some("Display Name".to_string()),
        code: Some("cfg-1".to_string()),
        _f_: Some("llm".to_string()),
        _t_: Some("openai".to_string()),
        comments: None,
        credentials_set: true,
        settings: None,
        enabled: Some(true),
        is_default: None,
        domain_: None,
        public: Some(true),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let json = serde_json::to_string(&safe).unwrap();
    let back: SystemConfigSafeResponse = serde_json::from_str(&json).unwrap();
    assert!(back.credentials_set);
}
