//! Probe: does Spirc-driven playback (active Connect device) get audio keys
//! where the bare `Player::load` path does not?
//!
//! Run: cargo run -p spotify --features spotify-librespot --example spirc_probe
//!
//! Uses the credential blob already cached by the app, so no browser login.
//! Watch the log: success = "Loading <title>" then audio with no
//! "audio key error". Failure = the same `error audio key 0 1`.

use librespot::connect::{ConnectConfig, LoadRequest, LoadRequestOptions, Spirc};
use librespot::core::{cache::Cache, config::SessionConfig, session::Session};
use librespot::playback::config::{AudioFormat, PlayerConfig};
use librespot::playback::mixer::{self, MixerConfig};
use librespot::playback::{audio_backend, player::Player};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,librespot=debug"))
        .init();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let dir = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|d| d.cache_dir().join("spotify"))
        .expect("cache dir");
    println!("cache dir: {}", dir.display());
    let files = dir.join("files");
    let cache = Cache::new(Some(dir.clone()), Some(dir.clone()), Some(files), None)?;
    let credentials = cache
        .credentials()
        .expect("no cached credentials — log in via the app first");

    let session_config = SessionConfig::default();
    let session = Session::new(session_config, Some(cache));

    let backend = audio_backend::find(None).unwrap();
    let mixer = mixer::find(None).unwrap()(MixerConfig::default())?;
    let player = Player::new(
        PlayerConfig::default(),
        session.clone(),
        mixer.get_soft_volume(),
        move || backend(None, AudioFormat::default()),
    );

    let (spirc, spirc_task) = Spirc::new(
        ConnectConfig::default(),
        session.clone(),
        credentials,
        player,
        mixer,
    )
    .await?;

    spirc.activate()?;
    spirc.load(LoadRequest::from_context_uri(
        format!("spotify:user:{}:collection", session.username()),
        LoadRequestOptions::default(),
    ))?;
    spirc.play()?;

    // Run the spirc task for a bit, then quit.
    tokio::select! {
        _ = spirc_task => {}
        _ = tokio::time::sleep(std::time::Duration::from_secs(20)) => {
            println!("--- 20s elapsed, stopping ---");
        }
    }
    Ok(())
}
