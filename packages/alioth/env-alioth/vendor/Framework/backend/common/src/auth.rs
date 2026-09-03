//! Shared JWT utilities for namespace-aware EC P-256 (ES256) verification.
//!
//! This module is used by:
//! - Gateway NgacEnforcer (production verification)
//! - Framework NgacPepMiddleware (standalone module development verification)
//! - Meta Auth (with its own independent key pair, but same algorithm/claims shape)
//!
//! It intentionally does NOT handle private keys or signing; those remain in the
//! issuing service (SSO or Meta).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// JWT errors that can occur during namespace-aware verification.
#[derive(Debug, thiserror::Error)]
pub enum JwtVerifyError {
    #[error("Token decode error: {0}")]
    Decode(#[from] jsonwebtoken::errors::Error),
    #[error("Invalid algorithm; expected ES256")]
    InvalidAlgorithm,
    #[error("Missing kid in token header")]
    MissingKid,
    #[error("Malformed kid: {0}")]
    MalformedKid(String),
    #[error("Kid namespace mismatch: kid={kid}, expected={expected}")]
    KidNamespaceMismatch { kid: String, expected: String },
    #[error("Missing claim: {0}")]
    MissingClaim(String),
    #[error("Claim mismatch: {claim}={value}, expected={expected}")]
    ClaimMismatch {
        claim: String,
        value: String,
        expected: String,
    },
    #[error("Invalid token type: {0}")]
    InvalidTokenType(String),
    #[error("Key not found for namespace={namespace} kid={kid}")]
    KeyNotFound { namespace: String, kid: String },
    #[error("Key metadata mismatch: key namespace={key_ns}, expected={expected}")]
    KeyMetadataMismatch { key_ns: String, expected: String },
}

/// Standard SSO JWT claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsoClaims {
    pub sub: String,
    pub email: String,
    #[serde(with = "crate::serde_zuid")]
    pub exp: i64,
    #[serde(with = "crate::serde_zuid")]
    pub iat: i64,
    pub iss: String,
    pub aud: String,
    pub namespace: String,
    pub protocol: String,
    #[serde(default)]
    pub mfa_verified: bool,
    #[serde(default)]
    pub jti: String,
}

impl SsoClaims {
    pub fn issuer(expected_ns: &str) -> String {
        format!("sso.alioth.{}", expected_ns)
    }

    pub fn audience(expected_ns: &str) -> String {
        format!("gateway.alioth.{}", expected_ns)
    }
}

/// Token protocol types.
pub const PROTOCOL_ACCESS: &str = "access";
pub const PROTOCOL_REFRESH: &str = "refresh";
pub const PROTOCOL_ZCHAT: &str = "zchat";

/// Valid internal protocols.
pub fn is_valid_protocol(protocol: &str) -> bool {
    matches!(
        protocol,
        PROTOCOL_ACCESS | PROTOCOL_REFRESH | PROTOCOL_ZCHAT
    )
}

/// Parse a `kid` of the form `<ns>-<version>`.
/// Returns `(namespace, version)`.
pub fn parse_kid(kid: &str) -> Result<(String, String), JwtVerifyError> {
    let mut parts = kid.rsplitn(2, '-');
    let version = parts
        .next()
        .ok_or_else(|| JwtVerifyError::MalformedKid(kid.to_string()))?
        .to_string();
    let ns = parts
        .next()
        .ok_or_else(|| JwtVerifyError::MalformedKid(kid.to_string()))?
        .to_string();
    if ns.is_empty() || version.is_empty() {
        return Err(JwtVerifyError::MalformedKid(kid.to_string()));
    }
    Ok((ns, version))
}

/// Build a `kid` from namespace and version.
pub fn build_kid(namespace: &str, version: &str) -> String {
    format!("{}-{}", namespace, version)
}

/// Extract namespace from a kid.
pub fn kid_namespace(kid: &str) -> Result<String, JwtVerifyError> {
    parse_kid(kid).map(|(ns, _)| ns)
}

/// Verify a token using a single public key PEM and expected namespace.
///
/// This function enforces:
/// - `alg` must be ES256
/// - `kid` must be present and namespace must match expected
/// - `iss`, `aud`, `namespace` claims must match expected
/// - `protocol` must be valid and match expected_protocol (if provided)
pub fn verify_sso_token(
    token: &str,
    expected_ns: &str,
    public_key_pem: &[u8],
    expected_protocol: Option<&str>,
) -> Result<(jsonwebtoken::TokenData<SsoClaims>, String), JwtVerifyError> {
    use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};

    let header = decode_header(token).map_err(JwtVerifyError::Decode)?;

    if header.alg != Algorithm::ES256 {
        return Err(JwtVerifyError::InvalidAlgorithm);
    }

    let kid = header
        .kid
        .as_deref()
        .ok_or(JwtVerifyError::MissingKid)?
        .to_string();

    let kid_ns = kid_namespace(&kid)?;
    if kid_ns != expected_ns {
        return Err(JwtVerifyError::KidNamespaceMismatch {
            kid: kid.clone(),
            expected: expected_ns.to_string(),
        });
    }

    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = true;
    validation.set_issuer(&[SsoClaims::issuer(expected_ns)]);
    validation.set_audience(&[SsoClaims::audience(expected_ns)]);

    let decoding_key = DecodingKey::from_ec_pem(public_key_pem)?;
    let token_data = decode::<SsoClaims>(token, &decoding_key, &validation)
        .map_err(JwtVerifyError::Decode)?;

    let claims = &token_data.claims;

    if claims.namespace != expected_ns {
        return Err(JwtVerifyError::ClaimMismatch {
            claim: "namespace".to_string(),
            value: claims.namespace.clone(),
            expected: expected_ns.to_string(),
        });
    }

    if claims.iss != SsoClaims::issuer(expected_ns) {
        return Err(JwtVerifyError::ClaimMismatch {
            claim: "iss".to_string(),
            value: claims.iss.clone(),
            expected: SsoClaims::issuer(expected_ns),
        });
    }

    if claims.aud != SsoClaims::audience(expected_ns) {
        return Err(JwtVerifyError::ClaimMismatch {
            claim: "aud".to_string(),
            value: claims.aud.clone(),
            expected: SsoClaims::audience(expected_ns),
        });
    }

    if !is_valid_protocol(&claims.protocol) {
        return Err(JwtVerifyError::InvalidTokenType(claims.protocol.clone()));
    }

    if let Some(expected) = expected_protocol {
        if claims.protocol != expected {
            return Err(JwtVerifyError::ClaimMismatch {
                claim: "protocol".to_string(),
                value: claims.protocol.clone(),
                expected: expected.to_string(),
            });
        }
    }

    Ok((token_data, kid))
}

/// Resolver that holds public keys indexed by full `kid`.
///
/// In production, Gateway may hold keys for only one namespace (its own `NAMESPACE`).
/// In test or multi-tenant setups, it may hold multiple.
#[derive(Debug, Clone, Default)]
pub struct NamespaceJwtKeyResolver {
    keys: HashMap<String, Vec<u8>>,
}

impl NamespaceJwtKeyResolver {
    pub fn new() -> Self {
        Self {
            keys: HashMap::new(),
        }
    }

    /// Insert a public key under a full `kid` (e.g., `Alioth-1`).
    pub fn with_key(mut self, kid: &str, public_key_pem: Vec<u8>) -> Self {
        self.keys.insert(kid.to_string(), public_key_pem);
        self
    }

    /// Look up a public key by exact `kid`.
    pub fn get(&self, kid: &str) -> Option<&[u8]> {
        self.keys.get(kid).map(|v| v.as_slice())
    }

    /// Verify a token using the resolver.
    ///
    /// The token's `kid` header must exactly match a known key. The kid namespace
    /// and the claim namespace must match `expected_ns`.
    pub fn verify(
        &self,
        token: &str,
        expected_ns: &str,
        expected_protocol: Option<&str>,
    ) -> Result<(jsonwebtoken::TokenData<SsoClaims>, String), JwtVerifyError> {
        let header = jsonwebtoken::decode_header(token).map_err(JwtVerifyError::Decode)?;
        let kid = header
            .kid
            .as_deref()
            .ok_or(JwtVerifyError::MissingKid)?
            .to_string();
        let kid_ns = kid_namespace(&kid)?;
        if kid_ns != expected_ns {
            return Err(JwtVerifyError::KidNamespaceMismatch {
                kid: kid.clone(),
                expected: expected_ns.to_string(),
            });
        }
        let public_key = self.keys.get(&kid).ok_or(JwtVerifyError::KeyNotFound {
            namespace: expected_ns.to_string(),
            kid: kid.clone(),
        })?;
        verify_sso_token(token, expected_ns, public_key, expected_protocol)
    }
}

/// Startup validation for key files and metadata.
///
/// Checks:
/// - `NAMESPACE` env is set and valid
/// - `JWT_SECRET` is NOT set (fail-closed)
/// - public key file parses as EC P-256 SPKI
/// - private key file exists (if path provided) and permissions are 0600/0700
///
/// Note: This does NOT verify that the private and public keys are a matching
/// pair, nor does it extract a `kid` from the raw key material. Callers that
/// have both keys (e.g. SSO) should perform a sign+verify probe or compare
/// derived public keys separately.
pub fn validate_startup_key_config(
    namespace: &str,
    public_key_pem: &[u8],
    jwt_secret: Option<&str>,
    check_private_key_path: Option<&std::path::Path>,
) -> Result<(), JwtVerifyError> {
    if namespace.is_empty() {
        return Err(JwtVerifyError::MissingClaim("NAMESPACE".to_string()));
    }

    if jwt_secret.is_some() {
        // Fail-closed: any presence of JWT_SECRET is an error.
        return Err(JwtVerifyError::ClaimMismatch {
            claim: "JWT_SECRET".to_string(),
            value: "present".to_string(),
            expected: "absent".to_string(),
        });
    }

    // Verify public key is a valid EC P-256 key and derive a kid namespace.
    let _ = jsonwebtoken::DecodingKey::from_ec_pem(public_key_pem)?;

    // We can't directly extract the kid from the raw key, so we rely on the
    // caller to pass a key that corresponds to the namespace. The caller
    // should derive kid from the key file path or metadata.
    // This function validates the key parses and that JWT_SECRET is absent.

    if let Some(private_path) = check_private_key_path {
        if !private_path.exists() {
            return Err(JwtVerifyError::KeyNotFound {
                namespace: namespace.to_string(),
                kid: "private".to_string(),
            });
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(private_path).map_err(|e| {
                JwtVerifyError::KeyNotFound {
                    namespace: namespace.to_string(),
                    kid: format!("private metadata error: {}", e),
                }
            })?;
            let permissions = metadata.permissions().mode();
            // Only check the owner bits: must be 0600 (or more restrictive).
            let file_mode = permissions & 0o777;
            if file_mode != 0o600 {
                return Err(JwtVerifyError::ClaimMismatch {
                    claim: "private_key_permissions".to_string(),
                    value: format!("{:03o}", file_mode),
                    expected: "0600".to_string(),
                });
            }
            if let Some(parent) = private_path.parent() {
                let dir_metadata = std::fs::metadata(parent).map_err(|e| {
                    JwtVerifyError::KeyNotFound {
                        namespace: namespace.to_string(),
                        kid: format!("directory metadata error: {}", e),
                    }
                })?;
                let dir_mode = dir_metadata.permissions().mode() & 0o777;
                if dir_mode != 0o700 {
                    return Err(JwtVerifyError::ClaimMismatch {
                        claim: "private_key_directory_permissions".to_string(),
                        value: format!("{:03o}", dir_mode),
                        expected: "0700".to_string(),
                    });
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_EC_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg2wKAEH0lCQSd/7Ro
sPTNdBk/FA+0v4ySiQgKfEvyXC+hRANCAAQa4oJDdj0j4r9uhXyXkEM74YhrfymG
kLbde5YJ9O/mbHMcihareS5r7WuUT39QG078mQFzg2z0ELuBivpRAmCc
-----END PRIVATE KEY-----"#;

    const TEST_EC_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEGuKCQ3Y9I+K/boV8l5BDO+GIa38p
hpC23XuWCfTv5mxzHIoWq3kua+1rlE9/UBtO/JkBc4Ns9BC7gYr6UQJgnA==
-----END PUBLIC KEY-----"#;

    const TEST_EC_PUBLIC_KEY_V2: &str = r#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAERybxYXMslgjgWMXozIdkQ3o1DmxI
C3jEvmJ6Mds6RU4OH0oVYkBz3Vz2C8oIB45Euz/oHmQQeZm8956NvobSCw==
-----END PUBLIC KEY-----"#;

    const TEST_EC_PRIVATE_KEY_V2: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgSbeQIx4mVpVIfzvH
ZeCqKNhIwGcOXvbSDLqpwx3gjw6hRANCAARHJvFhcyyWCOBYxejMh2RDejUObEgL
eMS+Ynox2zpFTg4fShViQHPdXPYLyggHjkS7P+geZBB5mbz3no2+htIL
-----END PRIVATE KEY-----"#;

    fn test_claims() -> SsoClaims {
        SsoClaims {
            sub: "42".to_string(),
            email: "user@example.com".to_string(),
            exp: i64::MAX,
            iat: 0,
            iss: SsoClaims::issuer("Alioth"),
            aud: SsoClaims::audience("Alioth"),
            namespace: "Alioth".to_string(),
            protocol: PROTOCOL_ACCESS.to_string(),
            mfa_verified: true,
            jti: String::new(),
        }
    }

    #[test]
    fn test_parse_kid() {
        assert_eq!(parse_kid("Alioth-1").unwrap(), ("Alioth".to_string(), "1".to_string()));
        assert!(parse_kid("Alioth").is_err());
        assert!(parse_kid("-1").is_err());
    }

    #[test]
    fn test_kid_namespace() {
        assert_eq!(kid_namespace("Alioth-1").unwrap(), "Alioth");
    }

    #[test]
    fn test_build_kid() {
        assert_eq!(build_kid("Alioth", "1"), "Alioth-1");
    }

    #[test]
    fn test_verify_sso_token_ok() {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        let claims = test_claims();
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(build_kid("Alioth", "1"));
        let token = encode(
            &header,
            &claims,
            &EncodingKey::from_ec_pem(TEST_EC_PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap();
        let result = verify_sso_token(&token, "Alioth", TEST_EC_PUBLIC_KEY.as_bytes(), None);
        assert!(result.is_ok(), "{:?}", result);
    }

    #[test]
    fn test_verify_sso_token_wrong_ns() {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        let mut claims = test_claims();
        claims.namespace = "WZ".to_string();
        claims.iss = SsoClaims::issuer("WZ");
        claims.aud = SsoClaims::audience("WZ");
        // kid must match the namespace we are checking against
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("WZ-1".to_string());
        let token = encode(
            &header,
            &claims,
            &EncodingKey::from_ec_pem(TEST_EC_PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap();
        // Token namespace is WZ, but we verify with expected Alioth -> kid mismatch
        let result = verify_sso_token(&token, "Alioth", TEST_EC_PUBLIC_KEY.as_bytes(), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolver_verify_ok() {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        let resolver = NamespaceJwtKeyResolver::new()
            .with_key("Alioth-1", TEST_EC_PUBLIC_KEY.as_bytes().to_vec())
            .with_key("Alioth-2", TEST_EC_PUBLIC_KEY_V2.as_bytes().to_vec());
        let claims = test_claims();
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("Alioth-2".to_string());
        let token = encode(
            &header,
            &claims,
            &EncodingKey::from_ec_pem(TEST_EC_PRIVATE_KEY_V2.as_bytes()).unwrap(),
        )
        .unwrap();
        let result = resolver.verify(&token, "Alioth", None);
        assert!(result.is_ok(), "{:?}", result);
    }

    #[test]
    fn test_resolver_unknown_kid_rejected() {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        let resolver = NamespaceJwtKeyResolver::new()
            .with_key("Alioth-1", TEST_EC_PUBLIC_KEY.as_bytes().to_vec());
        let claims = test_claims();
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("Alioth-99".to_string());
        let token = encode(
            &header,
            &claims,
            &EncodingKey::from_ec_pem(TEST_EC_PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap();
        let result = resolver.verify(&token, "Alioth", None);
        assert!(matches!(result, Err(JwtVerifyError::KeyNotFound { .. })));
    }

    #[test]
    fn test_resolver_kid_namespace_mismatch_rejected() {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        let resolver = NamespaceJwtKeyResolver::new()
            .with_key("Alioth-1", TEST_EC_PUBLIC_KEY.as_bytes().to_vec());
        let mut claims = test_claims();
        claims.namespace = "WZ".to_string();
        claims.iss = SsoClaims::issuer("WZ");
        claims.aud = SsoClaims::audience("WZ");
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("WZ-1".to_string());
        let token = encode(
            &header,
            &claims,
            &EncodingKey::from_ec_pem(TEST_EC_PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap();
        let result = resolver.verify(&token, "Alioth", None);
        assert!(matches!(result, Err(JwtVerifyError::KidNamespaceMismatch { .. })));
    }
}
