use encoding::crc32;

#[test]
fn test_crc32_ieee_test_vector() {
    // Known CRC32-IEEE test vector for "123456789"
    assert_eq!(crc32::compute_checksum(b"123456789"), 0xcbf43926);
}

#[test]
fn test_crc32_validate_matching_checksum() {
    let data = b"test data";
    let checksum = crc32::compute_checksum(data);
    assert!(crc32::validate_checksum(data, checksum));
}

#[test]
fn test_crc32_validate_mismatched_checksum() {
    let data = b"test data";
    let checksum = crc32::compute_checksum(data);
    assert!(!crc32::validate_checksum(data, checksum.wrapping_add(1)));
}

#[test]
fn test_crc32_compute_checksum_hex() {
    let hex = crc32::compute_checksum_hex_lower(b"hello world");
    assert_eq!(hex.len(), 8);
    assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(hex, hex.to_ascii_lowercase());
}
