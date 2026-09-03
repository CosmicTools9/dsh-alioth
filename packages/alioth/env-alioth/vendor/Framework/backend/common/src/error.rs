//! Alioth 统一错误类型
//!
//! 提供跨模块和 Gateway 共享的标准化错误类型，兼容 actix-web 的 ResponseError。

use actix_web::{http::StatusCode, HttpResponse, ResponseError};
use serde::Serialize;
use thiserror::Error;

/// 错误来源标注，用于区分跨系统边界错误的语义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorSource {
    Http,
    ZChat,
    Mqtt,
    Timewheel,
    Cluster,
    Database,
    Internal,
}

impl std::fmt::Display for ErrorSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorSource::Http => write!(f, "Http"),
            ErrorSource::ZChat => write!(f, "ZChat"),
            ErrorSource::Mqtt => write!(f, "MQTT"),
            ErrorSource::Timewheel => write!(f, "Timewheel"),
            ErrorSource::Cluster => write!(f, "Cluster"),
            ErrorSource::Database => write!(f, "Database"),
            ErrorSource::Internal => write!(f, "Internal"),
        }
    }
}

/// Alioth 统一应用错误类型
///
/// 所有模块和 Gateway 统一使用此错误类型，确保 API 错误响应格式一致。
///
/// ## 错误语义标注指南
///
/// | 场景 | 推荐变体 | 示例 |
/// |------|----------|------|
/// | HTTP 客户端参数错误 | `BadRequest` / `NotFound` | 缺少必填字段、资源不存在 |
/// | ZChat 消息投递失败 | `External { source: "ZChat", ... }` | MessageRouter 不可用 |
/// | MQTT 设备命令失败 | `External { source: "MQTT", ... }` | 设备离线、topic 不存在 |
/// | 时间轮调度失败 | `External { source: "Timewheel", ... }` | 任务队列满 |
/// | 集群广播失败 | `External { source: "Cluster", ... }` | 无可用节点 |
/// | 数据库操作失败 | `Database` | 连接超时、约束违反 |
/// | 其他内部错误 | `Internal` | 未知 panic、逻辑错误 |
#[derive(Debug, Error)]
pub enum AliothError {
    /// 请求参数验证失败 (HTTP 400)
    #[error("请求错误：{0}")]
    BadRequest(String),

    /// 未授权 (HTTP 401)
    #[error("未授权：{0}")]
    Unauthorized(String),

    /// 禁止访问 (HTTP 403)
    #[error("禁止访问：{0}")]
    Forbidden(String),

    /// 资源不存在 (HTTP 404)
    #[error("资源不存在：{0}")]
    NotFound(String),

    /// 字段验证错误 (HTTP 400)
    #[error("字段 '{field}' 验证失败：{message}")]
    Validation {
        /// 验证失败的字段名
        field: String,
        /// 验证失败的具体消息
        message: String,
    },

    /// 内部服务器错误 (HTTP 500)
    #[error("内部错误：{0}")]
    Internal(String),

    /// 数据库错误 (HTTP 500)
    #[error("数据库错误：{0}")]
    Database(String),

    /// 序列化/反序列化错误 (HTTP 500)
    #[error("序列化错误：{0}")]
    Serialization(#[from] serde_json::Error),

    /// 功能尚未实现 (HTTP 501)
    #[error("功能未实现：{0}")]
    NotImplemented(String),

    /// 外部子系统错误（ZChat、MQTT、Timewheel、Cluster 等基础设施错误）
    #[error("[{subsystem}] {message}")]
    External {
        /// 错误来源子系统
        subsystem: String,
        /// 具体错误消息
        message: String,
    },

    /// 外部服务不可用 (HTTP 503)
    #[error("服务不可用：{0}")]
    ServiceUnavailable(String),
}

impl AliothError {
    /// 从 sqlx 错误智能转换为 AliothError
    ///
    /// - `RowNotFound` → `NotFound`
    /// - 约束违反 → `BadRequest`
    /// - 其他 → `Database`
    pub fn from_sqlx(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => AliothError::NotFound("资源不存在".to_string()),
            sqlx::Error::Database(dbe) if dbe.constraint().is_some() => {
                AliothError::BadRequest(format!("约束违反: {}", dbe))
            }
            _ => AliothError::Database(err.to_string()),
        }
    }

    /// 构造外部子系统错误。
    ///
    /// 用于 ZChat、MQTT、Timewheel、Cluster 等基础设施错误的语义标注。
    ///
    /// # 示例
    /// ```
    /// use common::error::{AliothError, ErrorSource};
    /// let err = AliothError::external(ErrorSource::ZChat, "MessageRouter unavailable");
    /// ```
    pub fn external(source: ErrorSource, message: impl Into<String>) -> Self {
        Self::External {
            subsystem: source.to_string(),
            message: message.into(),
        }
    }

    /// 获取错误来源标注。
    ///
    /// 返回 `Some("ZChat")` / `Some("MQTT")` 等；非 `External` 变体返回 `None`。
    pub fn error_source(&self) -> Option<&str> {
        match self {
            Self::External { subsystem, .. } => Some(subsystem),
            Self::Database(_) => Some("Database"),
            Self::Internal(_) => Some("Internal"),
            Self::Serialization(_) => Some("Serialization"),
            _ => Some("Http"),
        }
    }
}

impl From<sqlx::Error> for AliothError {
    fn from(err: sqlx::Error) -> Self {
        Self::from_sqlx(err)
    }
}

/// 错误响应结构
///
/// 所有后端服务统一使用此结构返回错误，序列化为 `{code, message, details?}`。
#[derive(Serialize, Debug, Clone)]
pub struct ErrorResponse {
    /// 错误代码，用于程序识别
    pub code: String,
    /// 人类可读的错误消息
    pub message: String,
    /// 额外的错误详情（可选）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ErrorResponse {
    /// 构造 Not Found 错误响应
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: "NOT_FOUND".to_string(),
            message: message.into(),
            details: None,
        }
    }

    /// 构造 Bad Request 错误响应
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            code: "BAD_REQUEST".to_string(),
            message: message.into(),
            details: None,
        }
    }

    /// 构造 Unauthorized 错误响应
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            code: "UNAUTHORIZED".to_string(),
            message: message.into(),
            details: None,
        }
    }

    /// 构造 Forbidden 错误响应
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            code: "FORBIDDEN".to_string(),
            message: message.into(),
            details: None,
        }
    }

    /// 构造 Internal Server Error 响应
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "INTERNAL_ERROR".to_string(),
            message: message.into(),
            details: None,
        }
    }

    /// 构造 Validation Error 响应
    pub fn validation(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: "VALIDATION_ERROR".to_string(),
            message: format!("Validation error on '{}': {}", field.into(), message.into()),
            details: None,
        }
    }
}

impl ResponseError for AliothError {
    fn status_code(&self) -> StatusCode {
        match self {
            AliothError::BadRequest(_) | AliothError::Validation { .. } => StatusCode::BAD_REQUEST,
            AliothError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AliothError::Forbidden(_) => StatusCode::FORBIDDEN,
            AliothError::NotFound(_) => StatusCode::NOT_FOUND,
            AliothError::Internal(_)
            | AliothError::Database(_)
            | AliothError::Serialization(_)
            | AliothError::External { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            AliothError::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            AliothError::NotImplemented(_) => StatusCode::NOT_IMPLEMENTED,
        }
    }

    fn error_response(&self) -> HttpResponse {
        let code = match self {
            AliothError::BadRequest(_) => "BAD_REQUEST",
            AliothError::Unauthorized(_) => "UNAUTHORIZED",
            AliothError::Forbidden(_) => "FORBIDDEN",
            AliothError::NotFound(_) => "NOT_FOUND",
            AliothError::Validation { .. } => "VALIDATION_ERROR",
            AliothError::Internal(_) => "INTERNAL_ERROR",
            AliothError::Database(_) => "DATABASE_ERROR",
            AliothError::Serialization(_) => "SERIALIZATION_ERROR",
            AliothError::External { .. } => "EXTERNAL_ERROR",
            AliothError::ServiceUnavailable(_) => "SERVICE_UNAVAILABLE",
            AliothError::NotImplemented(_) => "NOT_IMPLEMENTED",
        };

        HttpResponse::build(self.status_code()).json(ErrorResponse {
            code: code.to_string(),
            message: self.to_string(),
            details: None,
        })
    }
}

/// 便捷类型别名
pub type Result<T> = std::result::Result<T, AliothError>;

impl ErrorResponse {
    /// 通用构造器，兼容旧 ApiErrorResponse 的两参数签名
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    /// 构造 Conflict 错误响应
    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            code: "CONFLICT".to_string(),
            message: message.into(),
            details: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::body::MessageBody;
    use actix_web::ResponseError;

    #[test]
    fn test_error_response_matches_l1_contract() {
        let err = AliothError::NotFound("org not found".to_string());
        let response = err.error_response();
        let body = response.into_body().try_into_bytes().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["code"], "NOT_FOUND");
        assert!(json["message"].as_str().unwrap().contains("org not found"));
    }

    #[test]
    fn test_all_l1_error_variants_serialize_with_code_field() {
        let cases = vec![
            (AliothError::BadRequest("bad".to_string()), "BAD_REQUEST"),
            (
                AliothError::Unauthorized("unauth".to_string()),
                "UNAUTHORIZED",
            ),
            (AliothError::Forbidden("forbid".to_string()), "FORBIDDEN"),
            (AliothError::NotFound("nf".to_string()), "NOT_FOUND"),
            (AliothError::Internal("int".to_string()), "INTERNAL_ERROR"),
            (AliothError::Database("db".to_string()), "DATABASE_ERROR"),
            (
                AliothError::ServiceUnavailable("fssc down".to_string()),
                "SERVICE_UNAVAILABLE",
            ),
        ];

        for (err, expected_code) in cases {
            let response = err.error_response();
            let body = response.into_body().try_into_bytes().unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(
                json["code"], expected_code,
                "Expected code {} for error variant",
                expected_code
            );
        }
    }

    #[test]
    fn test_sqlx_not_found_maps_to_not_found() {
        let sqlx_err = sqlx::Error::RowNotFound;
        let api_err: AliothError = sqlx_err.into();
        let response = api_err.error_response();
        let body = response.into_body().try_into_bytes().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["code"], "NOT_FOUND");
    }
}
