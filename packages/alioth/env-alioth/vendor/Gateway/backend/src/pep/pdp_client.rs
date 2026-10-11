//! PEP 内部 seam — PDP 远端客户端与 PDP 类型（评审候选 5 分缝之一：pdp_client）。
//!
//! 自 `pep/middleware.rs` 内部抽出；`PdpError` 与 `PdpCheck*` 经 `pep::middleware`
//! `pub use` 保持原公开路径。

/// PDP (Policy Decision Point) error types.
///
/// Errors that can occur during PDP communication, including token validation,
/// network failures, and invalid responses.
#[derive(Debug)]
pub enum PdpError {
    /// JWT token decoding or validation failed
    TokenDecodeError(String),
    /// Network error during PDP communication
    NetworkError(String),
    /// Invalid response format from PDP
    InvalidResponse(String),
    /// PDP service unavailable
    PdpUnavailable(String),
}

impl std::fmt::Display for PdpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PdpError::TokenDecodeError(reason) => write!(f, "Token decode failed: {}", reason),
            PdpError::NetworkError(reason) => write!(f, "Network error: {}", reason),
            PdpError::InvalidResponse(reason) => write!(f, "Invalid PDP response: {}", reason),
            PdpError::PdpUnavailable(reason) => write!(f, "PDP unavailable: {}", reason),
        }
    }
}

impl std::error::Error for PdpError {}

/// PDP 检查请求 / 响应类型已从 ngac-contract crate 导入。
/// 本地保留类型别名以便测试代码兼容。
pub type PdpCheckRequest = ngac_contract::PdpCheckRequest;
pub type PdpCheckResponse = ngac_contract::PdpCheckResponse;

/// 远程调用 SSO NGAC PDP 决策端点
/// 通过 HTTP 调用 SSO 的 `/api/ngac/decide` 接口，消除编译时耦合
pub(super) async fn call_pdp_remote(
    ngac_client: &ngac_contract::HttpNgacClient,
    auth_token: &str,
    user_id: i64,
    resource: &str,
    action: &str,
) -> Result<PdpCheckResponse, PdpError> {
    let request = ngac_contract::PdpCheckRequest {
        user_id,
        resource: resource.to_string(),
        action: action.to_string(),
    };

    let response = ngac_client
        .decide(&request, auth_token)
        .await
        .map_err(|e| {
            common::telemetry::error!("NGAC remote decision failed: {}", e);
            PdpError::NetworkError(e.to_string())
        })?;

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pdp_check_request_serialize() {
        let request = PdpCheckRequest {
            user_id: 42,
            resource: "document:123".to_string(),
            action: "read".to_string(),
        };

        let json = serde_json::to_string(&request).unwrap();
        // serde_zuid 迁移：user_id 序列化为字符串（双向兼容，SSO 端同构 deserialize）
        assert!(json.contains("\"user_id\":\"42\""));
        assert!(json.contains("\"resource\":\"document:123\""));
        assert!(json.contains("\"action\":\"read\""));
    }

    #[test]
    fn test_pdp_check_response_deserialize() {
        let json = r#"{
            "permitted": true,
            "reason": "Access granted"
        }"#;

        let response: PdpCheckResponse = serde_json::from_str(json).unwrap();
        assert!(response.permitted);
        assert_eq!(response.reason, "Access granted");
    }

    #[test]
    fn test_pdp_check_response_deserialize_deny() {
        let json = r#"{
            "permitted": false,
            "reason": "Access denied by policy"
        }"#;

        let response: PdpCheckResponse = serde_json::from_str(json).unwrap();
        assert!(!response.permitted);
        assert_eq!(response.reason, "Access denied by policy");
    }

    #[test]
    fn test_pdp_check_request_missing_fields() {
        let json = r#"{
            "user_id": 42
        }"#;

        let result: Result<PdpCheckRequest, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_pdp_error_display() {
        let err = PdpError::TokenDecodeError("Invalid token".to_string());
        assert!(err.to_string().contains("Token decode failed"));

        let err = PdpError::NetworkError("Connection refused".to_string());
        assert!(err.to_string().contains("Network error"));

        let err = PdpError::InvalidResponse("JSON parse error".to_string());
        assert!(err.to_string().contains("Invalid PDP response"));

        let err = PdpError::PdpUnavailable("Service down".to_string());
        assert!(err.to_string().contains("PDP unavailable"));
    }
}
