//! ApiError <-> tonic Status, shared by the server and the Rust client so
//! the mapping cannot drift. Every code the daemon sends is one-to-one with
//! a status code. A reason rides in the status details as an [`ErrorDetail`],
//! absent when there is none.
//!
//! `DaemonGone` is the exception: the daemon cannot report its own absence,
//! so only `from_status` produces it, from a failure tonic raised locally.

use prost::Message;
use tonic::{Code, Status};

use crate::ErrorDetail;
use crate::convert::{error_reason_from_proto, error_reason_to_proto};

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
        api::ErrorCode::SourceChallenged => Code::ResourceExhausted,
        api::ErrorCode::SourceRefused => Code::Aborted,
        api::ErrorCode::ItemUnavailable => Code::PermissionDenied,
        // Never sent: the daemon is the thing that would be missing.
        api::ErrorCode::DaemonGone | api::ErrorCode::Internal => Code::Internal,
    };
    match error.reason {
        None => Status::new(grpc_code, error.message),
        Some(reason) => {
            let detail = ErrorDetail {
                reason: error_reason_to_proto(Some(reason)) as i32,
            };
            Status::with_details(grpc_code, error.message, detail.encode_to_vec().into())
        }
    }
}

pub fn from_status(status: &Status) -> api::ApiError {
    // tonic raises UNAVAILABLE for a transport failure of its own and
    // attaches the cause; a status the daemon actually sent arrives with
    // none. That is the only thing separating "the daemon is not there"
    // from "a media server did not answer", which the daemon does send.
    let from_transport = std::error::Error::source(status).is_some();
    let code = match status.code() {
        Code::InvalidArgument => api::ErrorCode::InvalidInput,
        Code::FailedPrecondition => api::ErrorCode::SourceAuthExpired,
        Code::NotFound => api::ErrorCode::NotFound,
        Code::AlreadyExists => api::ErrorCode::Conflict,
        Code::Unimplemented => api::ErrorCode::Unsupported,
        Code::Unavailable if from_transport => api::ErrorCode::DaemonGone,
        Code::Unavailable => api::ErrorCode::SourceUnreachable,
        Code::ResourceExhausted => api::ErrorCode::SourceChallenged,
        Code::Aborted => api::ErrorCode::SourceRefused,
        Code::PermissionDenied => api::ErrorCode::ItemUnavailable,
        _ => api::ErrorCode::Internal,
    };
    let reason = ErrorDetail::decode(status.details())
        .ok()
        .and_then(|detail| error_reason_from_proto(detail.reason));
    api::ApiError {
        code,
        message: status.message().to_string(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every code the daemon can send round-trips. `DaemonGone` is absent
    /// on purpose: it is synthesized by the client and never sent.
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
            api::ErrorCode::SourceChallenged,
            api::ErrorCode::SourceRefused,
            api::ErrorCode::ItemUnavailable,
        ];
        for code in codes {
            let error = api::ApiError::new(code, "m");
            let back = from_status(&to_status(error.clone()));
            assert_eq!(error, back);
        }
    }

    #[test]
    fn a_reason_survives_the_status_round_trip() {
        let error = api::ApiError::invalid_input("set the API key and secret first")
            .because(api::ErrorReason::ApiKeyMissing);
        let back = from_status(&to_status(error.clone()));
        assert_eq!(back, error);
        let plain = from_status(&Status::invalid_argument("no reason"));
        assert_eq!(plain.reason, None);
    }

    /// The daemon-sent half of the UNAVAILABLE split. The transport half
    /// cannot be faked here -- a hand-built Status carries no transport
    /// cause -- so it is covered end to end in the daemon's contract tests.
    #[test]
    fn a_media_server_failure_stays_a_media_server_failure() {
        let sent = to_status(api::ApiError::new(
            api::ErrorCode::SourceUnreachable,
            "jellyfin timed out",
        ));
        assert_eq!(sent.code(), Code::Unavailable);
        assert_eq!(from_status(&sent).code, api::ErrorCode::SourceUnreachable);
    }
}
