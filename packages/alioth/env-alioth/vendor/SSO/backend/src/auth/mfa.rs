//! TOTP-based MFA implementation

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MfaError {
    #[error("Failed to generate TOTP secret")]
    SecretGenerationError,
    #[error("Failed to verify TOTP code")]
    VerificationError,
    #[error("Failed to encrypt secret")]
    EncryptionError,
    #[error("Failed to decrypt secret")]
    DecryptionError,
    #[error("Failed to generate QR code: {0}")]
    QrCodeError(String),
}

pub struct TotpSetup {
    pub secret: String,
    pub qr_code_data: String,
}

pub fn generate_totp_secret(account_name: &str, issuer: &str) -> Result<TotpSetup, MfaError> {
    use base32::Alphabet;
    use totp_rs::{Algorithm, Secret, TOTP};

    // Generate a random 20-byte secret
    let raw_secret: Vec<u8> = (0..20).map(|_| rand::random::<u8>()).collect();
    let secret = Secret::Raw(raw_secret);
    let secret_bytes = secret
        .to_bytes()
        .map_err(|_| MfaError::SecretGenerationError)?;
    let encoded_secret = base32::encode(Alphabet::Rfc4648 { padding: false }, &secret_bytes);

    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret_bytes,
        Some(issuer.to_string()),
        account_name.to_string(),
    )
    .map_err(|_| MfaError::SecretGenerationError)?;

    let provisioning_uri = totp.get_url();

    Ok(TotpSetup {
        secret: encoded_secret,
        qr_code_data: provisioning_uri,
    })
}

pub fn generate_qr_code_image(setup: &TotpSetup) -> Result<Vec<u8>, MfaError> {
    use qrcode::render::svg;
    use qrcode::QrCode;

    let code =
        QrCode::new(&setup.qr_code_data).map_err(|e| MfaError::QrCodeError(e.to_string()))?;

    let image = code
        .render()
        .min_dimensions(200, 200)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build();

    Ok(image.into_bytes())
}

pub fn verify_totp_code(secret: &[u8], code: &str) -> bool {
    verify_totp_code_with_leeway(secret, code, 1)
}

pub fn verify_totp_code_with_leeway(secret: &[u8], code: &str, leeway: i64) -> bool {
    use totp_rs::{Algorithm, TOTP};

    let totp = match TOTP::new(
        Algorithm::SHA1,
        6,
        leeway as u8,
        30,
        secret.to_vec(),
        None,
        String::new(),
    ) {
        Ok(t) => t,
        Err(_) => return false,
    };

    totp.check_current(code).unwrap_or(false)
}

pub fn generate_mfa_bypass_codes(count: usize) -> Vec<String> {
    use rand::RngExt;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::rng();

    (0..count)
        .map(|_| {
            (0..8)
                .map(|_| {
                    let idx = rng.random_range(0..CHARSET.len());
                    CHARSET[idx] as char
                })
                .collect::<String>()
                .chars()
                .enumerate()
                .map(|(i, c)| {
                    if i > 0 && i % 4 == 0 {
                        format!("-{}", c)
                    } else {
                        c.to_string()
                    }
                })
                .collect::<String>()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_verify_totp() {
        let setup = generate_totp_secret("test@example.com", "AliothStudio").unwrap();
        let secret =
            base32::decode(base32::Alphabet::Rfc4648 { padding: false }, &setup.secret).unwrap();

        // We can't predict the current TOTP code, but we can verify an obviously wrong one
        assert!(!verify_totp_code(&secret, "000000"));
    }

    #[test]
    fn test_generate_qr_code() {
        let setup = generate_totp_secret("test@example.com", "AliothStudio").unwrap();
        let image = generate_qr_code_image(&setup).unwrap();
        assert!(!image.is_empty());
    }

    #[test]
    fn test_generate_bypass_codes() {
        let codes = generate_mfa_bypass_codes(5);
        assert_eq!(codes.len(), 5);
        for code in &codes {
            // 8 个字符，在第 4 个位置插入 1 个 dash，总长度 9
            assert_eq!(code.len(), 9, "Bypass code format should be XXXX-XXXX");
            assert!(code.contains('-'), "Bypass code should contain a dash");
        }
    }
}
