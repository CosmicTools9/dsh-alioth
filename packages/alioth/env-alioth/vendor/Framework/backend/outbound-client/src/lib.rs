//! 出向调用治理框架（gateway-openapi-outbound-unify）——provider 无关。
//!
//! 任意 namespace 对接第三方（FSSC/其他财务/供应商系统）复用统一治理：
//! - `isahl_auth.outbound_client` 注册表加载（fail-closed 凭据解密）
//! - HTTP 超时（connect/request）+ 幂等读重试（指数退避）+ 写不重试
//! - HTTP 非 2xx 转错误 + 出向计量（`isahl_auth.outbound_call_log`）
//! - 运行模式显式声明（`OUTBOUND_RUNTIME_MODE` production|mock，未设置 fail-closed）
//! - `token` 模块：iERP 共享平台两步 token 握手（2.1 app_token + 2.2 通行证）+ 进程内缓存

pub mod crypto;
pub mod repository;
pub mod token;

use common::AliothError as ApiError;
use serde_json::Value;

/// 出向运行模式：显式声明，禁止隐式默认。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundRuntimeMode {
    /// 生产：注册表 fail-closed，真实调用必须计量
    Production,
    /// mock/dev：注册表优先，空表允许回退 env
    Mock,
}

/// 解析 `OUTBOUND_RUNTIME_MODE`（production|mock）；兼容旧名 `WZ_FSSC_RUNTIME_MODE`。
/// 未设置或非法值 → Err（安全默认 fail-closed，不隐式降级 mock）。
pub fn runtime_mode() -> Result<OutboundRuntimeMode, ApiError> {
    let val = std::env::var("OUTBOUND_RUNTIME_MODE")
        .or_else(|_| std::env::var("WZ_FSSC_RUNTIME_MODE"))
        .ok();
    match val.as_deref() {
        Some("production") => Ok(OutboundRuntimeMode::Production),
        Some("mock") => Ok(OutboundRuntimeMode::Mock),
        Some(other) => Err(ApiError::Internal(format!(
            "OUTBOUND_RUNTIME_MODE 非法值 '{}'（须 production|mock）",
            other
        ))),
        None => Err(ApiError::Internal(
            "OUTBOUND_RUNTIME_MODE 未设置：出向调用禁止隐式 mock（须显式 production|mock）"
                .to_string(),
        )),
    }
}

/// 出向调用方配置（通用化 FsscClientConfig——provider 无关）。
#[derive(Debug, Clone)]
pub struct OutboundClientConfig {
    pub base_url: String,
    pub app_id: String,
    pub app_secret: String,
    pub tenant_id: String,
    pub account_id: String,
    pub language: String,
    pub user: String,
    pub usertype: String,
    pub workflow_view_user: String,
    pub mock: bool,
}

impl Default for OutboundClientConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            app_id: String::new(),
            app_secret: String::new(),
            tenant_id: String::new(),
            account_id: String::new(),
            language: String::new(),
            user: String::new(),
            usertype: String::new(),
            workflow_view_user: String::new(),
            mock: true,
        }
    }
}

impl OutboundClientConfig {
    /// 从 `isahl_auth.outbound_client` 注册表加载（code 参数化，如 'fssc-wzgroup'）。
    ///
    /// fail-closed 分层：
    /// - 注册表有 enabled 行 → 使用该行配置；`app_secret_enc` 解密**失败即报错**
    ///   （不静默回退 env——错误密钥配置必须暴露）。
    /// - 注册表无有效行 → `allow_env_fallback=true`（mock）回退 env（读 OUTBOUND_* /
    ///   旧名 WZ_FSSC_*）；`false`（生产）报错。
    pub async fn load_from_registry(
        pool: &sqlx::PgPool,
        code: &str,
        allow_env_fallback: bool,
    ) -> Result<Self, ApiError> {
        let row = sqlx::query_as::<
            _,
            (
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ),
        >(
            "SELECT base_url, app_id, app_secret_enc, tenant_id, account_id, \
                    language, \"user\", usertype, workflow_view_user \
             FROM isahl_auth.outbound_client \
             WHERE code = $1 AND enabled = TRUE AND deleted_at IS NULL \
             ORDER BY version DESC LIMIT 1",
        )
        .bind(code)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::from_sqlx)?;

        let Some((
            base_url,
            app_id,
            secret_enc,
            tenant_id,
            account_id,
            language,
            user,
            usertype,
            wf_user,
        )) = row
        else {
            if allow_env_fallback {
                return Ok(Self::from_env());
            }
            return Err(ApiError::Internal(format!(
                "出向调用方注册表无有效配置（outbound_client code='{}' 空或全 disabled）：生产模式禁止回退环境变量",
                code
            )));
        };

        let secret_enc = secret_enc.ok_or_else(|| {
            ApiError::Internal(format!(
                "outbound_client '{}' 缺 app_secret_enc（请配置密文）",
                code
            ))
        })?;
        let app_secret = crate::crypto::decrypt(&secret_enc)?;

        let cfg = Self {
            mock: false,
            base_url: base_url.unwrap_or_default(),
            app_id: app_id.unwrap_or_default(),
            app_secret,
            tenant_id: tenant_id.unwrap_or_default(),
            account_id: account_id.unwrap_or_default(),
            language: language.unwrap_or_default(),
            user: user.unwrap_or_default(),
            usertype: usertype.unwrap_or_default(),
            workflow_view_user: wf_user.unwrap_or_default(),
        };
        Ok(cfg)
    }

    /// 从环境变量读取（OUTBOUND_* 优先，兼容旧名 WZ_FSSC_*）。
    pub fn from_env() -> Self {
        let get_env = |key: &str, legacy: &str| {
            std::env::var(key)
                .ok()
                .or_else(|| std::env::var(legacy).ok())
        };
        let cfg = Self {
            mock: !std::env::var("OUTBOUND_MOCK")
                .or_else(|_| std::env::var("WZ_FSSC_MOCK"))
                .map(|v| v.trim().eq_ignore_ascii_case("false"))
                .unwrap_or(false),
            base_url: get_env("OUTBOUND_URL", "WZ_FSSC_URL").unwrap_or_default(),
            app_id: get_env("OUTBOUND_APP_ID", "WZ_FSSC_APP_ID").unwrap_or_default(),
            app_secret: get_env("OUTBOUND_APP_SECRET", "WZ_FSSC_APP_SECRET").unwrap_or_default(),
            tenant_id: get_env("OUTBOUND_TENANT_ID", "WZ_FSSC_TENANT_ID").unwrap_or_default(),
            account_id: get_env("OUTBOUND_ACCOUNT_ID", "WZ_FSSC_ACCOUNT_ID").unwrap_or_default(),
            language: get_env("OUTBOUND_LANGUAGE", "WZ_FSSC_LANGUAGE").unwrap_or_default(),
            user: get_env("OUTBOUND_USER", "WZ_FSSC_USER").unwrap_or_default(),
            usertype: get_env("OUTBOUND_USERTYPE", "WZ_FSSC_USERTYPE").unwrap_or_default(),
            workflow_view_user: get_env(
                "OUTBOUND_WORKFLOW_VIEW_USER",
                "WZ_FSSC_WORKFLOW_VIEW_USER",
            )
            .unwrap_or_default(),
        };
        cfg
    }
}

/// 指数退避延迟（base 500ms，倍率 2，确定性抖动——测试可复现）；attempt 从 0 起。
pub fn backoff_delay_ms(attempt: u32) -> u64 {
    let base = 500u64 * 2u64.pow(attempt);
    let jitter = match attempt % 3 {
        0 => 0,
        1 => base / 5,
        _ => base / 5 * 2,
    };
    base + jitter
}

/// 出向 HTTP 客户端（provider 无关治理）。
#[derive(Clone)]
pub struct OutboundClient {
    config: OutboundClientConfig,
    client: reqwest::Client,
    /// 出向计量落库；真实模式（mock=false）MUST Some（fail-closed）
    pool: Option<sqlx::PgPool>,
}

impl OutboundClient {
    /// 基础构造（mock 或无计量场景；真实模式请用 `with_pool`）。
    pub fn new(config: OutboundClientConfig) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            // 出向目标为企业内网（FSSC 10.x）：dev 机 macOS 系统代理（sing-box 等）
            // 会把内网流量送进代理导致超时，reqwest 默认读系统代理，必须显式禁用。
            .no_proxy()
            .build()
            .expect("reqwest client build");
        Self {
            config,
            client,
            pool: None,
        }
    }

    /// 带出向计量池构造。真实模式（mock=false）MUST 提供 pool（fail-closed——
    /// 真实调用必须有计量落库）；mock 允许 None。配置非法返回 Err（不 panic）。
    pub fn with_pool(
        config: OutboundClientConfig,
        pool: Option<sqlx::PgPool>,
    ) -> Result<Self, ApiError> {
        if !config.mock && pool.is_none() {
            return Err(ApiError::Internal(
                "OutboundClient 真实模式必须提供 PgPool（出向计量落库）——with_pool(None) 仅限 mock"
                    .to_string(),
            ));
        }
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            // 同 new()：内网出向禁用系统代理。
            .no_proxy()
            .build()
            .map_err(|e| ApiError::Internal(format!("reqwest client build failed: {}", e)))?;
        Ok(Self {
            config,
            client,
            pool,
        })
    }

    pub fn is_mock(&self) -> bool {
        self.config.mock
    }

    pub fn config(&self) -> &OutboundClientConfig {
        &self.config
    }

    /// 通用出向 POST（token 由具体 provider 注入 header）。
    ///
    /// - `retryable=true`（幂等读）：网络错误/5xx/超时按指数退避重试 3 次。
    /// - `retryable=false`（写操作）：不重试，直接上抛（防重复提交）。
    /// - 计量：结果确定后写 `isahl_auth.outbound_call_log`（不含 payload/凭据）。
    pub async fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: Value,
        retryable: bool,
        interface: &str,
        provider: &str,
    ) -> Result<Value, ApiError> {
        self.post_json_governed(url, headers, body, retryable, interface, provider)
            .await
    }

    /// 治理化 GET(只读):与 POST 同套超时/重试/计量,method 记 'GET'
    pub async fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        retryable: bool,
        interface: &str,
        provider: &str,
    ) -> Result<Value, ApiError> {
        self.send_governed("GET", url, headers, None, retryable, interface, provider)
            .await
    }

    async fn post_json_governed(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: Value,
        retryable: bool,
        interface: &str,
        provider: &str,
    ) -> Result<Value, ApiError> {
        self.send_governed(
            "POST",
            url,
            headers,
            Some(body),
            retryable,
            interface,
            provider,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)] // 出向请求参数（通道/重试/回退/鉴权）
    async fn send_governed(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<Value>,
        retryable: bool,
        interface: &str,
        provider: &str,
    ) -> Result<Value, ApiError> {
        if self.config.mock {
            return Err(ApiError::Internal(
                "OutboundClient 处于 mock 模式，不能发起真实调用".into(),
            ));
        }
        let request_id = format!(
            "outbound-{}-{}",
            interface.replace('/', "_"),
            chrono::Utc::now().timestamp_millis()
        );
        let started = std::time::Instant::now();

        let max_attempts = if retryable { 3 } else { 1 };
        let mut last_err: Option<ApiError> = None;
        for attempt in 0..max_attempts {
            let mut req = match method {
                "GET" => self.client.get(url),
                _ => self.client.post(url),
            };
            for (k, v) in headers {
                req = req.header(k, v);
            }
            if let Some(b) = &body {
                req = req
                    .header("Content-Type", "application/json; charset=utf-8")
                    .json(b);
            }
            let send_res = req.send().await;
            match send_res {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        let body = resp.text().await.unwrap_or_default();
                        let err = ApiError::ServiceUnavailable(format!(
                            "{} HTTP {}: {}",
                            interface,
                            status,
                            body.chars().take(200).collect::<String>()
                        ));
                        if status.is_server_error() && retryable && attempt + 1 < max_attempts {
                            let delay = backoff_delay_ms(attempt);
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            last_err = Some(err);
                            continue;
                        }
                        last_err = Some(err);
                        break;
                    }
                    let json_res = resp.json::<Value>().await;
                    match json_res {
                        Ok(v) => {
                            self.log_call(provider, interface, method, "ok", started, &request_id)
                                .await;
                            return Ok(v);
                        }
                        Err(e) => {
                            last_err = Some(ApiError::ServiceUnavailable(format!(
                                "{} response decode failed: {}",
                                interface, e
                            )));
                            if retryable && attempt + 1 < max_attempts {
                                let delay = backoff_delay_ms(attempt);
                                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                                continue;
                            }
                            break;
                        }
                    }
                }
                Err(e) => {
                    last_err = Some(ApiError::ServiceUnavailable(format!(
                        "{} request failed: {}",
                        interface, e
                    )));
                    if retryable && attempt + 1 < max_attempts {
                        let delay = backoff_delay_ms(attempt);
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }
                    break;
                }
            }
        }

        self.log_call(provider, interface, method, "error", started, &request_id)
            .await;
        Err(last_err
            .unwrap_or_else(|| ApiError::ServiceUnavailable(format!("{} failed", interface))))
    }

    /// 出向计量（best-effort，失败仅 warn 不阻塞）
    async fn log_call(
        &self,
        provider: &str,
        interface: &str,
        method: &str,
        status: &str,
        started: std::time::Instant,
        request_id: &str,
    ) {
        let Some(pool) = &self.pool else { return };
        let res = sqlx::query(
            "INSERT INTO isahl_auth.outbound_call_log \
             (id, provider, interface, method, status, latency_ms, requested_at, request_id) \
             VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, NOW(), $6)",
        )
        .bind(provider)
        .bind(interface)
        .bind(method)
        .bind(status)
        .bind(started.elapsed().as_millis() as i64)
        .bind(request_id)
        .execute(pool)
        .await;
        if let Err(e) = res {
            log::warn!("[outbound-client] 出向计量写入失败: {}", e);
        }
    }
}

/// 便捷构造：注册表加载 + 运行模式判定 + with_pool（兼容旧 env 名）。
pub async fn client_from_registry(
    pool: &sqlx::PgPool,
    code: &str,
) -> Result<OutboundClient, ApiError> {
    let mode = runtime_mode()?;
    let allow_env_fallback = mode == OutboundRuntimeMode::Mock;
    let config = OutboundClientConfig::load_from_registry(pool, code, allow_env_fallback).await?;
    OutboundClient::with_pool(config, Some(pool.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_delay_is_exponential() {
        let d0 = backoff_delay_ms(0);
        let d1 = backoff_delay_ms(1);
        let d2 = backoff_delay_ms(2);
        assert!((500..600).contains(&d0), "attempt0 应 ~500ms: {}", d0);
        assert!((1000..1300).contains(&d1), "attempt1 应 ~1000ms: {}", d1);
        assert!((2000..3000).contains(&d2), "attempt2 应 ~2000ms: {}", d2);
    }

    #[test]
    fn runtime_mode_requires_explicit() {
        std::env::remove_var("OUTBOUND_RUNTIME_MODE");
        std::env::remove_var("WZ_FSSC_RUNTIME_MODE");
        assert!(runtime_mode().is_err(), "未设置应 fail-closed");

        std::env::set_var("OUTBOUND_RUNTIME_MODE", "banana");
        assert!(runtime_mode().is_err(), "非法值应 fail-closed");

        std::env::set_var("OUTBOUND_RUNTIME_MODE", "production");
        assert_eq!(runtime_mode().unwrap(), OutboundRuntimeMode::Production);

        std::env::remove_var("OUTBOUND_RUNTIME_MODE");
        std::env::set_var("WZ_FSSC_RUNTIME_MODE", "mock");
        assert_eq!(
            runtime_mode().unwrap(),
            OutboundRuntimeMode::Mock,
            "兼容旧名"
        );

        std::env::remove_var("WZ_FSSC_RUNTIME_MODE");
    }

    #[test]
    fn with_pool_fail_closed_real_mode() {
        let err = OutboundClient::with_pool(
            OutboundClientConfig {
                mock: false,
                ..Default::default()
            },
            None,
        );
        assert!(err.is_err(), "真实模式无 pool 应 Err");

        let ok = OutboundClient::with_pool(
            OutboundClientConfig {
                mock: true,
                ..Default::default()
            },
            None,
        );
        assert!(ok.is_ok(), "mock 模式可无 pool");
    }
}
