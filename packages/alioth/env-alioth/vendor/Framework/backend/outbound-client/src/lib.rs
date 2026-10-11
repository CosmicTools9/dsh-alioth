//! 出向调用治理框架（provider 无关）+ provider 抽象装载。
//!
//! 分层职责（`docs/specs/EXTERNAL_INTEGRATION_SPEC.md` §8.2）：
//!
//! | 面 | 落点 | 内容 |
//! |---|---|---|
//! | provider 无关治理（本 crate） | `Framework/backend/outbound-client` | 注册表加载（fail-closed 解密）、HTTP 超时、幂等读重试、写不重试、非 2xx 转错、出向计量、provider 抽象与派发 |
//! | provider 协议实现（**不在本 crate**） | L-ADP 适配器面（`Pre-Proc/{ns}/Ext-adapter/<ext>/backend` 或平台共享 `Ext-adapter/<ext>/backend`） | 握手/签名/专有字段与配置 schema |
//! | 字段语义映射（**不在本 crate**） | 消费 service 的 `.../integration/<ext>/` | `<ext>_to_isahl` / `isahl_to_<ext>` |
//!
//! 本 crate MUST NOT 含任何 provider 的协议实现，MUST NOT 读取 provider 专属环境变量
//! （凭据唯一来源 = `isahl_auth.outbound_client`）。

pub mod crypto;
pub mod provider;
pub mod repository;

pub use provider::{
    install, registry, resolve, OutboundProvider, ProviderContext, ProviderHttp, ProviderRegistry,
    RequestInfo, AUTH_TYPE_NONE,
};

use common::AliothError as ApiError;
use serde_json::Value;

/// 出向运行模式：显式声明，禁止隐式默认。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundRuntimeMode {
    Production,
    Mock,
}

/// 解析 `OUTBOUND_RUNTIME_MODE`（production|mock）。未设置或非法值 → Err
/// （安全默认 fail-closed，不隐式降级 mock）。
pub fn runtime_mode() -> Result<OutboundRuntimeMode, ApiError> {
    match std::env::var("OUTBOUND_RUNTIME_MODE") {
        Ok(v) if v.trim().eq_ignore_ascii_case("production") => Ok(OutboundRuntimeMode::Production),
        Ok(v) if v.trim().eq_ignore_ascii_case("mock") => Ok(OutboundRuntimeMode::Mock),
        Ok(v) => Err(ApiError::Internal(format!(
            "OUTBOUND_RUNTIME_MODE 非法（实得 '{v}'；取值 production|mock）"
        ))),
        Err(_) => Err(ApiError::Internal(
            "OUTBOUND_RUNTIME_MODE 未设置（取值 production|mock；出向构造 fail-closed）"
                .to_string(),
        )),
    }
}

/// 出向调用方配置（provider 中立；`config` 的 schema 归 provider）。
#[derive(Debug, Clone)]
pub struct OutboundClientConfig {
    /// 注册表 `provider` 列 = provider 派发键（`auth_type='none'` 时可为空）。
    pub provider_id: String,
    /// 注册表 `auth_type` 列（`none` = 无认证；其余值由 provider 定义）。
    pub auth_type: String,
    /// 注册表 `base_url` 列（空 = 未配置端点，调用方按自身契约降级）。
    pub base_url: String,
    /// 注册表 `config` 列（jsonb；provider 私有配置）。
    pub config: Value,
    /// 解密后的明文凭据（仅内存，MUST NOT 落日志）。
    pub secret: Option<String>,
    /// mock 模式（`OUTBOUND_RUNTIME_MODE=mock` 且注册表无有效行时的显式降级态）。
    pub mock: bool,
}

impl Default for OutboundClientConfig {
    fn default() -> Self {
        Self {
            provider_id: String::new(),
            auth_type: provider::AUTH_TYPE_NONE.to_string(),
            base_url: String::new(),
            config: Value::Object(Default::default()),
            secret: None,
            mock: true,
        }
    }
}

impl OutboundClientConfig {
    /// 从 `isahl_auth.outbound_client` 注册表加载（`code` 参数化，如 `fssc-wzgroup`）。
    ///
    /// fail-closed 分层：
    /// - 有 enabled 行 → 用该行；`app_secret_enc` 解密失败即报错（错误密钥必须暴露）；
    /// - 无有效行 → **production 报错**；**mock 模式**返回 mock 配置（`mock=true`、空
    ///   `base_url`），由调用方按自身契约降级（如日志 + 跳过推送）——MUST NOT 读取环境
    ///   变量凭据。
    pub async fn load_from_registry(pool: &sqlx::PgPool, code: &str) -> Result<Self, ApiError> {
        let mode = runtime_mode()?;
        let row = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<String>,
                Option<Value>,
                Option<String>,
            ),
        >(
            "SELECT provider, auth_type, base_url, config, app_secret_enc \
             FROM isahl_auth.outbound_client \
             WHERE code = $1 AND enabled = TRUE AND deleted_at IS NULL \
             ORDER BY version DESC LIMIT 1",
        )
        .bind(code)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::from_sqlx)?;

        let Some((provider_id, auth_type, base_url, config, secret_enc)) = row else {
            return match mode {
                OutboundRuntimeMode::Mock => Ok(Self {
                    mock: true,
                    ..Default::default()
                }),
                OutboundRuntimeMode::Production => Err(ApiError::Internal(format!(
                    "出向调用方注册表无有效配置（outbound_client code='{code}' 空或全 disabled）：\
                     生产模式禁止回退（凭据唯一来源 = 注册表行；请经管理面或种子配置该行）"
                ))),
            };
        };

        if provider_id.trim().is_empty() {
            return Err(ApiError::Internal(format!(
                "注册行 '{code}' 的 provider 为空——派发键必填"
            )));
        }
        if auth_type.trim().is_empty() {
            return Err(ApiError::Internal(format!(
                "注册行 '{code}' 的 auth_type 为空——认证方案必填（无认证填 'none'）"
            )));
        }

        let secret = match secret_enc.filter(|s| !s.is_empty()) {
            Some(enc) => Some(crypto::decrypt(&enc)?),
            None if auth_type == provider::AUTH_TYPE_NONE => None,
            None => {
                return Err(ApiError::Internal(format!(
                    "注册行 '{code}'（auth_type='{auth_type}'）缺 app_secret_enc（请配置密文）"
                )))
            }
        };

        Ok(Self {
            provider_id,
            auth_type,
            base_url: base_url.unwrap_or_default(),
            config: config.unwrap_or_else(|| Value::Object(Default::default())),
            secret,
            mock: false,
        })
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

/// 出向原始应答（provider 需要原文面：应答验签依赖**原始字节**、`204 No Content`、
/// 或非 JSON 载荷）。`body` 为未解析文本——签名验证 MUST 以此字节为准（`Value` 路径
/// 重序列化后的字节与上游原文不一致，无法验签）。
#[derive(Debug, Clone)]
pub struct RawResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// 出向 HTTP 客户端（provider 无关治理 + provider 认证注入）。
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
        Self {
            config,
            client: build_http_client(),
            pool: None,
        }
    }

    /// 带出向计量池构造。真实模式（mock=false）MUST 提供 pool（fail-closed——
    /// 真实调用必须有计量落库）。
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
        Ok(Self {
            config,
            client: build_http_client(),
            pool,
        })
    }

    pub fn is_mock(&self) -> bool {
        self.config.mock
    }

    pub fn config(&self) -> &OutboundClientConfig {
        &self.config
    }

    fn http(&self) -> ProviderHttp {
        ProviderHttp::new(
            self.client.clone(),
            self.pool.clone(),
            self.config.provider_id.clone(),
        )
    }

    /// 构造 provider 上下文（按本 client 的注册行）。
    fn provider_context<'a>(&'a self, http: &'a ProviderHttp) -> ProviderContext<'a> {
        ProviderContext::new(
            &self.config.provider_id,
            &self.config.auth_type,
            &self.config.base_url,
            &self.config.config,
            self.config.secret.as_deref(),
            http,
        )
    }

    /// 请求体级认证注入（provider 契约 `prepare_body`）——**唯一变换点**。
    ///
    /// MUST 在序列化前调用（使 `RequestInfo.body_text` 恒为实际发送字节）；
    /// `auth_type='none'` 跳过（与 [`Self::auth_headers`] 同分支语义）。
    async fn prepare_governed_body(
        &self,
        method: &str,
        url: &str,
        body: Option<Value>,
    ) -> Result<Option<Value>, ApiError> {
        match body {
            Some(b) if self.config.auth_type != provider::AUTH_TYPE_NONE => {
                let provider = provider::resolve(&self.config.provider_id)?;
                let http = self.http();
                let ctx = self.provider_context(&http);
                Ok(Some(provider.prepare_body(&ctx, method, url, b).await?))
            }
            other => Ok(other),
        }
    }

    /// 解析认证头（`auth_type='none'` ⇒ 空；provider 未注册 ⇒ fail-closed）。
    async fn auth_headers(
        &self,
        method: &str,
        url: &str,
        body_text: Option<&str>,
    ) -> Result<Vec<(String, String)>, ApiError> {
        if self.config.auth_type == provider::AUTH_TYPE_NONE {
            return Ok(Vec::new());
        }
        let provider = resolve(&self.config.provider_id)?;
        let http = self.http();
        let ctx = self.provider_context(&http);
        let req = RequestInfo {
            method,
            url,
            body_text,
        };
        provider.authorize(ctx, &req).await
    }

    /// 通知 provider 认证态失效（best-effort：provider 缺失时静默——认证失败本身已上抛）。
    async fn invalidate_auth(&self) {
        if self.config.auth_type == provider::AUTH_TYPE_NONE {
            return;
        }
        if let Ok(provider) = resolve(&self.config.provider_id) {
            let http = self.http();
            let ctx = self.provider_context(&http);
            provider.invalidate(ctx).await;
        }
    }

    /// 通用出向 POST（认证头由 provider 注入；调用方 MUST NOT 自行拼装认证头与内容类型）。
    ///
    /// - `retryable=true`（幂等读）：网络错误/5xx 按指数退避重试至多 3 次；
    /// - `retryable=false`（写操作）：不重试（防重复提交）；
    /// - 认证 401/403：provider `invalidate()` 后重试一次（401 保证无业务副作用）。
    pub async fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: Value,
        retryable: bool,
        interface: &str,
    ) -> Result<Value, ApiError> {
        self.send_governed("POST", url, headers, Some(body), retryable, interface)
            .await
    }

    /// 治理化 GET（只读；语义同 [`OutboundClient::post_json`]）。
    pub async fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        retryable: bool,
        interface: &str,
    ) -> Result<Value, ApiError> {
        self.send_governed("GET", url, headers, None, retryable, interface)
            .await
    }

    /// 治理化**原始**出向：超时、重试（网络错误与 5xx，`retryable=true` 时）、计量
    /// 同 [`OutboundClient::post_json`]；差异有二（刻意）：
    /// 1. **不做 JSON 解析**——返回原文文本（应答验签、`204` 无内容、非 JSON 载荷）；
    /// 2. **非 2xx 不转 Err**——由调用方按 provider 契约解析错误体（HTTP 层已落计量）。
    #[allow(unused_assignments)] // last_err 初值 = 重试循环异常退出的兜底（非死代码）
    #[allow(clippy::too_many_arguments)] // 出向请求参数（通道/重试/计量归因）
    pub async fn send_raw(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<Value>,
        retryable: bool,
        interface: &str,
    ) -> Result<RawResponse, ApiError> {
        if self.config.mock {
            return Err(ApiError::Internal(
                "OutboundClient 处于 mock 模式，不能发起真实调用".into(),
            ));
        }
        let body = self.prepare_governed_body(method, url, body).await?;
        let body_text = serialize_body(body.as_ref())?;
        let max_attempts = if retryable { 3 } else { 1 };
        let mut attempt = 0u32;
        let mut last_err: Option<ApiError> = None;
        loop {
            let mut all_headers = match self.auth_headers(method, url, body_text.as_deref()).await {
                Ok(h) => h,
                Err(e) => return Err(e),
            };
            all_headers.extend(headers.iter().cloned());
            match Self::raw_once(
                &self.client,
                method,
                url,
                &all_headers,
                body_text.as_deref(),
            )
            .await
            {
                Ok(raw) => {
                    let status = raw.status;
                    if matches!(status, 401 | 403) {
                        self.invalidate_auth().await;
                        if attempt == 0 {
                            attempt += 1;
                            continue;
                        }
                    }
                    let ok = (200..300).contains(&status);
                    Self::log_call(
                        self.pool.as_ref(),
                        &self.config.provider_id,
                        interface,
                        method,
                        if ok { "ok" } else { "error" },
                        std::time::Instant::now(),
                    )
                    .await;
                    return Ok(raw);
                }
                Err(e) => {
                    if retryable && attempt + 1 < max_attempts {
                        let delay = backoff_delay_ms(attempt);
                        attempt += 1;
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }
                    last_err = Some(e);
                    break;
                }
            }
        }
        Self::log_call(
            self.pool.as_ref(),
            &self.config.provider_id,
            interface,
            method,
            "error",
            std::time::Instant::now(),
        )
        .await;
        Err(last_err.unwrap_or_else(|| ApiError::ServiceUnavailable(format!("{interface} failed"))))
    }

    #[allow(unused_assignments)] // last_err 初值 = 重试循环异常退出的兜底（非死代码）
    #[allow(clippy::too_many_arguments)] // 出向请求参数（通道/重试/计量归因）
    async fn send_governed(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<Value>,
        retryable: bool,
        interface: &str,
    ) -> Result<Value, ApiError> {
        if self.config.mock {
            return Err(ApiError::Internal(
                "OutboundClient 处于 mock 模式，不能发起真实调用".into(),
            ));
        }
        let body = self.prepare_governed_body(method, url, body).await?;
        let body_text = serialize_body(body.as_ref())?;
        let max_attempts = if retryable { 3 } else { 1 };
        let mut attempt = 0u32;
        let mut last_err: Option<ApiError> = None;
        loop {
            let mut all_headers = match self.auth_headers(method, url, body_text.as_deref()).await {
                Ok(h) => h,
                Err(e) => return Err(e),
            };
            all_headers.extend(headers.iter().cloned());
            match Self::send_once_inner(
                method,
                url,
                &all_headers,
                body_text.as_deref(),
                &self.client,
            )
            .await
            {
                Ok((status, _raw, json)) => {
                    if matches!(status, 401 | 403) {
                        self.invalidate_auth().await;
                        if attempt == 0 {
                            attempt += 1;
                            continue;
                        }
                    }
                    if (200..300).contains(&status) {
                        Self::log_call(
                            self.pool.as_ref(),
                            &self.config.provider_id,
                            interface,
                            method,
                            "ok",
                            std::time::Instant::now(),
                        )
                        .await;
                        return Ok(json);
                    }
                    let err = ApiError::ServiceUnavailable(format!(
                        "{interface} HTTP {status}: {}",
                        json.to_string().chars().take(200).collect::<String>()
                    ));
                    if status >= 500 && retryable && attempt + 1 < max_attempts {
                        let delay = backoff_delay_ms(attempt);
                        attempt += 1;
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }
                    last_err = Some(err);
                    break;
                }
                Err(e) => {
                    if retryable && attempt + 1 < max_attempts {
                        let delay = backoff_delay_ms(attempt);
                        attempt += 1;
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }
                    last_err = Some(e);
                    break;
                }
            }
        }
        Self::log_call(
            self.pool.as_ref(),
            &self.config.provider_id,
            interface,
            method,
            "error",
            std::time::Instant::now(),
        )
        .await;
        Err(last_err.unwrap_or_else(|| ApiError::ServiceUnavailable(format!("{interface} failed"))))
    }

    /// provider 面治理化发送（`ProviderHttp` 唯一实现；计量 + 超时 + 重试）。
    ///
    /// 注意：provider 内层请求**不注入认证头**（避免自引用），认证失败亦不触发 invalidate
    /// （由外层业务调用的 401/403 统一处理）。
    #[allow(unused_assignments)] // last_err 初值 = 重试循环异常退出的兜底（非死代码）
    #[allow(clippy::too_many_arguments)] // provider 通道参数
    pub(crate) async fn send_once(
        client: &reqwest::Client,
        pool: Option<&sqlx::PgPool>,
        provider_id: &str,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<Value>,
        retryable: bool,
        interface: &str,
    ) -> Result<Value, ApiError> {
        let body_text = serialize_body(body.as_ref())?;
        let max_attempts = if retryable { 3 } else { 1 };
        let mut attempt = 0u32;
        let mut last_err: Option<ApiError> = None;
        loop {
            match Self::send_once_inner(method, url, headers, body_text.as_deref(), client).await {
                Ok((status, _raw, json)) => {
                    if (200..300).contains(&status) {
                        Self::log_call(
                            pool,
                            provider_id,
                            interface,
                            method,
                            "ok",
                            std::time::Instant::now(),
                        )
                        .await;
                        return Ok(json);
                    }
                    let err = ApiError::ServiceUnavailable(format!(
                        "{interface} HTTP {status}: {}",
                        json.to_string().chars().take(200).collect::<String>()
                    ));
                    if status >= 500 && retryable && attempt + 1 < max_attempts {
                        let delay = backoff_delay_ms(attempt);
                        attempt += 1;
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }
                    last_err = Some(err);
                    break;
                }
                Err(e) => {
                    if retryable && attempt + 1 < max_attempts {
                        let delay = backoff_delay_ms(attempt);
                        attempt += 1;
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }
                    last_err = Some(e);
                    break;
                }
            }
        }
        Self::log_call(
            pool,
            provider_id,
            interface,
            method,
            "error",
            std::time::Instant::now(),
        )
        .await;
        Err(last_err.unwrap_or_else(|| ApiError::ServiceUnavailable(format!("{interface} failed"))))
    }

    /// 单次发送（无重试/无计量）：返回（状态码, 原文, JSON 值或 `null`）。
    async fn send_once_inner(
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body_text: Option<&str>,
        client: &reqwest::Client,
    ) -> Result<(u16, RawResponse, Value), ApiError> {
        let raw = Self::raw_once(client, method, url, headers, body_text).await?;
        let json = serde_json::from_str::<Value>(&raw.body).unwrap_or(Value::Null);
        Ok((raw.status, raw, json))
    }

    /// 单次 HTTP 发送（body 以**已序列化文本**发送：签名与发送字节 MUST 同源）。
    async fn raw_once(
        client: &reqwest::Client,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body_text: Option<&str>,
    ) -> Result<RawResponse, ApiError> {
        let mut req = match method {
            "GET" => client.get(url),
            _ => client.post(url),
        };
        for (k, v) in headers {
            req = req.header(k, v);
        }
        if let Some(text) = body_text {
            req = req
                .header("Content-Type", "application/json; charset=utf-8")
                .body(text.to_string());
        }
        let resp = req.send().await.map_err(|e| {
            ApiError::ServiceUnavailable(format!("outbound request failed ({method} {url}): {e}"))
        })?;
        let status = resp.status().as_u16();
        let resp_headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| {
                v.to_str()
                    .ok()
                    .map(|s| (k.as_str().to_string(), s.to_string()))
            })
            .collect();
        let body = resp.text().await.unwrap_or_default();
        Ok(RawResponse {
            status,
            headers: resp_headers,
            body,
        })
    }

    /// 出向计量（best-effort，失败仅 warn 不阻塞）。
    async fn log_call(
        pool: Option<&sqlx::PgPool>,
        provider: &str,
        interface: &str,
        method: &str,
        status: &str,
        started: std::time::Instant,
    ) {
        let Some(pool) = pool else { return };
        let request_id = format!(
            "outbound-{}-{}",
            interface.replace('/', "_"),
            chrono::Utc::now().timestamp_millis()
        );
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

/// body 序列化（唯一实现：签名与发送共用同一文本）。
fn serialize_body(body: Option<&Value>) -> Result<Option<String>, ApiError> {
    match body {
        None => Ok(None),
        Some(v) => serde_json::to_string(v)
            .map(Some)
            .map_err(|e| ApiError::Internal(format!("出向 body 序列化失败: {e}"))),
    }
}

/// 内网出向 HTTP 客户端（与治理口径一致：connect 5s / request 30s / 禁系统代理）。
fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(30))
        // 出向目标多为企业内网：dev 机 macOS 系统代理（sing-box 等）会把内网流量
        // 送进代理导致超时，reqwest 默认读系统代理，必须显式禁用。
        .no_proxy()
        .build()
        .expect("reqwest client build")
}

/// 便捷构造：注册表加载 + 运行模式判定 + with_pool。
pub async fn client_from_registry(
    pool: &sqlx::PgPool,
    code: &str,
) -> Result<OutboundClient, ApiError> {
    let config = OutboundClientConfig::load_from_registry(pool, code).await?;
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
        assert!(runtime_mode().is_err(), "未设置应 fail-closed");

        std::env::set_var("OUTBOUND_RUNTIME_MODE", "banana");
        assert!(runtime_mode().is_err(), "非法值应 fail-closed");

        std::env::set_var("OUTBOUND_RUNTIME_MODE", "production");
        assert_eq!(runtime_mode().unwrap(), OutboundRuntimeMode::Production);

        std::env::set_var("OUTBOUND_RUNTIME_MODE", "mock");
        assert_eq!(runtime_mode().unwrap(), OutboundRuntimeMode::Mock);

        // 旧名 MUST NOT 生效（迁移期兼容已终止）
        std::env::remove_var("OUTBOUND_RUNTIME_MODE");
        std::env::set_var("WZ_FSSC_RUNTIME_MODE", "mock");
        assert!(
            runtime_mode().is_err(),
            "旧名 WZ_FSSC_RUNTIME_MODE 不得生效"
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

    #[tokio::test]
    async fn unregistered_provider_fails_closed_before_any_request() {
        let client = OutboundClient::with_pool(
            OutboundClientConfig {
                provider_id: "not-installed-anywhere".into(),
                auth_type: "custom".into(),
                base_url: "http://127.0.0.1:1/never".into(),
                mock: false,
                ..Default::default()
            },
            Some(dead_pool()),
        )
        .expect("client");
        let err = client
            .post_json(
                "http://127.0.0.1:1/never",
                &[],
                serde_json::json!({}),
                false,
                "probe",
            )
            .await
            .expect_err("未注册 provider 必须 fail-closed");
        let msg = format!("{err}");
        assert!(
            msg.contains("未注册") || msg.contains("未安装"),
            "文案应指向 provider 装载面: {msg}"
        );
    }

    #[tokio::test]
    async fn mock_mode_blocks_real_calls() {
        let client = OutboundClient::new(OutboundClientConfig {
            mock: true,
            ..Default::default()
        });
        let err = client
            .get_json("http://127.0.0.1:1/x", &[], true, "probe")
            .await
            .expect_err("mock 模式不得发起真实调用");
        assert!(format!("{err}").contains("mock"), "{err}");
    }

    /// 惰性连接池（测试不触库：仅构造，不做查询）。
    fn dead_pool() -> sqlx::PgPool {
        sqlx::PgPool::connect_lazy("postgres://localhost:5432/aliothstudio_dev").expect("lazy pool")
    }
}
