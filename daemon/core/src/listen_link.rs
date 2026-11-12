//! "Listen on Kopuz" links; Discord buttons take only https, so the website hands the fragment to `kopuz://j/`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use api::{
    ApiError, ApiEvent, Intent, Phase, PlayerCommand, PlayerState, QueueContext, QueueMode,
    SetQueueRequest, SourceInfo,
};
use config::MusicService;
use futures_util::StreamExt;
use tokio::sync::broadcast;

const SITE: &str = "https://kopuz.moe/j#";
const SCHEME: &str = "kopuz://j/";
const VERSION: &str = "1";
/// Discord refuses a button whose URL is longer.
const MAX_URL: usize = 512;
/// A link older than this would land somewhere arbitrary in the song, so it starts from the top.
const STALE_SECS: u64 = 12 * 60 * 60;

/// Where the sharer is in the first song.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At {
    /// Playing, and at 0:00 at this Unix time.
    Since(u64),
    /// Paused this many seconds in.
    Paused(u64),
}

impl At {
    /// Playing `elapsed_secs` into the song as of now, or paused there.
    pub fn now(elapsed_secs: u64, playing: bool) -> Self {
        match playing {
            true => Self::Since(unix_now().saturating_sub(elapsed_secs)),
            false => Self::Paused(elapsed_secs),
        }
    }

    /// Milliseconds into the first song to start at, `now` being Unix seconds.
    pub fn position_ms(self, now: u64) -> u64 {
        let secs = match self {
            Self::Since(since) if since > now => 0,
            Self::Since(since) => now - since,
            Self::Paused(secs) => secs,
        };
        if secs > STALE_SECS { 0 } else { secs * 1000 }
    }

    fn encode(self) -> String {
        match self {
            Self::Since(since) => format!("s{since}"),
            Self::Paused(secs) => format!("p{secs}"),
        }
    }

    fn decode(raw: &str) -> Option<Self> {
        let (kind, value) = raw.split_at_checked(1)?;
        let value = value.parse().ok()?;
        match kind {
            "s" => Some(Self::Since(value)),
            "p" => Some(Self::Paused(value)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenLink {
    pub service: MusicService,
    pub at: At,
    /// The song first, then what follows it; never empty.
    pub keys: Vec<String>,
}

impl ListenLink {
    /// The button's URL with as many of `keys` as fit; `None` when the first is not one a link can carry.
    pub fn url<'a>(
        service: MusicService,
        at: At,
        keys: impl IntoIterator<Item = &'a str>,
    ) -> Option<String> {
        let mut url = format!("{SITE}{VERSION}/{}/{}/", service.id(), at.encode());
        let mut keys = keys.into_iter();
        let first = keys.next().filter(|key| is_key(key))?;
        url.push_str(first);
        for key in keys.filter(|key| is_key(key)) {
            if url.len() + 1 + key.len() > MAX_URL {
                break;
            }
            url.push(',');
            url.push_str(key);
        }
        (url.len() <= MAX_URL).then_some(url)
    }

    /// A link as the website hands it over, as the button carries it, or its bare payload.
    pub fn parse(link: &str) -> Option<Self> {
        let link = link.trim();
        let payload = link
            .strip_prefix(SCHEME)
            .or_else(|| link.strip_prefix(SITE))
            .unwrap_or(link)
            .trim_end_matches('/');
        let mut parts = payload.split('/');
        if parts.next()? != VERSION {
            return None;
        }
        let service = MusicService::from_id(parts.next()?)?;
        let at = At::decode(parts.next()?)?;
        let keys: Vec<String> = parts.next()?.split(',').map(str::to_string).collect();
        if parts.next().is_some() || !keys.iter().all(|key| is_key(key)) {
            return None;
        }
        Some(Self { service, at, keys })
    }
}

/// What a link may name: a bare source id, so no key can be read as a path or reach past the fields around it.
fn is_key(key: &str) -> bool {
    (1..=64).contains(&key.len())
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// Why a link did not play, as the notice key that says so.
pub(crate) struct Refusal {
    pub notice: &'static str,
    pub error: ApiError,
}

impl Refusal {
    fn new(notice: &'static str, error: ApiError) -> Self {
        Self { notice, error }
    }
}

/// Lookups in flight at once while resolving the queued songs.
const RESOLVING: usize = 6;

/// Switch to a source of the link's service, then play its songs from where the sharer is.
pub(crate) async fn play(
    link: &str,
    sources: &crate::sources::SourceService,
    catalog: &crate::catalog::CatalogService,
    session: &crate::session::SessionHandle,
) -> Result<(), Refusal> {
    let link = ListenLink::parse(link).ok_or_else(|| {
        Refusal::new(
            "listen_link_invalid",
            ApiError::invalid_input("not a Listen on Kopuz link"),
        )
    })?;
    let candidates: Vec<SourceInfo> = sources
        .sources()
        .await
        .map_err(|error| Refusal::new("listen_link_failed", error))?
        .into_iter()
        .filter(|source| source.service.id == link.service.id())
        .collect();
    let source = candidates
        .iter()
        .find(|source| source.active)
        .or_else(|| {
            candidates
                .iter()
                .find(|source| source.authenticated || source.anonymous)
        })
        .or_else(|| candidates.first())
        .ok_or_else(|| {
            Refusal::new(
                "listen_link_no_source",
                ApiError::not_found(format!("no {} source to play from", link.service.id())),
            )
        })?;
    if !source.active {
        sources
            .switch_source(&source.id)
            .await
            .map_err(|error| Refusal::new("listen_link_failed", error))?;
    }

    let found: Vec<Option<String>> = futures_util::stream::iter(link.keys)
        .map(|key: String| async move {
            match catalog.track(&key).await {
                Ok(Some(_)) => Some(key),
                Ok(None) => None,
                Err(error) => {
                    tracing::debug!(%error, key, "a shared song did not resolve");
                    None
                }
            }
        })
        .buffered(RESOLVING)
        .collect()
        .await;
    if found.first().is_none_or(Option::is_none) {
        return Err(Refusal::new(
            "listen_link_failed",
            ApiError::not_found("the shared song is not on this source"),
        ));
    }
    let keys: Vec<String> = found.into_iter().flatten().collect();
    let first = keys[0].clone();

    let mut events = session.subscribe();
    session
        .set_queue(SetQueueRequest {
            mode: QueueMode::Replace,
            context: QueueContext::Tracks { keys },
            start_index: Some(0),
            shuffle: None,
        })
        .await
        .map_err(|error| Refusal::new("listen_link_failed", error))?;
    if link.at.position_ms(unix_now()) > 0 {
        catch_up(session, &mut events, &first, link.at).await;
    }
    Ok(())
}

/// A stream still resolving drops a seek, so this waits for the song's first sound.
const SEEK_PATIENCE: Duration = Duration::from_secs(20);

/// Seek `key` to where the sharer is by now, once it is audibly playing.
async fn catch_up(
    session: &crate::session::SessionHandle,
    events: &mut broadcast::Receiver<ApiEvent>,
    key: &str,
    at: At,
) {
    let sounding = |state: &PlayerState| {
        state.track.as_ref().is_some_and(|track| track.key == key)
            && state.phase == Phase::Playing
            && matches!(state.intent, Intent::Committed { .. })
    };
    let started = tokio::time::timeout(SEEK_PATIENCE, async {
        while !sounding(&session.state()) {
            if let Err(broadcast::error::RecvError::Closed) = events.recv().await {
                return false;
            }
        }
        true
    })
    .await;
    if started != Ok(true) {
        tracing::debug!("the shared song never started; it plays from the top");
        return;
    }
    let position_ms = at.position_ms(unix_now());
    if let Err(error) = session
        .player_command(PlayerCommand::Seek { position_ms })
        .await
    {
        tracing::debug!(%error, "could not seek to where the sharer is");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_reads_back_as_written_from_every_form_it_arrives_in() {
        let url = ListenLink::url(
            MusicService::YtMusic,
            At::Since(1_731_000_000),
            ["dQw4w9WgXcQ", "kJQP7kiw5Fk"],
        )
        .expect("url");
        assert_eq!(
            url,
            "https://kopuz.moe/j#1/ytmusic/s1731000000/dQw4w9WgXcQ,kJQP7kiw5Fk"
        );
        let expected = ListenLink {
            service: MusicService::YtMusic,
            at: At::Since(1_731_000_000),
            keys: vec!["dQw4w9WgXcQ".into(), "kJQP7kiw5Fk".into()],
        };
        let payload = url.strip_prefix(SITE).expect("site prefix");
        assert_eq!(ListenLink::parse(&url), Some(expected.clone()));
        assert_eq!(
            ListenLink::parse(&format!("kopuz://j/{payload}/")),
            Some(expected.clone())
        );
        assert_eq!(ListenLink::parse(payload), Some(expected));
    }

    #[test]
    fn the_queue_stops_where_discord_would_refuse_the_url() {
        let keys: Vec<String> = (0..100).map(|n| format!("video{n:06}")).collect();
        let url = ListenLink::url(
            MusicService::YtMusic,
            At::Paused(42),
            keys.iter().map(String::as_str),
        )
        .expect("url");
        assert!(url.len() <= MAX_URL);
        let link = ListenLink::parse(&url).expect("parse");
        assert!(link.keys.len() > 30, "it packs as many as fit");
        assert_eq!(link.keys[..], keys[..link.keys.len()]);
    }

    #[test]
    fn a_key_that_could_be_a_path_or_split_the_fields_is_never_carried() {
        assert_eq!(
            ListenLink::url(MusicService::YtMusic, At::Paused(0), ["/home/a.flac"]),
            None
        );
        let url = ListenLink::url(
            MusicService::YtMusic,
            At::Paused(0),
            ["ok", "a,b", "../x", "fine"],
        )
        .expect("url");
        assert_eq!(
            ListenLink::parse(&url).map(|link| link.keys),
            Some(vec!["ok".to_string(), "fine".to_string()])
        );
        assert_eq!(ListenLink::parse("1/ytmusic/p0/..%2Fx"), None);
        assert_eq!(ListenLink::parse("1/ytmusic/p0/a/b"), None);
        assert_eq!(ListenLink::parse("1/ytmusic/p0/"), None);
        assert_eq!(ListenLink::parse("2/ytmusic/p0/abc"), None);
        assert_eq!(ListenLink::parse("1/nope/p0/abc"), None);
    }

    #[test]
    fn the_position_follows_the_clock_while_playing_and_holds_while_paused() {
        assert_eq!(At::Since(1_000).position_ms(1_090), 90_000);
        assert_eq!(At::Paused(30).position_ms(9_999_999), 30_000);
        assert_eq!(
            At::Since(2_000).position_ms(1_000),
            0,
            "a clock ahead of ours"
        );
        assert_eq!(
            At::Since(0).position_ms(STALE_SECS + 1),
            0,
            "a day-old link"
        );
    }
}
