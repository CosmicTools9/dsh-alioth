use crate::types::{
    NgacError, PdpCheckRequest, PdpCheckResponse, PdpColumnsRequest, PdpColumnsResponse,
    PdpListRequest, PdpListResponse,
};

/// HTTP client for calling SSO NGAC decision endpoints.
#[derive(Debug, Clone)]
pub struct HttpNgacClient {
    base_url: String,
    http_client: reqwest::Client,
}

impl HttpNgacClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3))
                .pool_max_idle_per_host(0)
                // NGAC PDP 目标恒为本地 SSO 回环——禁用 env 代理劫持
                // （ALL_PROXY=socks5 等曾致本地 decide 连接失败，2026-09-08 实证）
                .no_proxy()
                .build()
                .expect("Failed to build NGAC HTTP client"),
        }
    }

    /// POST 一次发送；连接/传输层失败（reqwest send 错误——SSO 不可达/瞬时断连）时
    /// 退避 150ms 重试 1 次。PDP decide/list/columns 均为只读判定（幂等语义），重试
    /// 安全；HTTP 状态响应（4xx/5xx）不重试——调用方按既有语义处理（业务拒绝不变）。
    /// 2026-09-08 加固：3s 超时窗内单次连接瞬断曾致 PEP 误 deny（偶发 PDP 错误）。
    async fn post_with_retry(
        &self,
        url: &str,
        auth_token: &str,
        request: &impl serde::Serialize,
    ) -> Result<reqwest::Response, NgacError> {
        let mut attempted = false;
        loop {
            match self
                .http_client
                .post(url)
                .header("Authorization", format!("Bearer {}", auth_token))
                .json(request)
                .send()
                .await
            {
                Ok(response) => return Ok(response),
                Err(_e) if !attempted => {
                    attempted = true;
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                }
                Err(e) => return Err(NgacError::HttpError(e.to_string())),
            }
        }
    }

    /// Call the SSO NGAC decide endpoint.
    ///
    /// The `auth_token` should be the raw JWT token (without the "Bearer " prefix)
    /// so that SSO can validate the request.
    pub async fn decide(
        &self,
        request: &PdpCheckRequest,
        auth_token: &str,
    ) -> Result<PdpCheckResponse, NgacError> {
        let url = format!("{}/api/ngac/pdp/decide", self.base_url);

        let response = self.post_with_retry(&url, auth_token, request).await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(NgacError::ServiceUnavailable(format!(
                "SSO returned {}: {}",
                status, body
            )));
        }

        let decision = response
            .json::<PdpCheckResponse>()
            .await
            .map_err(|e| NgacError::InvalidResponse(e.to_string()))?;

        Ok(decision)
    }

    /// Call the SSO NGAC list endpoint to get visible resource IDs.
    pub async fn list(
        &self,
        request: &PdpListRequest,
        auth_token: &str,
    ) -> Result<PdpListResponse, NgacError> {
        let url = format!("{}/api/ngac/pdp/list", self.base_url);

        let response = self.post_with_retry(&url, auth_token, request).await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(NgacError::ServiceUnavailable(format!(
                "SSO returned {}: {}",
                status, body
            )));
        }

        let result = response
            .json::<PdpListResponse>()
            .await
            .map_err(|e| NgacError::InvalidResponse(e.to_string()))?;

        Ok(result)
    }

    /// Call the SSO NGAC columns endpoint to get user-authorized column set for a resource type.
    pub async fn columns(
        &self,
        request: &PdpColumnsRequest,
        auth_token: &str,
    ) -> Result<PdpColumnsResponse, NgacError> {
        let url = format!("{}/api/ngac/pdp/columns", self.base_url);

        let response = self.post_with_retry(&url, auth_token, request).await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(NgacError::ServiceUnavailable(format!(
                "SSO returned {}: {}",
                status, body
            )));
        }

        let result = response
            .json::<PdpColumnsResponse>()
            .await
            .map_err(|e| NgacError::InvalidResponse(e.to_string()))?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PdpCheckRequest;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// 微型 HTTP server：前 `drop_first` 个连接 accept 后立即关闭（模拟瞬时断连——
    /// reqwest send 侧即连接错误）；其后正常返回 200 JSON（Connection: close，
    /// 与 client pool_max_idle_per_host(0) 每请求新连接匹配）。
    async fn start_server(drop_first: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut conn = 0usize;
            loop {
                let accepted = listener.accept().await;
                let Ok((mut sock, _)) = accepted else {
                    break;
                };
                conn += 1;
                if conn <= drop_first {
                    drop(sock);
                    continue;
                }
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let body = r#"{"permitted":true,"reason":"retry-ok"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn decide_succeeds_without_retry() {
        let base = start_server(0).await;
        let client = HttpNgacClient::new(base);
        let req = PdpCheckRequest {
            user_id: 1,
            resource: "r:1".into(),
            action: "read".into(),
        };
        let out = client.decide(&req, "t").await.expect("无瞬断应直接成功");
        assert!(out.permitted);
    }

    #[tokio::test]
    async fn decide_retries_once_on_transient_conn_failure() {
        let base = start_server(1).await;
        let client = HttpNgacClient::new(base);
        let req = PdpCheckRequest {
            user_id: 1,
            resource: "r:1".into(),
            action: "read".into(),
        };
        let out = client
            .decide(&req, "t")
            .await
            .expect("首连瞬断后重试应成功");
        assert!(out.permitted);
        assert_eq!(out.reason, "retry-ok");
    }

    #[tokio::test]
    async fn decide_errors_after_single_retry_exhausted() {
        let base = start_server(3).await;
        let client = HttpNgacClient::new(base);
        let req = PdpCheckRequest {
            user_id: 1,
            resource: "r:1".into(),
            action: "read".into(),
        };
        let err = client.decide(&req, "t").await.expect_err("重试耗尽应报错");
        assert!(
            err.to_string().contains("HTTP request failed"),
            "应透传连接错误，实为 {err}"
        );
    }
}
