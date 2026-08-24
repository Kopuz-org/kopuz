use serde::{Deserialize, Serialize};

/// Stable, machine-readable error codes. Frontends localize by code; the
/// daemon never sends localized strings. New codes may be added within an API
/// version, so clients must treat unknown codes as [`ErrorCode::Internal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    Unauthorized,
    NotFound,
    Conflict,
    SourceAuthExpired,
    SourceUnreachable,
    Unsupported,
    Internal,
}

impl ErrorCode {
    pub fn http_status(self) -> u16 {
        match self {
            Self::InvalidInput => 400,
            Self::Unauthorized | Self::SourceAuthExpired => 401,
            Self::NotFound => 404,
            Self::Conflict => 409,
            Self::Internal => 500,
            Self::Unsupported => 501,
            Self::SourceUnreachable => 502,
        }
    }
}

/// The JSON error payload: `{"error": {"code", "message", "details"}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unsupported, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    pub fn body(&self) -> ErrorBody {
        ErrorBody {
            code: self.code,
            message: self.message.clone(),
            details: self.details.clone(),
        }
    }
}

impl From<ErrorBody> for ApiError {
    fn from(body: ErrorBody) -> Self {
        Self {
            code: body.code,
            message: body.message,
            details: body.details,
        }
    }
}
