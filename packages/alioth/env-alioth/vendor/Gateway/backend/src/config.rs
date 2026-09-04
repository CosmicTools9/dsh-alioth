use log;
use serde::Deserialize;
use std::env;

#[derive(Clone, Deserialize)]
pub struct Config {
    pub database_url: String,
    pub server_addr: String,
    /// SSO JWT ES256 公钥（PEM）。
    /// 可选：未配置时 Gateway 通过 SSO JWKS 端点动态获取公钥（"去私钥"）。
    pub sso_jwt_public_key: String,
    /// 轮换窗口内的历史 ES256 公钥（PEM 字节，可选）——静态回退多 key（prev）。
    pub sso_jwt_public_key_prev: Vec<u8>,
    /// SSO 令牌 issuer（与 SSO `config.oidc_issuer` 一致），用于校验 JWT `iss` 声明。
    pub sso_jwt_issuer: String,
    pub sso_service_url: String,
    pub cors_allowed_origins: Vec<String>,
}

/// 加载 SSO JWT ES256 公钥
/// 优先从 SSO_JWT_PUBLIC_KEY 环境变量读取（非空且不是 enc: 密文）；
/// 否则从 SSO_JWT_PUBLIC_KEY_PATH 指向的文件读取。
/// 两者均未配置时返回空字符串，由 Gateway 通过 SSO JWKS 动态获取公钥。
fn load_sso_jwt_public_key() -> anyhow::Result<String> {
    // 尝试环境变量
    if let Ok(key) = env::var("SSO_JWT_PUBLIC_KEY") {
        if !key.is_empty() && !key.starts_with("enc:") {
            return Ok(key);
        }
    }
    // 尝试文件路径
    match env::var("SSO_JWT_PUBLIC_KEY_PATH") {
        Ok(path) if !path.is_empty() => Ok(std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path, e))?
            .trim()
            .to_string()),
        _ => {
            log::warn!(
                "SSO_JWT_PUBLIC_KEY / SSO_JWT_PUBLIC_KEY_PATH 未配置：\
                 将依赖 SSO JWKS 端点动态获取公钥（去私钥）"
            );
            Ok(String::new())
        }
    }
}

/// 加载 SSO JWT 轮换窗口内的历史 ES256 公钥（可选，PEM 字节）
/// 从 SSO_JWT_PUBLIC_KEY_PREV_PATH 指向的文件读取；未配置或文件为空 → 空 Vec。
fn load_sso_jwt_public_key_prev() -> anyhow::Result<Vec<u8>> {
    match env::var("SSO_JWT_PUBLIC_KEY_PREV_PATH") {
        Ok(path) if !path.is_empty() => {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path, e))?;
            Ok(content.trim().as_bytes().to_vec())
        }
        _ => Ok(Vec::new()),
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let cors_origins = env::var("CORS_ALLOWED_ORIGINS")
            .expect("CORS_ALLOWED_ORIGINS must be set — check .mise.toml or .env")
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();
        Ok(Config {
            database_url: env::var("DATABASE_URL").expect(
                "DATABASE_URL must be set — check .mise.toml → .env → decrypt-env.sh chain",
            ),
            server_addr: env::var("SERVER_ADDR")
                .expect("SERVER_ADDR must be set — e.g. 127.0.0.1:9001"),
            sso_jwt_public_key: load_sso_jwt_public_key()?,
            sso_jwt_public_key_prev: load_sso_jwt_public_key_prev()?,
            sso_jwt_issuer: env::var("SSO_JWT_ISSUER")
                .unwrap_or_else(|_| "http://localhost:9002".to_string()),
            sso_service_url: env::var("SSO_SERVICE_URL").unwrap_or_default(),
            cors_allowed_origins: cors_origins,
        })
    }
}
