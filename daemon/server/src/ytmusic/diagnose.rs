//! A connection test: every step between Kopuz and a playing YouTube Music
//! stream, run one by one and reported whether it passed, so a broken day
//! says which step broke instead of "all stream paths failed".

use std::future::Future;
use std::time::Duration;

use super::{SessionCheck, YouTubeMusicClient, botguard, clients, player};
use crate::source::{Check, CheckStep};

/// A long-lived public music video. Any playable one would do; this one has
/// stayed up and unrestricted everywhere for well over a decade.
const PROBE_VIDEO: &str = "dQw4w9WgXcQ";

/// Per step, so one that hangs is reported as such and the rest still run.
const STEP_TIMEOUT: Duration = Duration::from_secs(20);

async fn bounded<T>(step: impl Future<Output = Result<T, String>>) -> Result<T, String> {
    tokio::time::timeout(STEP_TIMEOUT, step)
        .await
        .unwrap_or_else(|_| Err(format!("no answer within {}s", STEP_TIMEOUT.as_secs())))
}

/// Runs as the client's chosen account, so it tests what playback uses.
pub(crate) async fn run(client: &YouTubeMusicClient) -> Vec<Check> {
    let mut checks = Vec::with_capacity(5);

    checks.push(Check::of(
        CheckStep::Reach,
        bounded(clients::refresh_live_version()).await,
        |live| {
            let baked = clients::WEB_REMIX.baked_version;
            if live == baked {
                format!("web client {live}")
            } else {
                format!("web client {live} (this build shipped {baked})")
            }
        },
    ));

    match client.cookies() {
        Some(cookies) => {
            let session = bounded(async {
                match client.check_session().await {
                    SessionCheck::Valid => Ok(()),
                    SessionCheck::SignedOut => {
                        Err("YouTube answers this session as signed out".to_string())
                    }
                    SessionCheck::Unknown(error) => Err(error),
                }
            })
            .await;
            checks.push(Check::of(CheckStep::Session, session, |()| {
                "signed in".to_string()
            }));
            checks.push(Check::of(
                CheckStep::SignedInPlayback,
                bounded(client.account(player::probe_signed_in(PROBE_VIDEO, cookies))).await,
                describe,
            ));
        }
        None => {
            checks.push(Check::skipped(CheckStep::Session, "not signed in"));
            checks.push(Check::skipped(CheckStep::SignedInPlayback, "not signed in"));
        }
    }

    checks.push(Check::of(
        CheckStep::AnonymousPlayback,
        bounded(player::probe_anonymous(PROBE_VIDEO)).await,
        describe,
    ));

    // The minter boots on its first request, so asking is the test.
    checks.push(if cfg!(target_os = "android") {
        Check::skipped(CheckStep::PlaybackToken, "no token minter on Android")
    } else {
        Check::of(
            CheckStep::PlaybackToken,
            bounded(botguard::mint_content_pot(PROBE_VIDEO)).await,
            |_| "minted".to_string(),
        )
    });

    checks
}

fn describe(info: player::YtStreamInfo) -> String {
    let kbps = info.bitrate.map(|bits| bits / 1000).unwrap_or_default();
    match info.itag {
        Some(itag) => format!("itag {itag}, {kbps} kbps"),
        None => format!("{kbps} kbps"),
    }
}
