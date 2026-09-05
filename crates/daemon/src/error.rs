use api::{ApiError, ErrorCode};
use server::source::SourceError;

pub(crate) fn db(error: db::DbError) -> ApiError {
    ApiError::internal(format!("database error: {error}"))
}

pub(crate) fn source(error: SourceError) -> ApiError {
    match error {
        SourceError::Unsupported(operation) => ApiError::unsupported(operation),
        SourceError::Connectivity => {
            ApiError::new(ErrorCode::SourceUnreachable, "the source is unreachable")
        }
        SourceError::Auth => ApiError::new(
            ErrorCode::SourceAuthExpired,
            "the source needs authentication",
        ),
        SourceError::InvalidInput(message) => ApiError::invalid_input(message),
        SourceError::Backend(message) => ApiError::internal(message),
    }
}
