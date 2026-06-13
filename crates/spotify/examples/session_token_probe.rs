//! Throwaway probe: validate that all Spotify browsing data the app needs is
//! reachable through the librespot session's native spclient channel, with no
//! Web API involved.
//!
//! Run with:
//!   SPOT_TOKEN=$(security find-generic-password -s kopuz.spotify -a tokens -w | python3 -c "import sys,json; print(json.load(sys.stdin)['access_token'])") \
//!   cargo run -p spotify --example session_token_probe --features spotify-librespot

use librespot::core::{
    SpotifyUri, authentication::Credentials, config::SessionConfig, session::Session,
};
use librespot::metadata::{Metadata, Playlist, Track};
use librespot::protocol::playlist4_external::SelectedListContent;
use protobuf::Message;

#[tokio::main]
async fn main() {
    let access_token = std::env::var("SPOT_TOKEN").expect("SPOT_TOKEN env var");
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let mut cfg = SessionConfig::default();
    cfg.client_id = spotify::KEYMASTER_CLIENT_ID.to_string();
    let session = Session::new(cfg, None);
    session
        .connect(Credentials::with_access_token(access_token), false)
        .await
        .expect("session connect");
    let username = session.username();
    println!("session connected, username={username:?}");

    // 1. Rootlist: user playlists with names.
    let mut first_playlist_uri = None;
    match session.spclient().get_rootlist(0, Some(20)).await {
        Ok(bytes) => match SelectedListContent::parse_from_bytes(&bytes) {
            Ok(root) => {
                let items = &root.contents.items;
                let metas = &root.contents.meta_items;
                println!("rootlist ok: {} items, {} meta", items.len(), metas.len());
                for (i, item) in items.iter().enumerate().take(3) {
                    let name = metas
                        .get(i)
                        .and_then(|m| m.attributes.name.clone())
                        .unwrap_or_default();
                    println!("  playlist {}: {} name={:?}", i, item.uri(), name);
                    if first_playlist_uri.is_none() {
                        first_playlist_uri = Some(item.uri().to_string());
                    }
                }
            }
            Err(e) => println!("rootlist parse error: {e}"),
        },
        Err(e) => println!("rootlist error: {e}"),
    }

    // 2. Playlist contents + track hydration.
    if let Some(uri) = first_playlist_uri {
        match SpotifyUri::from_uri(&uri) {
            Ok(parsed) => match Playlist::get(&session, &parsed).await {
                Ok(pl) => {
                    println!("playlist ok: name={:?} len={}", pl.name(), pl.length);
                    if let Some(track_uri) = pl.tracks().next() {
                        match Track::get(&session, track_uri).await {
                            Ok(t) => println!(
                                "  track hydrate ok: {:?} by {:?} album {:?} {}ms",
                                t.name,
                                t.artists.first().map(|a| a.name.clone()),
                                t.album.name,
                                t.duration
                            ),
                            Err(e) => println!("  track hydrate error: {e}"),
                        }
                    }
                }
                Err(e) => println!("playlist error: {e}"),
            },
            Err(e) => println!("playlist uri parse error: {e}"),
        }
    }

    // 3. Liked songs via the collection context.
    let collection_uri = format!("spotify:user:{username}:collection");
    match session.spclient().get_context(&collection_uri).await {
        Ok(ctx) => {
            let n: usize = ctx.pages.iter().map(|p| p.tracks.len()).sum();
            println!("collection context ok: {} pages, {n} tracks", ctx.pages.len());
            if let Some(t) = ctx.pages.first().and_then(|p| p.tracks.first()) {
                println!("  first: uri={:?} metadata={:?}", t.uri, t.metadata);
            }
        }
        Err(e) => println!("collection context error: {e}"),
    }

    // 4. Search context.
    match session.spclient().get_context("spotify:search:daft+punk").await {
        Ok(ctx) => {
            let n: usize = ctx.pages.iter().map(|p| p.tracks.len()).sum();
            println!("search context ok: {} pages, {n} tracks", ctx.pages.len());
            if let Some(t) = ctx.pages.first().and_then(|p| p.tracks.first()) {
                println!("  first: uri={:?} metadata={:?}", t.uri, t.metadata);
            }
        }
        Err(e) => println!("search context error: {e}"),
    }
}
