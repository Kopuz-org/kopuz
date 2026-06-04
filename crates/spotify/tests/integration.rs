//! Integration tests requiring a real Spotify Premium account.
//!
//! These tests are `#[ignore]` by default. Run explicitly with:
//!   `cargo test -p spotify -- --ignored`
//!
//! Required environment variables:
//!   KOPUZ_SPOTIFY_CLIENT_ID  - Spotify Dashboard client_id
//!   KOPUZ_SPOTIFY_OPT_IN=1   - explicit opt-in
//!
//! Tokens are persisted via `MemoryTokenStore` so these tests do NOT touch
//! the user's real keyring. Never run in CI without the opt-in variable.

use spotify::auth::AuthCore;
use spotify::backends::connect::SpotifyConnectBackend;
use spotify::provider::StreamingProvider;
use spotify::token_store::MemoryTokenStore;
use spotify::types::SpotifyConfig;
use std::sync::Arc;

fn opted_in() -> Option<String> {
    if std::env::var("KOPUZ_SPOTIFY_OPT_IN").ok().as_deref() != Some("1") {
        return None;
    }
    std::env::var("KOPUZ_SPOTIFY_CLIENT_ID").ok()
}

#[tokio::test]
#[ignore]
async fn smoke_login_profile_and_devices() {
    let Some(client_id) = opted_in() else {
        eprintln!("skipped: set KOPUZ_SPOTIFY_OPT_IN=1 and KOPUZ_SPOTIFY_CLIENT_ID to run");
        return;
    };
    let store = Arc::new(MemoryTokenStore::new());
    let http = reqwest::Client::new();
    let auth = Arc::new(AuthCore::new(http, client_id.clone(), store));
    let cfg = SpotifyConfig {
        enabled: true,
        client_id,
        ..SpotifyConfig::default()
    };
    let backend = SpotifyConnectBackend::new(cfg, auth.clone());
    backend.login().await.expect("login");
    let me = backend.api.current_user_profile().await.expect("profile");
    assert!(!me.id.is_empty());
    let _devices = backend.devices().await.expect("devices");
}

#[cfg(feature = "spotify-web-playback")]
#[tokio::test]
#[ignore]
async fn web_playback_smoke_only_with_webview_host() {
    // This test does nothing useful without a real WebView host wired to the
    // bridge. It exists as a placeholder for host applications to extend.
    eprintln!("requires a WebView host; see crates/spotify/src/web_playback/spotify_player.html");
}
