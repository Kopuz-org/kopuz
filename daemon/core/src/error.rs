use api::ApiError;

pub(crate) fn source_error(error: server::source::SourceError) -> ApiError {
    use api::ErrorCode;
    use server::source::SourceError;
    match &error {
        SourceError::Unsupported(what) => ApiError::unsupported(*what),
        SourceError::Auth => ApiError::new(ErrorCode::SourceAuthExpired, error.to_string()),
        SourceError::Connectivity => ApiError::new(ErrorCode::SourceUnreachable, error.to_string()),
        SourceError::InvalidInput(message) => ApiError::invalid_input(message.clone()),
        SourceError::Backend(message) => ApiError::internal(message.clone()),
        SourceError::Refused(_, message) => {
            ApiError::new(crate::sources::source_error_code(&error), message.clone())
        }
    }
}
