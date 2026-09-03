//! 邮件服务 seam
//!
//! 提供基于 SMTP 的邮件发送能力。
//! 配置从 `zc_id_prot-email_config.settings` (jsonb) 读取，
//! 敏感凭证从 `enc_fields` (jsonb) 读取。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lettre::{
    message::header::ContentType, transport::smtp::authentication::Credentials, AsyncSmtpTransport,
    AsyncTransport, Message, Tokio1Executor,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::{Arc, RwLock};

use crate::error::{AliothError, Result};

/// 邮件服务 trait
#[async_trait]
pub trait EmailService: Send + Sync + 'static {
    /// 发送纯文本邮件
    async fn send(&self, to: &str, subject: &str, body: &str) -> Result<()>;

    /// 发送 HTML 邮件
    async fn send_html(&self, to: &str, subject: &str, html: &str) -> Result<()>;
}

/// SMTP 配置（从 `zc_id_prot-email_config.settings` JSON 解析）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailConfig {
    pub smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    pub username: String,
    #[serde(default)]
    pub from_address: Option<String>,
    #[serde(default)]
    pub from_name: Option<String>,
    #[serde(default = "default_true")]
    pub use_tls: bool,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

fn default_smtp_port() -> u16 {
    587
}
fn default_true() -> bool {
    true
}
fn default_timeout() -> u64 {
    30
}

type PasswordDecryptor = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// 带缓存的 SMTP 邮件服务实现
#[derive(Clone)]
pub struct SmtpEmailService {
    pool: PgPool,
    cache: Arc<RwLock<ConfigCache>>,
    decryptor: PasswordDecryptor,
}

#[derive(Clone)]
struct ConfigCache {
    config: Option<EmailConfig>,
    password: Option<String>,
    loaded_at: Option<DateTime<Utc>>,
}

impl SmtpEmailService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            cache: Arc::new(RwLock::new(ConfigCache {
                config: None,
                password: None,
                loaded_at: None,
            })),
            decryptor: Arc::new(|s: &str| {
                if s.starts_with("enc:") {
                    s.strip_prefix("enc:").unwrap_or(s).to_string()
                } else {
                    s.to_string()
                }
            }),
        }
    }

    /// 使用自定义密码解密器创建服务
    pub fn with_password_decryptor<F>(pool: PgPool, decryptor: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        Self {
            pool,
            cache: Arc::new(RwLock::new(ConfigCache {
                config: None,
                password: None,
                loaded_at: None,
            })),
            decryptor: Arc::new(decryptor),
        }
    }

    /// 从缓存或数据库加载配置（缓存 60 秒）
    async fn load_config(&self) -> Result<(EmailConfig, String)> {
        {
            let cache = self
                .cache
                .read()
                .map_err(|_| AliothError::Internal("Email config cache poisoned".to_string()))?;
            if let (Some(ref cfg), Some(ref pwd)) = (&cache.config, &cache.password) {
                if let Some(loaded_at) = cache.loaded_at {
                    if (Utc::now() - loaded_at).num_seconds() < 60 {
                        return Ok((cfg.clone(), pwd.clone()));
                    }
                }
            }
        }

        let row = sqlx::query_as::<_, (Value, Value)>(
            r#"
            SELECT settings, enc_fields
            FROM isahl."zc_id_prot-email_config"
            WHERE (settings->>'public')::boolean IS NOT FALSE
              AND deleted_at IS NULL
            ORDER BY id
            LIMIT 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AliothError::Internal(format!("Failed to load email config: {}", e)))?;

        let (config, password) = match row {
            Some((settings, enc_fields)) => {
                let cfg: EmailConfig = serde_json::from_value(settings).map_err(|e| {
                    AliothError::Internal(format!("Invalid email settings JSON: {}", e))
                })?;

                let enc_obj = enc_fields.as_object().ok_or_else(|| {
                    AliothError::Internal("enc_fields is not a JSON object".to_string())
                })?;

                let password_enc = enc_obj
                    .get("password")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AliothError::Internal("Password not found in enc_fields".to_string())
                    })?;

                let password = (self.decryptor)(password_enc);
                (cfg, password)
            }
            None => {
                return Err(AliothError::NotFound("email_config".to_string()));
            }
        };

        {
            let mut cache = self
                .cache
                .write()
                .map_err(|_| AliothError::Internal("Email config cache poisoned".to_string()))?;
            cache.config = Some(config.clone());
            cache.password = Some(password.clone());
            cache.loaded_at = Some(Utc::now());
        }

        Ok((config, password))
    }

    fn build_mailer(
        &self,
        cfg: &EmailConfig,
        password: &str,
    ) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
        let creds = Credentials::new(cfg.username.clone(), password.to_string());

        let mailer = if cfg.use_tls {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.smtp_host)
                .map_err(|e| AliothError::External {
                    subsystem: "SMTP".to_string(),
                    message: format!("Invalid SMTP host: {}", e),
                })?
                .port(cfg.smtp_port)
                .credentials(creds)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.smtp_host)
                .port(cfg.smtp_port)
                .credentials(creds)
        };

        Ok(mailer.build())
    }

    fn build_message(
        &self,
        cfg: &EmailConfig,
        to: &str,
        subject: &str,
        content_type: ContentType,
        body: String,
    ) -> Result<Message> {
        let from_address = cfg.from_address.as_deref().unwrap_or(&cfg.username);
        let from_name = cfg.from_name.as_deref().unwrap_or("AliothStudio");

        let email = Message::builder()
            .from(
                format!("{} <{}>", from_name, from_address)
                    .parse()
                    .map_err(|_| AliothError::BadRequest("Invalid from address".to_string()))?,
            )
            .to(to
                .parse()
                .map_err(|_| AliothError::BadRequest(format!("Invalid to address: {}", to)))?)
            .subject(subject)
            .header(content_type)
            .body(body)
            .map_err(|e| AliothError::Internal(format!("Failed to build email: {}", e)))?;

        Ok(email)
    }
}

#[async_trait]
impl EmailService for SmtpEmailService {
    async fn send(&self, to: &str, subject: &str, body: &str) -> Result<()> {
        let (cfg, password) = self.load_config().await?;
        let email =
            self.build_message(&cfg, to, subject, ContentType::TEXT_PLAIN, body.to_string())?;
        let mailer = self.build_mailer(&cfg, &password)?;

        mailer
            .send(email)
            .await
            .map_err(|e| AliothError::External {
                subsystem: "SMTP".to_string(),
                message: format!("Failed to send email: {}", e),
            })?;

        Ok(())
    }

    async fn send_html(&self, to: &str, subject: &str, html: &str) -> Result<()> {
        let (cfg, password) = self.load_config().await?;
        let email =
            self.build_message(&cfg, to, subject, ContentType::TEXT_HTML, html.to_string())?;
        let mailer = self.build_mailer(&cfg, &password)?;

        mailer
            .send(email)
            .await
            .map_err(|e| AliothError::External {
                subsystem: "SMTP".to_string(),
                message: format!("Failed to send email: {}", e),
            })?;

        Ok(())
    }
}
