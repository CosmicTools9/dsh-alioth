use serde::Deserialize;
use std::env;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// 服务器监听地址 (默认：0.0.0.0:9002)
    pub server_addr: String,

    /// 数据库连接字符串
    pub database_url: String,

    /// SSO JWT ES256 私钥 (PEM)
    pub sso_jwt_private_key: String,

    /// 加密密钥 (用于加密敏感数据)
    pub encryption_key: String,

    /// NGAC OA 页面预览截图目录（add-ngac-oa-preview，dev-only）：
    /// 采集器 scripts/ts/ngac-oa-preview-capture.sh 产物（png + manifest.json）；
    /// 未设置 → preview 功能禁用（端点字段 null）。
    pub ngac_preview_dir: Option<String>,

    /// JWT Access Token 过期时间 (秒) 默认 900 秒 (15 分钟)
    #[serde(with = "common::serde_zuid")]
    pub jwt_access_expiry: i64,

    /// JWT Refresh Token 过期时间 (秒) 默认 604800 秒 (7 天)
    #[serde(with = "common::serde_zuid")]
    pub jwt_refresh_expiry: i64,

    /// OAuth Google Client ID
    pub oauth_google_client_id: Option<String>,

    /// OAuth Google Client Secret
    pub oauth_google_client_secret: Option<String>,

    /// OAuth GitHub Client ID
    pub oauth_github_client_id: Option<String>,

    /// OAuth GitHub Client Secret
    pub oauth_github_client_secret: Option<String>,

    /// OAuth Microsoft Client ID
    pub oauth_microsoft_client_id: Option<String>,

    /// OAuth Microsoft Client Secret
    pub oauth_microsoft_client_secret: Option<String>,

    /// Microsoft Tenant ID
    pub oauth_microsoft_tenant_id: Option<String>,

    /// Okta Domain
    pub oauth_okta_domain: Option<String>,

    /// Okta Client ID
    pub oauth_okta_client_id: Option<String>,

    /// Okta Client Secret
    pub oauth_okta_client_secret: Option<String>,

    /// OAuth 回调 URL
    pub oauth_redirect_url: String,

    /// OIDC OP issuer URL（discovery 的 `issuer` 与 id_token `iss` 声明）
    pub oidc_issuer: String,

    /// OIDC client_id（单租户简化：授权端点接受此 client_id；多客户端注册待扩展）
    pub oidc_client_id: Option<String>,

    /// OIDC 允许的 redirect_uri 白名单（逗号分隔；授权端点强制校验，防开放重定向）
    pub oidc_redirect_uris: Vec<String>,

    /// 日志级别
    pub log_level: String,

    /// 身份证实模式：
    /// - "local"（默认）：本地核验提交数据的完整性；
    /// - "external"：调用第三方 API（需配置 identity_external_verify_url）。
    pub identity_verify_mode: String,

    /// 轮换窗口内的历史 ES256 公钥（PEM，可选）——JWKS 双钥过渡（prev）
    pub sso_jwt_public_key_prev: Option<String>,

    /// 第三方身份证实 API URL（identity_verify_mode=external 时使用）。
    pub identity_external_verify_url: Option<String>,

    /// 邮件发送模式：
    /// - "smtp"（默认）：真实 SMTP 发送（配置来自 zc_id_prot-email_config）；
    /// - "log"：dev 专用——邮件内容（含验证码）打印到日志，不实际投递。
    ///   未设置/非法值 → smtp（fail-closed，生产不允许隐式降级）。
    pub email_mode: String,
}

/// 加载 SSO JWT 轮换窗口内的历史 ES256 公钥（可选）
/// 从 SSO_JWT_PUBLIC_KEY_PREV_PATH 指向的文件读取；未配置或文件为空 → None。
/// prev 为公钥（可明文），仅用于轮换过渡期内验签旧 token。
fn load_sso_jwt_public_key_prev() -> Result<Option<String>, Box<dyn std::error::Error>> {
    match env::var("SSO_JWT_PUBLIC_KEY_PREV_PATH") {
        Ok(path) if !path.is_empty() => {
            let pem = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {path}: {e}"))?
                .trim()
                .to_string();
            Ok((!pem.is_empty()).then_some(pem))
        }
        _ => Ok(None),
    }
}

/// 加载 SSO JWT ES256 私钥
/// 优先从 SSO_JWT_PRIVATE_KEY 环境变量读取（非空且不是 enc: 密文）；
/// 否则从 SSO_JWT_PRIVATE_KEY_PATH 指向的文件读取。
/// 兜底默认路径为 Deploy/current/sso_jwt_private.pem（相对于 CWD）。
fn load_sso_jwt_private_key() -> Result<String, Box<dyn std::error::Error>> {
    if let Ok(key) = env::var("SSO_JWT_PRIVATE_KEY") {
        if !key.is_empty() && !key.starts_with("enc:") {
            return Ok(key);
        }
    }
    let path = env::var("SSO_JWT_PRIVATE_KEY_PATH")
        .unwrap_or_else(|_| "Deploy/current/sso_jwt_private.pem".to_string());
    Ok(std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {path}: {e}"))?
        .trim()
        .to_string())
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Config {
            server_addr: env::var("SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:9002".to_string()),
            database_url: env::var("DATABASE_URL")?,
            sso_jwt_private_key: load_sso_jwt_private_key()?,
            encryption_key: env::var("ENCRYPTION_KEY")?,
            ngac_preview_dir: env::var("NGAC_PREVIEW_DIR").ok().filter(|s| !s.is_empty()),
            jwt_access_expiry: env::var("JWT_ACCESS_EXPIRY")
                .unwrap_or_else(|_| "900".to_string())
                .parse()?,
            jwt_refresh_expiry: env::var("JWT_REFRESH_EXPIRY")
                .unwrap_or_else(|_| "604800".to_string())
                .parse()?,
            oauth_google_client_id: env::var("OAUTH_GOOGLE_CLIENT_ID").ok(),
            oauth_google_client_secret: env::var("OAUTH_GOOGLE_CLIENT_SECRET").ok(),
            oauth_github_client_id: env::var("OAUTH_GITHUB_CLIENT_ID").ok(),
            oauth_github_client_secret: env::var("OAUTH_GITHUB_CLIENT_SECRET").ok(),
            oauth_microsoft_client_id: env::var("OAUTH_MICROSOFT_CLIENT_ID").ok(),
            oauth_microsoft_client_secret: env::var("OAUTH_MICROSOFT_CLIENT_SECRET").ok(),
            oauth_microsoft_tenant_id: env::var("OAUTH_MICROSOFT_TENANT_ID").ok(),
            oauth_okta_domain: env::var("OAUTH_OKTA_DOMAIN").ok(),
            oauth_okta_client_id: env::var("OAUTH_OKTA_CLIENT_ID").ok(),
            oauth_okta_client_secret: env::var("OAUTH_OKTA_CLIENT_SECRET").ok(),
            oauth_redirect_url: env::var("OAUTH_REDIRECT_URL")
                .unwrap_or_else(|_| "http://localhost:9002/auth/callback".to_string()),
            oidc_issuer: env::var("OIDC_ISSUER")
                .unwrap_or_else(|_| "http://localhost:9002".to_string()),
            oidc_client_id: env::var("OIDC_CLIENT_ID").ok(),
            oidc_redirect_uris: env::var("OIDC_REDIRECT_URIS")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            log_level: env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
            identity_verify_mode: env::var("IDENTITY_VERIFY_MODE")
                .unwrap_or_else(|_| "local".to_string()),
            identity_external_verify_url: env::var("IDENTITY_EXTERNAL_VERIFY_URL").ok(),
            email_mode: env::var("SSO_EMAIL_MODE").unwrap_or_else(|_| "smtp".to_string()),
            sso_jwt_public_key_prev: load_sso_jwt_public_key_prev()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 测试共享全局 env 变量（Rust 测试默认并行，set_var/remove_var 交叉污染，
    /// 存量缺陷——所有写 env 的测试须持锁串行执行）。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_config_from_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        env::set_var("SERVER_ADDR", "0.0.0.0:9002");
        env::set_var("DATABASE_URL", "postgres://localhost:5432/test");
        env::set_var(
            "SSO_JWT_PRIVATE_KEY",
            "-----BEGIN PRIVATE KEY-----\n\
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC8iajvvNi68bOh\n\
msqpgDNwyPwgC2poPuj8hryIJphTEl/Pdj3ifUvH3QjTK3kbV8m/1kGuhCvMfC8A\n\
qP86TxeE4NSGk3Gs0+vwvK08z9d9CUmOJU2EP6Ru7TVxrc+jVeZUSajZ6daPAeq8\n\
BGR4LyLasoH5RfaKEIe+4AHV2nrHdqm/Xmt6CARyFj/JXatd4KLPUibyyGQ7QLZk\n\
Inl5e5lCzNNbRs0VfHkM4I7VYRKb0AHf04o+fAbtiZGZa9vQsaL4p9o2/axltbxO\n\
Z43yXe5N/u0ewVx0JTIfqmHD6L+Ss2pIwFC1Xg2p4wEeXrI9oDyvEC6IIXU0ZluF\n\
i1WmnSEFAgMBAAECggEAHCmnfLF6/niSRwBAT48uOTsHIF+UyBHM2QrA1CWF4Tgu\n\
rUidm3BqBzQH/vCFxP0RDvKBGNZPGzSaLhUeWRKXBdnb5Ch4odQDfiAOZG6swM9s\n\
MsQ3wZGfFn44MEHPG9OU2RsTfHlE10d/onBrZ5spfFp7lby+E1kq8/R1qq5TPP4M\n\
L4WUVh/VVQq9eF7t4kxg4RGAFq5cQCdXg8PB4jGNGLZ5bKEMw7PksPlNBFMoeOA9\n\
2+7qYLyKMpznQ2mxYvGPJULM90X+R6AD/oSjzJn4gXq0nMm6X4Rr2U9AjGFK0tMC\n\
VRH6g0pAKqh55v2t1a6qPOfHy/mhJYLHiVSn7NKFAKBgQDZjDK3AJVUMXv8O/Zh\n\
1r0bVrGFNHtE62Vqsq0B9LqUbqORF+LJTBjGc4u02M+sqslq0KCPWYBBJhJr5Vxv\n\
5F1EdhVqjlwkCyXByusVs5ftfj8eFb2Mq5k8b/HRfP9F9kDs0z7qwKFgQmTmgV/I\n\
rpQsI/uwKfaN7ZzCH6is/BRyFQKBgQDeJgU31R4uYQeidC6CQPMl9/BQ9ESyBoC6\n\
qxoFoY8T8h2X87rsK3z4XfoSJvNPkX2NYB3IIkq8OsGlO9eY/NddYZvO1VfHqjOu\n\
/dIHWxBHCrjBx8Am9P1MTCHSqJ4HmOqTmfNfeMJSJY+No+twH2HQTyqDgXpKSdR4\n\
A3hXSz/HhQKBgGlh0/xtMSKSNtSgjFVY8GqHFO4rD20l+tsETZ+NS9tMMyF8hKf7\n\
5aFwjCvxj2FS5ldEYas0nEBlwO4w0W/xMJND8q4pHvRG+R8kHFVNBSqPPvkRk8f0\n\
OM6o+V9YVn6NliBBrHY1V5Tvg2wu/LKSkBOppZyX4qOs2qZOp8qk/zRBAoGBAJ6q\n\
GSQpPRPGWGAEJ+GCjJDhRHE5vT5zYBBr5IRl4bwhfynIY3I5Vb5HEx52vzLfUF1g\n\
svECaDjDopmY1OleH2s18thIYGfLc8rMnvW9L28ay2w7rIDM4z4v8+INq0UeCSF1\n\
D1Nj2VweWZ8+JzD9MUE7j1c+ttEUb5iDH6Q+XCDhAoGAeQ8vxSHSB4gP7IrcHJKt\n\
O+Yi38lnR9XUjPA6SlcPG//JfIGFzsAQ9h1mYQXxH4fFnHaTyOAGQj75bXK8f9fH\n\
36l+1d+YZkRPhAzvCTih4v55hxWfrtbNdSkQ0pMSnAvAQKBQyx12txRgfU6VAnN\n\
eR+xFWpLhmI8/FTV3xI=\n\
-----END PRIVATE KEY-----",
        );
        env::set_var("ENCRYPTION_KEY", "test-encryption-key");

        let config = Config::from_env();
        assert!(config.is_ok());
        // 未设置 SSO_EMAIL_MODE → 默认 smtp（fail-closed）
        assert_eq!(config.as_ref().unwrap().email_mode, "smtp");

        env::remove_var("SERVER_ADDR");
        env::remove_var("DATABASE_URL");
        env::remove_var("SSO_JWT_PRIVATE_KEY");
        env::remove_var("ENCRYPTION_KEY");
    }

    #[test]
    fn test_load_private_key_from_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        env::set_var("SSO_JWT_PRIVATE_KEY", "begin-test-key");
        let result = load_sso_jwt_private_key().unwrap();
        assert_eq!(result, "begin-test-key");
        env::remove_var("SSO_JWT_PRIVATE_KEY");
    }

    #[test]
    fn test_load_private_key_skips_enc() {
        let _guard = ENV_LOCK.lock().unwrap();
        env::set_var("SSO_JWT_PRIVATE_KEY", "enc:abc:def");
        // Set path to a non-existent file so the fallback produces an error
        env::set_var(
            "SSO_JWT_PRIVATE_KEY_PATH",
            "/tmp/nonexistent-jwt-key-file-for-test.pem",
        );
        let result = load_sso_jwt_private_key();
        assert!(result.is_err()); // No file at the configured path
        env::remove_var("SSO_JWT_PRIVATE_KEY");
        env::remove_var("SSO_JWT_PRIVATE_KEY_PATH");
    }
}
