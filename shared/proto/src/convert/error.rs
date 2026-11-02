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
        api::ErrorCode::SourceChallenged => ErrorCode::SourceChallenged,
        api::ErrorCode::SourceRefused => ErrorCode::SourceRefused,
        api::ErrorCode::ItemUnavailable => ErrorCode::ItemUnavailable,
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
        ErrorCode::SourceChallenged => api::ErrorCode::SourceChallenged,
        ErrorCode::SourceRefused => api::ErrorCode::SourceRefused,
        ErrorCode::ItemUnavailable => api::ErrorCode::ItemUnavailable,
        ErrorCode::Internal | ErrorCode::Unspecified => api::ErrorCode::Internal,
    }
}

pub fn error_reason_to_proto(value: Option<api::ErrorReason>) -> ErrorReason {
    use api::ErrorReason as R;
    match value {
        None => ErrorReason::Unspecified,
        Some(R::SettingLocked) => ErrorReason::SettingLocked,
        Some(R::ConnectNotOffered) => ErrorReason::ConnectNotOffered,
        Some(R::ApiKeyMissing) => ErrorReason::ApiKeyMissing,
        Some(R::SignInNotStarted) => ErrorReason::SignInNotStarted,
        Some(R::BrowserNotOpened) => ErrorReason::BrowserNotOpened,
        Some(R::SignInTimedOut) => ErrorReason::SignInTimedOut,
        Some(R::BrowserProfileGone) => ErrorReason::BrowserProfileGone,
        Some(R::BrowserCookiesUnreadable) => ErrorReason::BrowserCookiesUnreadable,
        Some(R::BrowserProfileSignedOut) => ErrorReason::BrowserProfileSignedOut,
        Some(R::BrowserProfileNoToken) => ErrorReason::BrowserProfileNoToken,
        Some(R::IntegrationUnreachable) => ErrorReason::IntegrationUnreachable,
        Some(R::CredentialRejected) => ErrorReason::CredentialRejected,
        Some(R::DefaultSourceUndeletable) => ErrorReason::DefaultSourceUndeletable,
        Some(R::EmptySourceFolder) => ErrorReason::EmptySourceFolder,
        Some(R::SourceNameRequired) => ErrorReason::SourceNameRequired,
        Some(R::SourceFolderRequired) => ErrorReason::SourceFolderRequired,
        Some(R::SpotifyClientIdRequired) => ErrorReason::SpotifyClientIdRequired,
        Some(R::AppleMusicStorefrontRequired) => ErrorReason::AppleMusicStorefrontRequired,
        Some(R::AppleMusicTokenRequired) => ErrorReason::AppleMusicTokenRequired,
        Some(R::InvalidSmbUrl) => ErrorReason::InvalidSmbUrl,
        Some(R::InvalidServerUrl) => ErrorReason::InvalidServerUrl,
        Some(R::NetworkDown) => ErrorReason::NetworkDown,
        Some(R::OfflineMode) => ErrorReason::OfflineMode,
    }
}

/// A reason this build does not know reads as none, so the message stands in for it.
pub fn error_reason_from_proto(value: i32) -> Option<api::ErrorReason> {
    use api::ErrorReason as R;
    match ErrorReason::try_from(value).unwrap_or(ErrorReason::Unspecified) {
        ErrorReason::Unspecified => None,
        ErrorReason::SettingLocked => Some(R::SettingLocked),
        ErrorReason::ConnectNotOffered => Some(R::ConnectNotOffered),
        ErrorReason::ApiKeyMissing => Some(R::ApiKeyMissing),
        ErrorReason::SignInNotStarted => Some(R::SignInNotStarted),
        ErrorReason::BrowserNotOpened => Some(R::BrowserNotOpened),
        ErrorReason::SignInTimedOut => Some(R::SignInTimedOut),
        ErrorReason::BrowserProfileGone => Some(R::BrowserProfileGone),
        ErrorReason::BrowserCookiesUnreadable => Some(R::BrowserCookiesUnreadable),
        ErrorReason::BrowserProfileSignedOut => Some(R::BrowserProfileSignedOut),
        ErrorReason::BrowserProfileNoToken => Some(R::BrowserProfileNoToken),
        ErrorReason::IntegrationUnreachable => Some(R::IntegrationUnreachable),
        ErrorReason::CredentialRejected => Some(R::CredentialRejected),
        ErrorReason::DefaultSourceUndeletable => Some(R::DefaultSourceUndeletable),
        ErrorReason::EmptySourceFolder => Some(R::EmptySourceFolder),
        ErrorReason::SourceNameRequired => Some(R::SourceNameRequired),
        ErrorReason::SourceFolderRequired => Some(R::SourceFolderRequired),
        ErrorReason::SpotifyClientIdRequired => Some(R::SpotifyClientIdRequired),
        ErrorReason::AppleMusicStorefrontRequired => Some(R::AppleMusicStorefrontRequired),
        ErrorReason::AppleMusicTokenRequired => Some(R::AppleMusicTokenRequired),
        ErrorReason::InvalidSmbUrl => Some(R::InvalidSmbUrl),
        ErrorReason::InvalidServerUrl => Some(R::InvalidServerUrl),
        ErrorReason::NetworkDown => Some(R::NetworkDown),
        ErrorReason::OfflineMode => Some(R::OfflineMode),
    }
}

pub fn error_body_to_proto(value: &api::ErrorBody) -> ErrorBody {
    ErrorBody {
        code: error_code_to_proto(value.code) as i32,
        message: value.message.clone(),
        reason: error_reason_to_proto(value.reason) as i32,
    }
}

pub fn error_body_from_proto(value: &ErrorBody) -> api::ErrorBody {
    api::ErrorBody {
        code: error_code_from_proto(value.code),
        message: value.message.clone(),
        reason: error_reason_from_proto(value.reason),
    }
}
