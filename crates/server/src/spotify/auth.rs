//! Spotify OAuth (authorization-code + PKCE) sign-in via `librespot-oauth`.
//!
//! The flow opens the user's browser, spins up a localhost listener on the
//! redirect port, and blocks until Spotify redirects back with the auth code —
//! the same UX contract as `soundcloud::signin::launch_signin_and_extract`. We
//! keep both the access token (short-lived, ~1h) and the refresh token; the
//! keepalive task ([`crate::spotify::session`]) refreshes before expiry.

use librespot_oauth::OAuthClientBuilder;

use super::session;

/// Loopback redirect — port 5588 is the one librespot registers for the
/// keymaster client id, so it is accepted by Spotify's consent screen.
const REDIRECT_URI: &str = "http://127.0.0.1:5588/login";

/// Read access to the user's library + playlists, plus streaming (audio).
const SCOPES: &[&str] = &[
    "streaming",
    "playlist-read-private",
    "playlist-read-collaborative",
    "user-library-read",
    "user-read-private",
    "user-read-email",
];

/// The credential bundle captured from a successful sign-in.
pub struct SpotifyAuth {
    pub access_token: String,
    pub refresh_token: String,
    /// Spotify user id (canonical username) for display / config.
    pub user_id: String,
}

/// Pack `access` + `refresh` into the single `access_token` config column
/// (newline-separated) so no DB schema migration is needed.
pub fn pack_token(access: &str, refresh: &str) -> String {
    format!("{access}\n{refresh}")
}

/// Inverse of [`pack_token`] → `(access, refresh)`. Tolerates an un-packed
/// (access-only) value for forward/backward compatibility.
pub fn unpack_token(packed: &str) -> (String, String) {
    match packed.split_once('\n') {
        Some((a, r)) => (a.to_string(), r.to_string()),
        None => (packed.to_string(), String::new()),
    }
}

fn build_client(open_browser: bool) -> Result<librespot_oauth::OAuthClient, String> {
    let client_id = session::client_id();
    let mut b = OAuthClientBuilder::new(&client_id, REDIRECT_URI, SCOPES.to_vec());
    if open_browser {
        b = b.open_in_browser();
    }
    b.build().map_err(|e| format!("spotify oauth client: {e}"))
}

/// Open the browser, complete the PKCE flow, and resolve the signed-in user's
/// id. Blocks (on a blocking thread) until the browser redirect completes.
pub async fn launch_signin_and_extract() -> Result<SpotifyAuth, String> {
    let token = tokio::task::spawn_blocking(|| {
        let client = build_client(true)?;
        client
            .get_access_token()
            .map_err(|e| format!("spotify oauth: {e}"))
    })
    .await
    .map_err(|e| format!("spotify oauth task: {e}"))??;

    tracing::info!("spotify: oauth token obtained");
    let access = token.access_token.clone();
    let user_id = match session::on_rt(async move {
        let s = session::ensure_session(&access).await?;
        Ok::<String, String>(s.username())
    })
    .await
    {
        Ok(Ok(uid)) => uid,
        Ok(Err(e)) | Err(e) => {
            tracing::warn!(error = %e, "spotify: user_id resolve failed, storing token anyway");
            String::new()
        }
    };

    Ok(SpotifyAuth {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        user_id,
    })
}

/// Exchange a refresh token for a fresh access token. The returned
/// `refresh_token` may be empty (Spotify sometimes omits it) — the caller keeps
/// the previous one in that case.
pub async fn refresh(refresh_token: String) -> Result<SpotifyAuth, String> {
    let token = tokio::task::spawn_blocking(move || {
        let client = build_client(false)?;
        client
            .refresh_token(&refresh_token)
            .map_err(|e| format!("spotify token refresh: {e}"))
    })
    .await
    .map_err(|e| format!("spotify refresh task: {e}"))??;

    Ok(SpotifyAuth {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        user_id: String::new(),
    })
}

/// Whether the access token can still establish a session — for session-resume
/// validation. `Ok(())` = valid; `Err` = could not connect (expired or offline,
/// indistinguishable here).
pub async fn validate(access_token: &str) -> Result<(), String> {
    let access = access_token.to_string();
    session::on_rt(async move { session::ensure_session(&access).await.map(|_| ()) }).await?
}
