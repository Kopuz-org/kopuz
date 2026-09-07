//! ApiError <-> tonic Status, shared by the server and the Rust client so
//! the mapping cannot drift. It is one-to-one, so the status code alone
//! carries the Kopuz code and nothing rides alongside it in metadata.

use tonic::{Code, Status};

pub fn to_status(error: api::ApiError) -> Status {
    let grpc_code = match error.code {
        api::ErrorCode::InvalidInput => Code::InvalidArgument,
        // Not UNAUTHENTICATED: that says the caller's own token failed,
        // and a client answers it by re-reading the discovery file. An
        // expired source login is a precondition the user must fix.
        api::ErrorCode::SourceAuthExpired => Code::FailedPrecondition,
        api::ErrorCode::NotFound => Code::NotFound,
        api::ErrorCode::Conflict => Code::AlreadyExists,
        api::ErrorCode::Unsupported => Code::Unimplemented,
        api::ErrorCode::SourceUnreachable => Code::Unavailable,
        api::ErrorCode::Internal => Code::Internal,
    };
    Status::new(grpc_code, error.message)
}

pub fn from_status(status: &Status) -> api::ApiError {
    let code = match status.code() {
        Code::InvalidArgument => api::ErrorCode::InvalidInput,
        Code::FailedPrecondition => api::ErrorCode::SourceAuthExpired,
        Code::NotFound => api::ErrorCode::NotFound,
        Code::AlreadyExists => api::ErrorCode::Conflict,
        Code::Unimplemented => api::ErrorCode::Unsupported,
        Code::Unavailable => api::ErrorCode::SourceUnreachable,
        _ => api::ErrorCode::Internal,
    };
    api::ApiError {
        code,
        message: status.message().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_survives_the_status_round_trip() {
        let codes = [
            api::ErrorCode::InvalidInput,
            api::ErrorCode::NotFound,
            api::ErrorCode::Conflict,
            api::ErrorCode::SourceAuthExpired,
            api::ErrorCode::SourceUnreachable,
            api::ErrorCode::Unsupported,
            api::ErrorCode::Internal,
        ];
        for code in codes {
            let error = api::ApiError {
                code,
                message: "m".into(),
            };
            let back = from_status(&to_status(error.clone()));
            assert_eq!(error, back);
        }
    }
}
