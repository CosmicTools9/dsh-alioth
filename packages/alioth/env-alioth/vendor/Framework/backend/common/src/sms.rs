//! 短信服务 seam
//!
//! 提供基于阿里云和腾讯云 SMS 的短信发送能力。
//! 配置从 `zc_id_prot-sms_config.settings` (jsonb) 读取，
//! 敏感凭证从 `enc_fields` (jsonb) 读取。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hmac::{digest::KeyInit, Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::error::{AliothError, Result};

type HmacSha1 = Hmac<Sha1>;

/// 短信服务 trait
#[async_trait]
pub trait SmsService: Send + Sync + 'static {
    /// 发送短信验证码
    async fn send(&self, phone: &str, template_code: &str, params: &str) -> Result<()>;
}

/// SMS 配置（从 `zc_id_prot-sms_config.settings` JSON 解析）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SmsConfig {
    provider: String,
    sign_name: String,
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default = "default_region")]
    region: String,
    #[serde(default)]
    sms_sdk_app_id: Option<String>,
}

fn default_region() -> String {
    "cn-hangzhou".to_string()
}

type Decryptor = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// 带缓存的云 SMS 服务实现
#[derive(Clone)]
pub struct CloudSmsService {
    pool: PgPool,
    cache: Arc<RwLock<ConfigCache>>,
    decryptor: Decryptor,
    http_client: reqwest::Client,
}

#[derive(Clone)]
struct ConfigCache {
    config: Option<SmsConfig>,
    credentials: Option<Value>,
    loaded_at: Option<DateTime<Utc>>,
}

impl CloudSmsService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            cache: Arc::new(RwLock::new(ConfigCache {
                config: None,
                credentials: None,
                loaded_at: None,
            })),
            decryptor: Arc::new(|s: &str| {
                if s.starts_with("enc:") {
                    s.strip_prefix("enc:").unwrap_or(s).to_string()
                } else {
                    s.to_string()
                }
            }),
            http_client: reqwest::Client::new(),
        }
    }

    pub fn with_decryptor<F>(pool: PgPool, decryptor: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        Self {
            pool,
            cache: Arc::new(RwLock::new(ConfigCache {
                config: None,
                credentials: None,
                loaded_at: None,
            })),
            decryptor: Arc::new(decryptor),
            http_client: reqwest::Client::new(),
        }
    }

    async fn load_config(&self) -> Result<(SmsConfig, Value)> {
        {
            let cache = self
                .cache
                .read()
                .map_err(|_| AliothError::Internal("SMS config cache poisoned".to_string()))?;
            if let (Some(ref cfg), Some(ref creds)) = (&cache.config, &cache.credentials) {
                if let Some(loaded_at) = cache.loaded_at {
                    if (Utc::now() - loaded_at).num_seconds() < 60 {
                        return Ok((cfg.clone(), creds.clone()));
                    }
                }
            }
        }

        let row = sqlx::query_as::<_, (Value, Value)>(
            r#"
            SELECT settings, enc_fields
            FROM isahl."zc_id_prot-sms_config"
            WHERE (settings->>'public')::boolean IS NOT FALSE
              AND deleted_at IS NULL
            ORDER BY id
            LIMIT 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AliothError::Internal(format!("Failed to load SMS config: {}", e)))?;

        let (config, credentials) = match row {
            Some((settings, enc_fields)) => {
                let cfg: SmsConfig = serde_json::from_value(settings).map_err(|e| {
                    AliothError::Internal(format!("Invalid SMS settings JSON: {}", e))
                })?;

                let enc_obj = enc_fields.as_object().ok_or_else(|| {
                    AliothError::Internal("enc_fields is not a JSON object".to_string())
                })?;

                let mut decrypted = serde_json::Map::new();
                for (k, v) in enc_obj {
                    if let Some(s) = v.as_str() {
                        decrypted.insert(k.clone(), Value::String((self.decryptor)(s)));
                    } else {
                        decrypted.insert(k.clone(), v.clone());
                    }
                }
                (cfg, Value::Object(decrypted))
            }
            None => {
                return Err(AliothError::NotFound("sms_config".to_string()));
            }
        };

        {
            let mut cache = self
                .cache
                .write()
                .map_err(|_| AliothError::Internal("SMS config cache poisoned".to_string()))?;
            cache.config = Some(config.clone());
            cache.credentials = Some(credentials.clone());
            cache.loaded_at = Some(Utc::now());
        }

        Ok((config, credentials))
    }
}

#[async_trait]
impl SmsService for CloudSmsService {
    async fn send(&self, phone: &str, template_code: &str, params: &str) -> Result<()> {
        let (cfg, credentials) = self.load_config().await?;

        match cfg.provider.as_str() {
            "aliyun" => {
                send_aliyun(
                    &self.http_client,
                    phone,
                    &cfg.sign_name,
                    template_code,
                    params,
                    cfg.endpoint.as_deref().unwrap_or("dysmsapi.aliyuncs.com"),
                    &cfg.region,
                    &credentials,
                )
                .await
            }
            "tencent" => {
                let app_id = cfg.sms_sdk_app_id.as_deref().ok_or_else(|| {
                    AliothError::BadRequest("sms_sdk_app_id required for tencent".to_string())
                })?;
                send_tencent(
                    &self.http_client,
                    phone,
                    &cfg.sign_name,
                    template_code,
                    params,
                    app_id,
                    cfg.endpoint.as_deref().unwrap_or("sms.tencentcloudapi.com"),
                    &cfg.region,
                    &credentials,
                )
                .await
            }
            other => Err(AliothError::BadRequest(format!(
                "Unsupported SMS provider: {}",
                other
            ))),
        }
    }
}

// ============================================================================
// 阿里云 SMS
// ============================================================================

#[allow(clippy::too_many_arguments)]
async fn send_aliyun(
    client: &reqwest::Client,
    phone: &str,
    sign_name: &str,
    template_code: &str,
    params: &str,
    endpoint: &str,
    region: &str,
    credentials: &Value,
) -> Result<()> {
    let access_key_id = credentials
        .get("access_key_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AliothError::Internal("access_key_id not found".to_string()))?;
    let access_key_secret = credentials
        .get("access_key_secret")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AliothError::Internal("access_key_secret not found".to_string()))?;

    let mut query: BTreeMap<String, String> = BTreeMap::new();
    query.insert("Action".to_string(), "SendSms".to_string());
    query.insert("Version".to_string(), "2017-05-25".to_string());
    query.insert("AccessKeyId".to_string(), access_key_id.to_string());
    query.insert("SignatureMethod".to_string(), "HMAC-SHA1".to_string());
    query.insert("SignatureVersion".to_string(), "1.0".to_string());
    query.insert(
        "Timestamp".to_string(),
        Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    );
    query.insert(
        "SignatureNonce".to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    query.insert("RegionId".to_string(), region.to_string());
    query.insert("PhoneNumbers".to_string(), phone.to_string());
    query.insert("SignName".to_string(), sign_name.to_string());
    query.insert("TemplateCode".to_string(), template_code.to_string());
    query.insert("TemplateParam".to_string(), params.to_string());

    // 构造规范化查询字符串
    let canonical_query: Vec<String> = query
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect();
    let canonical_query_string = canonical_query.join("&");

    // 构造待签名字符串
    let string_to_sign = format!("GET&%2F&{}", url_encode(&canonical_query_string));

    // HMAC-SHA1 签名
    let mut mac = HmacSha1::new_from_slice(format!("{}&", access_key_secret).as_bytes())
        .map_err(|e| AliothError::Internal(format!("HMAC key error: {}", e)))?;
    mac.update(string_to_sign.as_bytes());
    let signature = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        mac.finalize().into_bytes(),
    );

    // 构造最终 URL
    let final_url = format!(
        "https://{}?Signature={}&{}",
        endpoint,
        url_encode(&signature),
        canonical_query_string
    );

    let resp = client
        .get(&final_url)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| AliothError::External {
            subsystem: "AliyunSMS".to_string(),
            message: format!("HTTP error: {}", e),
        })?;

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(AliothError::External {
            subsystem: "AliyunSMS".to_string(),
            message: format!("HTTP {}: {}", status, body_text),
        });
    }

    // 解析阿里云响应
    let body_json: Value = serde_json::from_str(&body_text).map_err(|e| AliothError::External {
        subsystem: "AliyunSMS".to_string(),
        message: format!("Invalid JSON response: {}", e),
    })?;

    if body_json.get("Code").and_then(|v| v.as_str()) != Some("OK") {
        return Err(AliothError::External {
            subsystem: "AliyunSMS".to_string(),
            message: format!(
                "Aliyun error: {} - {}",
                body_json
                    .get("Code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown"),
                body_json
                    .get("Message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            ),
        });
    }

    Ok(())
}

fn url_encode(s: &str) -> String {
    percent_encoding::percent_encode(s.as_bytes(), percent_encoding::NON_ALPHANUMERIC).to_string()
}

// ============================================================================
// 腾讯云 SMS
// ============================================================================

#[allow(clippy::too_many_arguments)]
async fn send_tencent(
    client: &reqwest::Client,
    phone: &str,
    sign_name: &str,
    template_code: &str,
    params: &str,
    sms_sdk_app_id: &str,
    endpoint: &str,
    region: &str,
    credentials: &Value,
) -> Result<()> {
    let secret_id = credentials
        .get("secret_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AliothError::Internal("secret_id not found".to_string()))?;
    let secret_key = credentials
        .get("secret_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AliothError::Internal("secret_key not found".to_string()))?;

    let timestamp = Utc::now().timestamp();
    let date = Utc::now().format("%Y-%m-%d").to_string();
    let service = "sms";

    // 请求体
    let payload = serde_json::json!({
        "PhoneNumberSet": [phone],
        "SmsSdkAppId": sms_sdk_app_id,
        "SignName": sign_name,
        "TemplateId": template_code,
        "TemplateParamSet": serde_json::from_str::<Value>(params).unwrap_or(Value::Null),
    });
    let payload_json = payload.to_string();
    let payload_hash = hex::encode(Sha256::digest(payload_json.as_bytes()));

    // HTTP 请求头
    let host = endpoint.to_string();
    let content_type = "application/json; charset=utf-8";

    // Step 1: 构造规范请求
    let http_request_method = "POST";
    let canonical_uri = "/";
    let canonical_query_string = "";
    let canonical_headers = format!(
        "content-type:{}
host:{}
",
        content_type, host
    );
    let signed_headers = "content-type;host";
    let canonical_request = format!(
        "{}
{}
{}
{}
{}
{}",
        http_request_method,
        canonical_uri,
        canonical_query_string,
        canonical_headers,
        signed_headers,
        payload_hash
    );

    // Step 2: 构造待签名字符串
    let algorithm = "TC3-HMAC-SHA256";
    let credential_scope = format!("{}/{}/tc3_request", date, service);
    let hashed_canonical_request = hex::encode(Sha256::digest(canonical_request.as_bytes()));
    let string_to_sign = format!(
        "{}
{}
{}
{}",
        algorithm, timestamp, credential_scope, hashed_canonical_request
    );

    // Step 3: 计算签名
    let secret_date = hmac_sha256(format!("TC3{}", secret_key).as_bytes(), date.as_bytes());
    let secret_service = hmac_sha256(&secret_date, service.as_bytes());
    let secret_signing = hmac_sha256(&secret_service, b"tc3_request");
    let signature = hex::encode(hmac_sha256(&secret_signing, string_to_sign.as_bytes()));

    // Step 4: 构造 Authorization
    let authorization = format!(
        "{} Credential={}/{}, SignedHeaders={}, Signature={}",
        algorithm, secret_id, credential_scope, signed_headers, signature
    );

    // 发送请求
    let resp = client
        .post(format!("https://{}", endpoint))
        .header("Host", host)
        .header("Content-Type", content_type)
        .header("X-TC-Action", "SendSms")
        .header("X-TC-Version", "2021-01-11")
        .header("X-TC-Timestamp", timestamp.to_string())
        .header("X-TC-Region", region)
        .header("Authorization", authorization)
        .body(payload_json)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| AliothError::External {
            subsystem: "TencentSMS".to_string(),
            message: format!("HTTP error: {}", e),
        })?;

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(AliothError::External {
            subsystem: "TencentSMS".to_string(),
            message: format!("HTTP {}: {}", status, body_text),
        });
    }

    // 解析腾讯云响应
    let body_json: Value = serde_json::from_str(&body_text).map_err(|e| AliothError::External {
        subsystem: "TencentSMS".to_string(),
        message: format!("Invalid JSON response: {}", e),
    })?;

    if body_json
        .get("Response")
        .and_then(|r| r.get("Error"))
        .is_some()
    {
        let err = body_json["Response"]["Error"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        return Err(AliothError::External {
            subsystem: "TencentSMS".to_string(),
            message: format!(
                "Tencent error: {} - {}",
                err.get("Code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown"),
                err.get("Message").and_then(|v| v.as_str()).unwrap_or("")
            ),
        });
    }

    Ok(())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    type HmacSha256 = Hmac<sha2::Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC can take key of any size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}
