//! 全局共享的 HTTP Client
//!
//! OAuth/OIDC/SMS 等模块每次请求都新建 `reqwest::Client` 会反复创建连接池和
//! 后台线程，导致句柄/线程泄漏。这里提供一个进程级复用的 Client（内部是 Arc，
//! clone 成本低），供所有需要发 HTTP 请求的 SSO handler 使用。

use std::sync::OnceLock;

/// 获取全局共享的 reqwest Client。
pub fn get() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        log::info!("Initializing global reqwest HTTP client");
        reqwest::Client::new()
    })
}
