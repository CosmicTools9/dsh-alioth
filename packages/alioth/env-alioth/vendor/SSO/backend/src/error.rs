use actix_web::{http::StatusCode, HttpResponse};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum SsoError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Invalid credentials")]
    InvalidCredentials,

    #[error("User not found")]
    UserNotFound,

    #[error("Session not found")]
    SessionNotFound,

    #[error("Invalid token")]
    InvalidToken,

    #[error("Token expired")]
    TokenExpired,

    #[error("MFA required")]
    MfaRequired,

    #[error("MFA invalid")]
    MfaInvalid,

    #[error("OAuth error: {0}")]
    OAuth(String),

    #[error("Internal server error")]
    Internal(String),
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
    pub code: u16,
}

impl actix_web::ResponseError for SsoError {
    fn status_code(&self) -> StatusCode {
        match self {
            SsoError::InvalidCredentials => StatusCode::UNAUTHORIZED,
            SsoError::UserNotFound => StatusCode::NOT_FOUND,
            SsoError::SessionNotFound => StatusCode::NOT_FOUND,
            SsoError::InvalidToken => StatusCode::UNAUTHORIZED,
            SsoError::TokenExpired => StatusCode::UNAUTHORIZED,
            SsoError::MfaRequired => StatusCode::BAD_REQUEST,
            SsoError::MfaInvalid => StatusCode::BAD_REQUEST,
            SsoError::OAuth(_) => StatusCode::BAD_REQUEST,
            SsoError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            SsoError::Auth(_) => StatusCode::UNAUTHORIZED,
            SsoError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).json(ErrorResponse {
            error: self.to_string(),
            message: self.to_string(),
            code: self.status_code().as_u16(),
        })
    }
}

pub type Result<T> = std::result::Result<T, SsoError>;
