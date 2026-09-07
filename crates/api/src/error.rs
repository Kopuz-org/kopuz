/// Stable, machine-readable error codes. Frontends localize by code; the
/// daemon never sends localized strings. New codes may be added within an
/// API version; an unrecognized one is treated as [`ErrorCode::Internal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// A failure carried inside a message rather than as an RPC status: the
/// result of a mutation that failed, and the terminal state of a failed job.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
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
        }
    }
}

impl From<ErrorBody> for ApiError {
    fn from(body: ErrorBody) -> Self {
        Self {
            code: body.code,
            message: body.message,
        }
    }
}
