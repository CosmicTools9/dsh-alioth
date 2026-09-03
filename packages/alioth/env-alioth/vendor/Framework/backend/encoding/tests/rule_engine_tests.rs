use encoding::rules::{
    ChecksumAlgorithm, EncodingContext, EncodingRule, EncodingRuleEngine, EncodingSegment,
};

#[test]
fn test_rule_engine_prefix() {
    let rule = EncodingRule {
        id: "prefix-test".to_string(),
        name: "Prefix Test".to_string(),
        segments: vec![EncodingSegment::Prefix {
            value: "PRE".to_string(),
        }],
        checksum_algorithm: None,
    };
    let engine = EncodingRuleEngine::new();
    let result = engine.apply(&rule, &EncodingContext::default()).unwrap();
    assert_eq!(result.code, "PRE");
}

#[test]
fn test_rule_engine_prefix_literal_concatenation() {
    let rule = EncodingRule {
        id: "concat-test".to_string(),
        name: "Concatenation Test".to_string(),
        segments: vec![
            EncodingSegment::Prefix {
                value: "ORD".to_string(),
            },
            EncodingSegment::Literal {
                value: "001".to_string(),
            },
        ],
        checksum_algorithm: None,
    };
    let engine = EncodingRuleEngine::new();
    let result = engine.apply(&rule, &EncodingContext::default()).unwrap();
    assert_eq!(result.code, "ORD001");
}

#[test]
fn test_rule_engine_date_format() {
    let rule = EncodingRule {
        id: "date-test".to_string(),
        name: "Date Test".to_string(),
        segments: vec![EncodingSegment::Date {
            format: "%Y%m%d".to_string(),
        }],
        checksum_algorithm: None,
    };
    let engine = EncodingRuleEngine::new();
    let result = engine.apply(&rule, &EncodingContext::default()).unwrap();
    assert_eq!(result.code.len(), 8, "Expected 8-digit date");
    assert!(
        result.code.chars().all(|c| c.is_ascii_digit()),
        "All chars must be digits"
    );
}

#[test]
fn test_rule_engine_random_charset() {
    let rule = EncodingRule {
        id: "random-test".to_string(),
        name: "Random Test".to_string(),
        segments: vec![EncodingSegment::Random {
            length: 6,
            charset: "ABCDEF".to_string(),
        }],
        checksum_algorithm: None,
    };
    let engine = EncodingRuleEngine::new();
    let result = engine.apply(&rule, &EncodingContext::default()).unwrap();
    assert_eq!(result.code.len(), 6);
    assert!(
        result.code.chars().all(|c| "ABCDEF".contains(c)),
        "All chars must be from charset"
    );
}

#[test]
fn test_rule_engine_crc32_checksum() {
    let rule = EncodingRule {
        id: "crc-test".to_string(),
        name: "CRC Test".to_string(),
        segments: vec![EncodingSegment::Prefix {
            value: "INV".to_string(),
        }],
        checksum_algorithm: Some(ChecksumAlgorithm::Crc32),
    };
    let engine = EncodingRuleEngine::new();
    let result = engine.apply(&rule, &EncodingContext::default()).unwrap();
    assert!(result.checksum.is_some(), "Checksum must be present");
    let checksum = result.checksum.unwrap();
    assert_eq!(checksum.len(), 8);
    assert!(result.code.starts_with("INV"));
    assert!(
        result.code.len() > 3,
        "Code should include checksum appended"
    );
}
