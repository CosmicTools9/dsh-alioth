//! 全局共享的 HTTP Client
//!
//! OAuth/OIDC/SMS 等模块每次请求都新建 `reqwest::Client` 会反复创建连接池和
//! 后台线程，导致句柄/线程泄漏。这里提供一个进程级复用的 Client（内部是 Arc，
//! clone 成本低），供所有需要发 HTTP 请求的 SSO handler 使用。
//!
//! 超时（fix-sso-residual-gaps G2）：connect 10s + 总 15s——外部 IdP/SMS 故障时
//! handler 最多悬挂 15s 后报错，不再无限期等待。SLO 自建 client（10s）不并入，
//! 语义更严且独立。

use std::sync::OnceLock;
use std::time::Duration;

/// 连接建立超时
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 单请求总超时（连接 + 请求 + 响应体）
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// 获取全局共享的 reqwest Client。
pub fn get() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        log::info!("Initializing global reqwest HTTP client");
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("build global reqwest client")
    })
}
