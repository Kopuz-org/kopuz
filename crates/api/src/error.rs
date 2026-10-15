/// Stable, machine-readable error codes. Frontends localize by code; the
/// daemon never sends localized strings. New codes may be added within an
/// API version; an unrecognized one is treated as [`ErrorCode::Internal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    InvalidInput,
    NotFound,
    Conflict,
    SourceAuthExpired,
    SourceUnreachable,
    /// The daemon is not running, or its socket went away mid-call. Distinct
    /// from SourceUnreachable, which is a media server failing to answer.
    DaemonGone,
    Unsupported,
    Internal,
    /// The source wants proof the request is not a bot, from this network.
    SourceChallenged,
    /// The source answered and refused, with nothing pointing at the
    /// account, the network or the item: an outage or a change on its side.
    SourceRefused,
    /// The source will not serve this item: removed, private or
    /// region-locked.
    ItemUnavailable,
}

/// Why a call failed, for a frontend to say in its own language; the daemon has no locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorReason {
    /// A managed settings file, drop-in or env var pins a setting this would change.
    SettingLocked,
    /// The integration does not connect this way.
    ConnectNotOffered,
    /// A web sign-in needs the API key and secret filled in first.
    ApiKeyMissing,
    /// The service did not hand out a sign-in token.
    SignInNotStarted,
    /// No browser could be opened for the approval page.
    BrowserNotOpened,
    /// Nobody approved the sign-in before it gave up.
    SignInTimedOut,
    /// The browser profile picked is no longer on this machine.
    BrowserProfileGone,
    /// The browser profile's cookies could not be read.
    BrowserCookiesUnreadable,
    /// The browser profile is not signed in to the service.
    BrowserProfileSignedOut,
    /// The service showed no token for the browser profile's session.
    BrowserProfileNoToken,
    /// The service could not be reached to check a credential.
    IntegrationUnreachable,
    /// The service does not accept the credential it was given.
    CredentialRejected,
    /// The built-in folder source is always there.
    DefaultSourceUndeletable,
    /// A source folder was left blank.
    EmptySourceFolder,
}

impl ErrorReason {
    pub const ALL: [ErrorReason; 14] = [
        Self::SettingLocked,
        Self::ConnectNotOffered,
        Self::ApiKeyMissing,
        Self::SignInNotStarted,
        Self::BrowserNotOpened,
        Self::SignInTimedOut,
        Self::BrowserProfileGone,
        Self::BrowserCookiesUnreadable,
        Self::BrowserProfileSignedOut,
        Self::BrowserProfileNoToken,
        Self::IntegrationUnreachable,
        Self::CredentialRejected,
        Self::DefaultSourceUndeletable,
        Self::EmptySourceFolder,
    ];

    /// The i18n key every frontend says this reason with, so they all phrase it alike.
    pub fn message_key(self) -> &'static str {
        match self {
            Self::SettingLocked => "error_setting_locked",
            Self::ConnectNotOffered => "error_connect_not_offered",
            Self::ApiKeyMissing => "error_api_key_missing",
            Self::SignInNotStarted => "error_sign_in_not_started",
            Self::BrowserNotOpened => "error_browser_not_opened",
            Self::SignInTimedOut => "error_sign_in_timed_out",
            Self::BrowserProfileGone => "error_browser_profile_gone",
            Self::BrowserCookiesUnreadable => "error_browser_cookies_unreadable",
            Self::BrowserProfileSignedOut => "error_browser_profile_signed_out",
            Self::BrowserProfileNoToken => "error_browser_profile_no_token",
            Self::IntegrationUnreachable => "error_integration_unreachable",
            Self::CredentialRejected => "error_credential_rejected",
            Self::DefaultSourceUndeletable => "error_default_source_undeletable",
            Self::EmptySourceFolder => "error_empty_source_folder",
        }
    }
}

/// A failure carried inside a message rather than as an RPC status: the
/// result of a mutation that failed, and the terminal state of a failed job.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    pub reason: Option<ErrorReason>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct ApiError {
    pub code: ErrorCode,
    /// English and for logs; a frontend shows it only when there is no known reason.
    pub message: String,
    pub reason: Option<ErrorReason>,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            reason: None,
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

    pub fn because(self, reason: ErrorReason) -> Self {
        Self {
            reason: Some(reason),
            ..self
        }
    }

    pub fn body(&self) -> ErrorBody {
        ErrorBody {
            code: self.code,
            message: self.message.clone(),
            reason: self.reason,
        }
    }
}

impl From<ErrorBody> for ApiError {
    fn from(body: ErrorBody) -> Self {
        Self {
            code: body.code,
            message: body.message,
            reason: body.reason,
        }
    }
}
