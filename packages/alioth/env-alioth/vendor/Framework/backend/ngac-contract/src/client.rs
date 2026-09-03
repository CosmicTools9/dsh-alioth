use crate::types::{
    NgacError, PdpCheckRequest, PdpCheckResponse, PdpColumnsRequest, PdpColumnsResponse,
    PdpListRequest, PdpListResponse, PolicyVersionResponse,
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
                .build()
                .expect("Failed to build NGAC HTTP client"),
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
        let url = format!("{}/api/ngac/decide", self.base_url);

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", auth_token))
            .json(request)
            .send()
            .await
            .map_err(|e| NgacError::HttpError(e.to_string()))?;

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

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", auth_token))
            .json(request)
            .send()
            .await
            .map_err(|e| NgacError::HttpError(e.to_string()))?;

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

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", auth_token))
            .json(request)
            .send()
            .await
            .map_err(|e| NgacError::HttpError(e.to_string()))?;

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

    /// 查询 SSO NGAC 策略版本（`GET /api/ngac/policy-version`）。
    ///
    /// 供 PEP 版本探针使用（fix-ngac-decision-consistency D4）：版本变化即失效
    /// per-worker 决策/列缓存。无需用户 token（版本号无敏感面），与 decide 同信任面。
    pub async fn policy_version(&self) -> Result<PolicyVersionResponse, NgacError> {
        let url = format!("{}/api/ngac/policy-version", self.base_url);

        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| NgacError::HttpError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(NgacError::ServiceUnavailable(format!(
                "SSO returned {}: {}",
                status, body
            )));
        }

        response
            .json::<PolicyVersionResponse>()
            .await
            .map_err(|e| NgacError::InvalidResponse(e.to_string()))
    }
}
