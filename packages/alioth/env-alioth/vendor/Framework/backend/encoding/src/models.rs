use serde::{Deserialize, Serialize};

// ==================== ZUID ====================

#[derive(Debug, Clone, Deserialize)]
pub struct GenerateZuidRequest {
    pub peer_type: Option<u8>,
    pub idc: Option<u8>,
    pub cluster: Option<u8>,
    pub node: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenerateZuidResponse {
    #[serde(with = "common::serde_zuid")]
    pub zuid: i64,
    pub zuid_u64: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtractZuidRequest {
    pub zuid: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtractZuidResponse {
    pub peer_type: Option<u8>,
    pub idc: u8,
    pub cluster: u8,
    pub node: u8,
    pub timestamp: u64,
    pub sequence: u16,
}

// ==================== Serial ====================

#[derive(Debug, Clone, Deserialize)]
pub struct GenerateSerialRequest {
    pub sequence_name: String,
    pub width: usize,
    #[serde(default = "default_pad_char")]
    pub pad_char: String,
}

fn default_pad_char() -> String {
    "0".to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct GenerateSerialResponse {
    pub serial: String,
}

// ==================== CRC32 ====================

#[derive(Debug, Clone, Deserialize)]
pub struct ComputeCrc32Request {
    pub data: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComputeCrc32Response {
    pub checksum: u32,
    pub checksum_hex: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ValidateCrc32Request {
    pub data: String,
    pub checksum: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidateCrc32Response {
    pub valid: bool,
}

// ==================== Rules ====================

pub use crate::rules::{EncodingResult, EncodingRule};

#[derive(Debug, Clone, Deserialize)]
pub struct ApplyRuleRequest {
    pub rule: EncodingRule,
    #[serde(default)]
    pub use_db_sequences: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyRuleResponse {
    pub code: String,
    pub checksum: Option<String>,
    pub segments: Vec<String>,
}
