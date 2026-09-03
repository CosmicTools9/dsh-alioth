use sqlx::PgPool;

use crate::{
    crc32,
    rules::{EncodingContext, EncodingResult, EncodingRule, EncodingRuleEngine, EncodingRuleError},
    serial::{SerialError, SerialGenerator},
    zuid::{PeerType, ZuidError, ZuidGenerator},
};

/// Unified encoding service combining ZUID, CRC32, serial generation, and rule engine
#[derive(Debug, Clone)]
pub struct EncodingService {
    pool: PgPool,
    zuid: ZuidGenerator,
    serial: SerialGenerator,
    rules: EncodingRuleEngine,
}

#[derive(Debug, thiserror::Error)]
pub enum EncodingServiceError {
    #[error("ZUID error: {0}")]
    Zuid(#[from] ZuidError),
    #[error("Serial error: {0}")]
    Serial(#[from] SerialError),
    #[error("Rule error: {0}")]
    Rule(#[from] EncodingRuleError),
    #[error("Sequence error: {0}")]
    Sequence(String),
}

impl EncodingService {
    /// Create a new encoding service with the default ZUID generator (Consumer, 0, 0, 0)
    pub fn new(pool: PgPool) -> Result<Self, EncodingServiceError> {
        Ok(Self {
            pool,
            zuid: ZuidGenerator::new(PeerType::Consumer, 0, 0, 0)?,
            serial: SerialGenerator::new(),
            rules: EncodingRuleEngine::new(),
        })
    }

    /// Create with a custom ZUID generator configuration
    pub fn with_zuid(
        pool: PgPool,
        peer_type: PeerType,
        idc: u8,
        cluster: u8,
        node: u8,
    ) -> Result<Self, EncodingServiceError> {
        Ok(Self {
            pool,
            zuid: ZuidGenerator::new(peer_type, idc, cluster, node)?,
            serial: SerialGenerator::new(),
            rules: EncodingRuleEngine::new(),
        })
    }

    /// Generate a new ZUID as i64 (compatible with PostgreSQL BIGINT)
    pub fn generate_zuid(&self) -> i64 {
        self.zuid.generate()
    }

    /// Generate a new ZUID as u64
    pub fn generate_zuid_u64(&self) -> u64 {
        self.zuid.generate_u64()
    }

    /// Get the peer ID component of the configured ZUID generator
    pub fn get_zuid_peer_id(&self) -> u64 {
        self.zuid.get_peer_id()
    }

    /// Compute CRC32 checksum for data
    pub fn compute_crc32(&self, data: &[u8]) -> u32 {
        crc32::compute_checksum(data)
    }

    /// Validate CRC32 checksum
    pub fn validate_crc32(&self, data: &[u8], expected: u32) -> bool {
        crc32::validate_checksum(data, expected)
    }

    /// Fetch the next value from a PostgreSQL sequence
    pub async fn next_sequence(&self, sequence_name: &str) -> Result<i64, EncodingServiceError> {
        Ok(self
            .serial
            .next_sequence_value(&self.pool, sequence_name)
            .await?)
    }

    /// Generate a formatted serial number from a PostgreSQL sequence
    pub async fn generate_serial(
        &self,
        sequence_name: &str,
        width: usize,
        pad_char: char,
    ) -> Result<String, EncodingServiceError> {
        Ok(self
            .serial
            .next_serial(&self.pool, sequence_name, width, pad_char)
            .await?)
    }

    /// Ensure a PostgreSQL sequence exists
    pub async fn ensure_sequence(
        &self,
        sequence_name: &str,
        start: i64,
    ) -> Result<(), EncodingServiceError> {
        Ok(self
            .serial
            .ensure_sequence(&self.pool, sequence_name, start)
            .await?)
    }

    /// Apply an encoding rule to generate a code
    pub fn apply_rule(
        &self,
        rule: &EncodingRule,
        ctx: &EncodingContext,
    ) -> Result<EncodingResult, EncodingServiceError> {
        Ok(self.rules.apply(rule, ctx)?)
    }

    /// Convenience: apply a rule and automatically resolve sequences from PostgreSQL
    pub async fn apply_rule_with_sequences(
        &self,
        rule: &EncodingRule,
    ) -> Result<EncodingResult, EncodingServiceError> {
        let mut ctx = EncodingContext::default();

        for segment in &rule.segments {
            if let crate::rules::EncodingSegment::Sequence { sequence_name, .. } = segment {
                if !ctx.sequences.contains_key(sequence_name) {
                    let value = self.next_sequence(sequence_name).await?;
                    ctx.sequences.insert(sequence_name.clone(), value as u64);
                }
            }
        }

        Ok(self.rules.apply(rule, &ctx)?)
    }
}

#[cfg(test)]
mod tests {
    use crate::crc32;
    use crate::rules::{EncodingContext, EncodingRule, EncodingRuleEngine, EncodingSegment};
    use crate::zuid::{PeerType, ZuidGenerator};

    #[test]
    fn test_service_generate_zuid() {
        let zuid = ZuidGenerator::new(PeerType::Consumer, 0, 0, 0).unwrap();
        let id = zuid.generate();
        assert!(id > 0);
    }

    #[test]
    fn test_service_compute_crc32() {
        let result = crc32::compute_checksum(b"test");
        assert_eq!(result, 3632233996); // CRC32 of "test"
    }

    #[test]
    fn test_service_validate_crc32() {
        let data = b"test";
        let checksum = crc32::compute_checksum(data);
        assert!(crc32::validate_checksum(data, checksum));
        assert!(!crc32::validate_checksum(data, checksum.wrapping_add(1)));
    }

    #[test]
    fn test_service_apply_rule() {
        let engine = EncodingRuleEngine::new();
        let rule = EncodingRule {
            id: "svc-test".to_string(),
            name: "Service Test".to_string(),
            segments: vec![EncodingSegment::Prefix {
                value: "PRE".to_string(),
            }],
            checksum_algorithm: None,
        };
        let result = engine.apply(&rule, &EncodingContext::default()).unwrap();
        assert_eq!(result.code, "PRE");
    }
}
