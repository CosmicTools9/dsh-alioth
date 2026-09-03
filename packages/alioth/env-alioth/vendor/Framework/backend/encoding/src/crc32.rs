use crc32fast::Hasher;

/// Compute IEEE CRC32 checksum for byte slice
pub fn compute_checksum(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

/// Validate that data matches the expected CRC32 checksum
pub fn validate_checksum(data: &[u8], expected: u32) -> bool {
    compute_checksum(data) == expected
}

/// Compute CRC32 and return as zero-padded uppercase hex string
pub fn compute_checksum_hex(data: &[u8]) -> String {
    format!("{:08X}", compute_checksum(data))
}

/// Compute CRC32 and return as zero-padded lowercase hex string
pub fn compute_checksum_hex_lower(data: &[u8]) -> String {
    format!("{:08x}", compute_checksum(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32_known_value() {
        // Known IEEE CRC32 for "hello world"
        assert_eq!(compute_checksum(b"hello world"), 0x0D4A1185);
    }

    #[test]
    fn test_crc32_empty() {
        assert_eq!(compute_checksum(b""), 0x0);
    }

    #[test]
    fn test_crc32_validation() {
        let data = b"test data";
        let checksum = compute_checksum(data);
        assert!(validate_checksum(data, checksum));
        assert!(!validate_checksum(data, checksum.wrapping_add(1)));
    }

    #[test]
    fn test_crc32_hex() {
        assert_eq!(compute_checksum_hex(b"hello world"), "0D4A1185");
        assert_eq!(compute_checksum_hex_lower(b"hello world"), "0d4a1185");
    }
}
