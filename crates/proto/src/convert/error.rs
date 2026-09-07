use crate::*;

pub fn error_code_to_proto(value: api::ErrorCode) -> ErrorCode {
    match value {
        api::ErrorCode::InvalidInput => ErrorCode::InvalidInput,
        api::ErrorCode::NotFound => ErrorCode::NotFound,
        api::ErrorCode::Conflict => ErrorCode::Conflict,
        api::ErrorCode::SourceAuthExpired => ErrorCode::SourceAuthExpired,
        api::ErrorCode::SourceUnreachable => ErrorCode::SourceUnreachable,
        api::ErrorCode::DaemonGone => ErrorCode::DaemonGone,
        api::ErrorCode::Unsupported => ErrorCode::Unsupported,
        api::ErrorCode::Internal => ErrorCode::Internal,
    }
}

pub fn error_code_from_proto(value: i32) -> api::ErrorCode {
    match ErrorCode::try_from(value).unwrap_or(ErrorCode::Unspecified) {
        ErrorCode::InvalidInput => api::ErrorCode::InvalidInput,
        ErrorCode::NotFound => api::ErrorCode::NotFound,
        ErrorCode::Conflict => api::ErrorCode::Conflict,
        ErrorCode::SourceAuthExpired => api::ErrorCode::SourceAuthExpired,
        ErrorCode::SourceUnreachable => api::ErrorCode::SourceUnreachable,
        ErrorCode::DaemonGone => api::ErrorCode::DaemonGone,
        ErrorCode::Unsupported => api::ErrorCode::Unsupported,
        ErrorCode::Internal | ErrorCode::Unspecified => api::ErrorCode::Internal,
    }
}

pub fn error_body_to_proto(value: &api::ErrorBody) -> ErrorBody {
    ErrorBody {
        code: error_code_to_proto(value.code) as i32,
        message: value.message.clone(),
    }
}

pub fn error_body_from_proto(value: &ErrorBody) -> api::ErrorBody {
    api::ErrorBody {
        code: error_code_from_proto(value.code),
        message: value.message.clone(),
    }
}
