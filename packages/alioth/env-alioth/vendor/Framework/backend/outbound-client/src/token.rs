//! iERP 共享平台两步 token 握手（2.1 获取 app_token + 2.2 获取通行证 token）+ 进程内缓存。
//!
//! 接口契约（物产中大财务共享三期外围系统单据对接接口 v1.3）：
//! - 2.1 `POST {base}/ierp/api/getAppToken.do`
//!   body `{appId, appSecuret, tenantid, accountId, language}` → `data.app_token`
//! - 2.2 `POST {base}/ierp/api/login.do`
//!   body `{user, apptoken, tenantid, accountId, usertype}` → `data.access_token`
//! - 响应统一 `{state: "success"|"error", errorCode, errorMsg, data: {...}}`
//! - 通行证 access_token 官方有效期 2 小时；缓存 TTL 取 7000s 并提前 60s 判定过期。
//!
//! 业务接口调用时将 access_token 放入 `access_token` 请求头（见 2.3 等接口）。
//! mock 配置下 MUST NOT 发起真实 token 请求（fail-closed）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::AliothError as ApiError;
use serde_json::{json, Value};

use crate::OutboundClientConfig;

/// 2.1 获取 app_token 接口路径
pub const APP_TOKEN_PATH: &str = "/ierp/api/getAppToken.do";
/// 2.2 获取通行证 token 接口路径
pub const LOGIN_TOKEN_PATH: &str = "/ierp/api/login.do";

/// 通行证缓存 TTL（官方 2h 有效，取 7000s 留余量）
const TOKEN_TTL: Duration = Duration::from_secs(7000);
/// 提前刷新余量：剩余有效期不足 60s 视为过期
const REFRESH_MARGIN: Duration = Duration::from_secs(60);

/// iERP 通行证 token 进程内缓存（Arc 共享，可放入 `app_data` 跨请求复用）。
///
/// 单飞（single-flight）：缓存过期瞬间的并发请求共享同一次握手——`get` 全程持
/// `tokio::sync::Mutex`（async 锁），第一个请求刷新并写缓存，其余等待者拿到锁后
/// 读到新缓存直接返回，不重复调远端。std Mutex 仅用于快速路径读，无 async 持有。
#[derive(Clone, Default)]
pub struct IerpTokenCache {
    token: Arc<tokio::sync::Mutex<Option<CachedToken>>>,
}

struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

impl IerpTokenCache {
    /// 获取通行证 access_token：缓存命中（剩余有效期 > 60s）直接返回，否则走两步握手。
    pub async fn get(&self, config: &OutboundClientConfig) -> Result<String, ApiError> {
        // 快速路径：命中直接返回（try_lock 失败=慢路径在跑，等慢路径刷新）
        if let Ok(guard) = self.token.try_lock() {
            if let Some(t) = guard.as_ref() {
                if t.expires_at > Instant::now() + REFRESH_MARGIN {
                    return Ok(t.access_token.clone());
                }
            }
        }

        // 慢路径：持 async 锁刷新（单飞——并发请求串行化，后到者复用新缓存）
        let mut guard = self.token.lock().await;
        if let Some(t) = guard.as_ref() {
            if t.expires_at > Instant::now() + REFRESH_MARGIN {
                // 其它请求已刷新
                return Ok(t.access_token.clone());
            }
        }

        let access_token = fetch_login_token(config).await?;
        *guard = Some(CachedToken {
            access_token: access_token.clone(),
            expires_at: Instant::now() + TOKEN_TTL,
        });
        Ok(access_token)
    }
}

/// 两步握手便捷入口（无缓存场景）；有缓存需求请用 `IerpTokenCache::get`。
pub async fn ierp_access_token(config: &OutboundClientConfig) -> Result<String, ApiError> {
    fetch_login_token(config).await
}

/// 2.1 获取 app_token。
pub async fn fetch_app_token(config: &OutboundClientConfig) -> Result<String, ApiError> {
    ensure_not_mock(config)?;
    let url = format!("{}{}", config.base_url, APP_TOKEN_PATH);
    let body = json!({
        "appId": config.app_id,
        "appSecuret": config.app_secret,
        "tenantid": config.tenant_id,
        "accountId": config.account_id,
        "language": config.language,
    });
    let res = token_http_client()?
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| ApiError::ServiceUnavailable(format!("iERP getAppToken request failed: {e}")))?
        .json::<Value>()
        .await
        .map_err(|e| {
            ApiError::ServiceUnavailable(format!("iERP getAppToken decode failed: {e}"))
        })?;
    check_state(&res)?;
    res["data"]["app_token"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| ApiError::Internal("iERP app_token missing".into()))
}

/// 2.2 获取通行证 token（内部先调 2.1 取 app_token）。
pub async fn fetch_login_token(config: &OutboundClientConfig) -> Result<String, ApiError> {
    ensure_not_mock(config)?;
    let app_token = fetch_app_token(config).await?;
    let url = format!("{}{}", config.base_url, LOGIN_TOKEN_PATH);
    let body = json!({
        "user": config.user,
        "apptoken": app_token,
        "tenantid": config.tenant_id,
        "accountId": config.account_id,
        "usertype": config.usertype,
    });
    let res = token_http_client()?
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| ApiError::ServiceUnavailable(format!("iERP login request failed: {e}")))?
        .json::<Value>()
        .await
        .map_err(|e| ApiError::ServiceUnavailable(format!("iERP login decode failed: {e}")))?;
    check_state(&res)?;
    res["data"]["access_token"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| ApiError::Internal("iERP access_token missing".into()))
}

/// mock 配置禁止真实 token 请求（fail-closed）。
fn ensure_not_mock(config: &OutboundClientConfig) -> Result<(), ApiError> {
    if config.mock {
        return Err(ApiError::Internal(
            "iERP token 请求禁止在 mock 模式发起（须注册表真实配置）".into(),
        ));
    }
    Ok(())
}

/// token 握手专用 HTTP 客户端（与治理框架同口径：connect 5s / request 30s）。
fn token_http_client() -> Result<reqwest::Client, ApiError> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        // 同 lib.rs：内网出向禁用系统代理（macOS 系统代理会吞内网流量）。
        .no_proxy()
        .build()
        .map_err(|e| ApiError::Internal(format!("reqwest client build failed: {e}")))
}

/// iERP 统一响应状态校验：`state=success` 放行，`error` 上抛 errorCode/errorMsg。
fn check_state(res: &Value) -> Result<(), ApiError> {
    let state = res["state"].as_str().unwrap_or("error");
    if state == "success" {
        return Ok(());
    }
    let code = res["errorCode"].as_str().unwrap_or("");
    let msg = res["errorMsg"].as_str().unwrap_or("unknown error");
    Err(ApiError::ServiceUnavailable(format!(
        "iERP token state=error errorCode={code} errorMsg={msg}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_state_success() {
        let res = json!({"state": "success", "data": {"app_token": "t"}});
        assert!(check_state(&res).is_ok());
    }

    #[test]
    fn check_state_error_includes_code_and_msg() {
        let res = json!({
            "state": "error",
            "errorCode": "login.loginBizException",
            "errorMsg": "不正确的第三方appId或appSecuret!"
        });
        let err = check_state(&res).unwrap_err().to_string();
        assert!(err.contains("login.loginBizException"), "err: {err}");
        assert!(
            err.contains("不正确的第三方appId或appSecuret"),
            "err: {err}"
        );
    }

    #[test]
    fn check_state_missing_state_treated_as_error() {
        let res = json!({"data": {}});
        assert!(check_state(&res).is_err());
    }

    #[test]
    fn ensure_not_mock_fail_closed() {
        let mock_cfg = OutboundClientConfig {
            mock: true,
            ..Default::default()
        };
        assert!(ensure_not_mock(&mock_cfg).is_err());
        let real_cfg = OutboundClientConfig {
            mock: false,
            ..Default::default()
        };
        assert!(ensure_not_mock(&real_cfg).is_ok());
    }

    #[tokio::test]
    async fn cache_hit_returns_without_network() {
        let cache = IerpTokenCache::default();
        {
            let mut guard = cache.token.lock().await;
            *guard = Some(CachedToken {
                access_token: "cached-token".to_string(),
                expires_at: Instant::now() + TOKEN_TTL,
            });
        }
        // base_url 故意为空：若未命中缓存发起网络请求必失败
        let cfg = OutboundClientConfig {
            mock: false,
            ..Default::default()
        };
        let token = cache.get(&cfg).await.unwrap();
        assert_eq!(token, "cached-token");
    }

    /// 单飞验证：缓存过期瞬间 10 个并发 get，只触发 1 次两步握手。
    #[tokio::test]
    async fn concurrent_get_single_flight() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let hit_count = Arc::new(AtomicUsize::new(0));
        let hits = hit_count.clone();

        // 本地 stub：任意请求都返回两步握手成功的 JSON（两步握手 = 2 次 HTTP）
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = br#"{"state":"success","data":{"app_token":"at","access_token":"access-1"}}"#;
        let resp_head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    continue;
                };
                hits.fetch_add(1, Ordering::SeqCst);
                let head = resp_head.clone();
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(body).await;
                });
            }
        });

        let cfg = OutboundClientConfig {
            mock: false,
            base_url: format!("http://{addr}"),
            app_id: "app".into(),
            app_secret: "secret".into(),
            tenant_id: "t".into(),
            account_id: "a".into(),
            ..Default::default()
        };

        let cache = IerpTokenCache::default();
        let mut handles = Vec::new();
        for _ in 0..10 {
            let cache = cache.clone();
            let cfg = cfg.clone();
            handles.push(tokio::spawn(async move { cache.get(&cfg).await.unwrap() }));
        }
        for h in handles {
            let t = h.await.unwrap();
            assert_eq!(t, "access-1");
        }

        server.abort();
        // 10 并发单飞 → 只 1 次两步握手 = 2 次 HTTP（app_token + login）
        let total = hit_count.load(Ordering::SeqCst);
        assert!(
            total <= 3,
            "单飞失败：10 并发触发了 {total} 次 HTTP 请求（应 = 2 次两步握手，容忍 1 余量）"
        );
    }

    #[tokio::test]
    async fn mock_config_rejected_before_network() {
        let cache = IerpTokenCache::default();
        let cfg = OutboundClientConfig {
            mock: true,
            ..Default::default()
        };
        let err = cache.get(&cfg).await.unwrap_err().to_string();
        assert!(err.contains("mock"), "err: {err}");
    }
}
