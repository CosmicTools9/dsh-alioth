use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single segment in an encoding rule
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EncodingSegment {
    /// Fixed text prefix
    Prefix { value: String },
    /// Current date formatted with strftime pattern
    Date { format: String },
    /// Database-backed sequence
    Sequence {
        sequence_name: String,
        width: usize,
        pad_char: char,
    },
    /// Random alphanumeric string
    Random { length: usize, charset: String },
    /// Fixed literal value
    Literal { value: String },
}

/// Complete encoding rule definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodingRule {
    pub id: String,
    pub name: String,
    pub segments: Vec<EncodingSegment>,
    pub checksum_algorithm: Option<ChecksumAlgorithm>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumAlgorithm {
    Crc32,
}

/// Context provided when applying a rule
#[derive(Debug, Clone, Default)]
pub struct EncodingContext {
    pub sequences: HashMap<String, u64>,
    pub overrides: HashMap<String, String>,
}

/// Result of applying an encoding rule
#[derive(Debug, Clone)]
pub struct EncodingResult {
    pub code: String,
    pub checksum: Option<String>,
    pub segments: Vec<String>,
}

/// Encoding rule engine
#[derive(Debug, Default, Clone)]
pub struct EncodingRuleEngine;

impl EncodingRuleEngine {
    pub fn new() -> Self {
        Self
    }

    /// Apply a rule to produce a code string
    pub fn apply(
        &self,
        rule: &EncodingRule,
        ctx: &EncodingContext,
    ) -> Result<EncodingResult, EncodingRuleError> {
        let mut segments = Vec::with_capacity(rule.segments.len());

        for segment in &rule.segments {
            let value = match segment {
                EncodingSegment::Prefix { value } => value.clone(),
                EncodingSegment::Literal { value } => value.clone(),
                EncodingSegment::Date { format } => Local::now().format(format).to_string(),
                EncodingSegment::Sequence {
                    sequence_name,
                    width,
                    pad_char,
                } => {
                    let seq = ctx.sequences.get(sequence_name).copied().unwrap_or(1);
                    format!("{:0width$}", seq, width = width).replace('0', &pad_char.to_string())
                }
                EncodingSegment::Random { length, charset } => {
                    Self::random_string(*length, charset)
                }
            };
            segments.push(value);
        }

        let mut code = segments.join("");
        let mut checksum = None;

        if let Some(algo) = &rule.checksum_algorithm {
            match algo {
                ChecksumAlgorithm::Crc32 => {
                    let crc = crate::crc32::compute_checksum_hex(code.as_bytes());
                    checksum = Some(crc.clone());
                    code.push_str(&crc);
                }
            }
        }

        Ok(EncodingResult {
            code,
            checksum,
            segments,
        })
    }

    fn random_string(length: usize, charset: &str) -> String {
        use rand::RngExt;
        let chars: Vec<char> = charset.chars().collect();
        if chars.is_empty() {
            return String::new();
        }
        let mut rng = rand::rng();
        (0..length)
            .map(|_| chars[rng.random_range(0..chars.len())])
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EncodingRuleError {
    #[error("Invalid segment configuration: {0}")]
    InvalidConfig(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_date_sequence_rule() {
        let rule = EncodingRule {
            id: "test-1".to_string(),
            name: "Test Rule".to_string(),
            segments: vec![
                EncodingSegment::Prefix {
                    value: "ORD".to_string(),
                },
                EncodingSegment::Date {
                    format: "%Y%m%d".to_string(),
                },
                EncodingSegment::Sequence {
                    sequence_name: "ord_seq".to_string(),
                    width: 4,
                    pad_char: '0',
                },
            ],
            checksum_algorithm: None,
        };
        let mut ctx = EncodingContext::default();
        ctx.sequences.insert("ord_seq".to_string(), 42);

        let engine = EncodingRuleEngine::new();
        let result = engine.apply(&rule, &ctx).unwrap();

        assert!(result.code.starts_with("ORD"));
        assert!(result.code.contains("0042"));
        assert_eq!(result.segments.len(), 3);
    }

    #[test]
    fn test_rule_with_crc32_checksum() {
        let rule = EncodingRule {
            id: "test-crc".to_string(),
            name: "CRC Rule".to_string(),
            segments: vec![
                EncodingSegment::Prefix {
                    value: "INV".to_string(),
                },
                EncodingSegment::Literal {
                    value: "001".to_string(),
                },
            ],
            checksum_algorithm: Some(ChecksumAlgorithm::Crc32),
        };

        let engine = EncodingRuleEngine::new();
        let result = engine.apply(&rule, &EncodingContext::default()).unwrap();

        assert!(result.code.starts_with("INV001"));
        assert!(result.checksum.is_some());
        let checksum = result.checksum.unwrap();
        assert_eq!(checksum.len(), 8);

        // Verify checksum is valid CRC32
        let base_code = "INV001";
        let expected_crc = crate::crc32::compute_checksum_hex_lower(base_code.as_bytes());
        assert_eq!(checksum.to_ascii_lowercase(), expected_crc);
        assert!(crate::crc32::validate_checksum(
            base_code.as_bytes(),
            u32::from_str_radix(&expected_crc, 16).unwrap()
        ));
    }

    #[test]
    fn test_sequence_segment() {
        let rule = EncodingRule {
            id: "seq-test".to_string(),
            name: "Sequence Test".to_string(),
            segments: vec![
                EncodingSegment::Prefix {
                    value: "SEQ".to_string(),
                },
                EncodingSegment::Sequence {
                    sequence_name: "my_seq".to_string(),
                    width: 5,
                    pad_char: '0',
                },
            ],
            checksum_algorithm: None,
        };
        let mut ctx = EncodingContext::default();
        ctx.sequences.insert("my_seq".to_string(), 7);

        let engine = EncodingRuleEngine::new();
        let result = engine.apply(&rule, &ctx).unwrap();
        assert_eq!(result.code, "SEQ00007");
    }

    #[test]
    fn test_random_segment_with_charset() {
        let rule = EncodingRule {
            id: "rand-test".to_string(),
            name: "Random Test".to_string(),
            segments: vec![EncodingSegment::Random {
                length: 8,
                charset: "XYZ".to_string(),
            }],
            checksum_algorithm: None,
        };
        let engine = EncodingRuleEngine::new();
        let result = engine.apply(&rule, &EncodingContext::default()).unwrap();
        assert_eq!(result.code.len(), 8);
        assert!(result.code.chars().all(|c| "XYZ".contains(c)));
    }
}
