//! Whether the internet is down or only one server is, asked only after a source that needs the internet stops answering.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use tokio::sync::OnceCell;
use tokio::time::Instant;

/// A check started by a failure waits this long after the last one began; a check already running is joined instead.
pub const CHECK_SPACING: Duration = Duration::from_secs(30);
/// While the internet is down, someone using the app asks again at most this often.
pub const NUDGE_SPACING: Duration = Duration::from_secs(5);
/// How long one address may take to answer.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(3);
/// While the internet is down, how long to wait before each recheck; the last one repeats.
pub const RECHECK_BACKOFF: [Duration; 3] = [
    Duration::from_secs(15),
    Duration::from_secs(30),
    Duration::from_secs(60),
];

/// What a connectivity check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Up,
    Down,
    /// There was nothing to ask, so nothing is known.
    Unchecked,
}

/// Up on the first address that answers, down once every one has failed, unchecked when there is none to ask.
pub async fn check<F, Fut>(urls: &[String], probe: F) -> Verdict
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = bool>,
{
    if urls.is_empty() {
        return Verdict::Unchecked;
    }
    let mut answers: FuturesUnordered<Fut> = urls.iter().cloned().map(probe).collect();
    while let Some(answered) = answers.next().await {
        if answered {
            return Verdict::Up;
        }
    }
    Verdict::Down
}

/// What a captive-portal check answers when the internet is there: 204, or 200 with the body `success`; a portal's redirect or login page is neither.
pub fn answered(status: u16, body: &str) -> bool {
    status == 204 || (status == 200 && body.trim() == "success")
}

/// A client that neither follows a redirect nor waits long, since a portal answers with one and a dead link with silence; offline mode lets it through.
pub fn check_client() -> Option<reqwest::Client> {
    utils::http::internet_builder()
        .timeout(CHECK_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| tracing::warn!(%error, "the connectivity check client failed to start"))
        .ok()
}

pub async fn probe(client: &reqwest::Client, url: String) -> bool {
    let response = match client.get(&url).send().await {
        Ok(response) => response,
        Err(error) => {
            tracing::debug!(url, error = %error.without_url(), "a connectivity check failed");
            return false;
        }
    };
    let status = response.status().as_u16();
    let body = match status {
        200 => response.text().await.unwrap_or_default(),
        _ => String::new(),
    };
    answered(status, &body)
}

/// Whether a source keeps working with the internet down: a folder library, or a server on this machine or its network.
pub fn is_local(source: &config::Source, server_url: Option<&str>) -> bool {
    match source {
        config::Source::LocalLibrary(_) => true,
        config::Source::Server(_) => server_url
            .and_then(|url| reqwest::Url::parse(url.trim()).ok())
            .and_then(|url| url.host_str().map(utils::http::local_host))
            .unwrap_or(false),
    }
}

/// Whether the active source is across the internet, so offline mode keeps its listings and playback to the copies here.
pub fn needs_internet(sources: &server::sources::Sources) -> bool {
    let url = sources.server.as_ref().map(|server| server.url.as_str());
    !is_local(&sources.active, url)
}

/// Whether the active source's songs go into the offline cache: across the internet, and played by the engine rather than an integration; never a folder library, an SMB share or a server on this network.
pub fn cacheable(sources: &server::sources::Sources) -> bool {
    let service = sources.server.as_ref().map(|server| server.service);
    needs_internet(sources)
        && !matches!(
            service,
            Some(config::MusicService::Smb | config::MusicService::Spotify)
        )
}

/// Where to look when the internet is down, best first: the default folder library, the other folder libraries, then the servers on this network.
pub fn local_candidates(
    settings: &config::Settings,
    servers: &[config::SavedServer],
) -> Vec<config::Source> {
    let folders = &settings.library.local_sources;
    let default = folders
        .iter()
        .filter(|saved| saved.id == config::DEFAULT_LOCAL_ID);
    let others = folders
        .iter()
        .filter(|saved| saved.id != config::DEFAULT_LOCAL_ID);
    let lan = servers
        .iter()
        .map(|server| (config::Source::Server(server.id.clone()), server))
        .filter(|(source, server)| is_local(source, Some(&server.url)))
        .map(|(source, _)| source);
    default
        .chain(others)
        .map(|saved| config::Source::from_column(&saved.id))
        .chain(lan)
        .collect()
}

/// The wait before the `attempt`th recheck, counting from zero.
pub fn recheck_delay(attempt: usize) -> Duration {
    RECHECK_BACKOFF[attempt.min(RECHECK_BACKOFF.len() - 1)]
}

type Running = Arc<OnceCell<Verdict>>;

/// One check at a time, and one started by a failure at most every [`CHECK_SPACING`].
#[derive(Default)]
pub struct Checker {
    last: Mutex<Option<(Instant, Running)>>,
}

impl Checker {
    /// The verdict of the check running now, of one begun within `spacing`, or of `start` run anew.
    pub async fn run<F, Fut>(&self, spacing: Duration, start: F) -> Verdict
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Verdict>,
    {
        let running = {
            let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
            match &*last {
                Some((began, running)) if !running.initialized() || began.elapsed() < spacing => {
                    running.clone()
                }
                _ => {
                    let running = Running::default();
                    *last = Some((Instant::now(), running.clone()));
                    running
                }
            }
        };
        *running.get_or_init(start).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn server(url: &str) -> bool {
        is_local(&config::Source::Server("s".into()), Some(url))
    }

    #[test]
    fn a_folder_library_and_servers_on_this_network_are_local() {
        assert!(is_local(&config::Source::default(), None));
        for url in [
            "http://localhost:4533",
            "http://LOCALHOST./",
            "http://127.0.0.1:8096",
            "http://127.8.9.10",
            "http://[::1]:8096",
            "http://10.0.0.5",
            "http://172.16.0.1",
            "http://172.31.255.254",
            "https://192.168.1.20:443/jellyfin",
            "http://169.254.10.1",
            "http://[fd12:3456:789a::1]",
            "http://[fc00::1]",
            "http://[fe80::1]",
            "http://[::ffff:192.168.1.2]",
            "http://nas.local:4533",
            "smb://nas.local/music",
            "smb://192.168.1.4/share",
        ] {
            assert!(server(url), "{url} is local");
        }
    }

    #[test]
    fn anything_across_the_internet_or_unreadable_is_not() {
        for url in [
            "https://music.youtube.com",
            "https://listener.clippsly.app",
            "https://jelly.example.com",
            "http://172.32.0.1",
            "http://172.15.255.255",
            "http://11.0.0.1",
            "http://8.8.8.8",
            "http://[2001:db8::1]",
            "http://local.example.com",
            "http://notlocal",
            "smb://nas.example.org/music",
            "a-spotify-client-id",
            "",
        ] {
            assert!(!server(url), "{url} is not local");
        }
        assert!(!is_local(&config::Source::Server("s".into()), None));
    }

    fn active(url: &str, service: config::MusicService) -> server::sources::Sources {
        server::sources::Sources {
            active: config::Source::Server("s".into()),
            server: Some(config::MusicServer::new_with_service(
                "S".into(),
                url.into(),
                service,
            )),
            ..Default::default()
        }
    }

    #[test]
    fn only_songs_from_across_the_internet_are_cached() {
        use config::MusicService::*;
        assert!(cacheable(&active("https://music.youtube.com", YtMusic)));
        assert!(cacheable(&active("https://jelly.example.com", Jellyfin)));
        assert!(cacheable(&active(
            "https://listener.clippsly.app",
            Clippsly
        )));
        assert!(
            !cacheable(&server::sources::Sources::default()),
            "a folder library"
        );
        assert!(
            !cacheable(&active("http://192.168.1.9:8096", Jellyfin)),
            "on this network"
        );
        assert!(!cacheable(&active("smb://nas.example.org/music", Smb)));
        assert!(
            !cacheable(&active("a-spotify-client-id", Spotify)),
            "an integration plays it"
        );
        assert!(needs_internet(&active("a-spotify-client-id", Spotify)));
        assert!(!needs_internet(&server::sources::Sources::default()));
    }

    fn urls(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("http://check/{i}")).collect()
    }

    #[tokio::test]
    async fn any_answer_means_up_and_all_failing_means_down() {
        let answers = |good: Option<usize>| {
            move |url: String| async move { good.is_some_and(|i| url.ends_with(&format!("/{i}"))) }
        };
        assert_eq!(check(&urls(2), answers(Some(1))).await, Verdict::Up);
        assert_eq!(check(&urls(2), answers(Some(0))).await, Verdict::Up);
        assert_eq!(check(&urls(2), answers(None)).await, Verdict::Down);
    }

    #[tokio::test]
    async fn no_address_means_no_check() {
        let asked = AtomicUsize::new(0);
        let verdict = check(&[], |_| {
            asked.fetch_add(1, Ordering::SeqCst);
            async { false }
        })
        .await;
        assert_eq!(verdict, Verdict::Unchecked);
        assert_eq!(asked.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_portal_check_counts_only_its_own_answer() {
        assert!(answered(204, ""));
        assert!(answered(200, "success\n"));
        assert!(!answered(200, "<html>Sign in to the hotel wifi</html>"));
        assert!(!answered(302, ""));
        assert!(!answered(503, "success"));
    }

    async fn counted(
        checker: &Checker,
        spacing: Duration,
        runs: &AtomicUsize,
        verdict: Verdict,
    ) -> Verdict {
        checker
            .run(spacing, || async {
                runs.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
                verdict
            })
            .await
    }

    #[tokio::test(start_paused = true)]
    async fn a_check_runs_at_most_once_per_spacing_and_is_shared_while_it_runs() {
        let checker = Checker::default();
        let runs = AtomicUsize::new(0);

        let spaced = CHECK_SPACING;
        let (a, b) = tokio::join!(
            counted(&checker, spaced, &runs, Verdict::Down),
            counted(&checker, spaced, &runs, Verdict::Up)
        );
        assert_eq!((a, b), (Verdict::Down, Verdict::Down), "one shared check");
        assert_eq!(runs.load(Ordering::SeqCst), 1);

        tokio::time::advance(CHECK_SPACING - Duration::from_secs(1)).await;
        assert_eq!(
            counted(&checker, spaced, &runs, Verdict::Up).await,
            Verdict::Down
        );
        assert_eq!(runs.load(Ordering::SeqCst), 1, "too soon for another");

        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(
            counted(&checker, spaced, &runs, Verdict::Up).await,
            Verdict::Up
        );
        assert_eq!(runs.load(Ordering::SeqCst), 2);

        assert_eq!(
            counted(&checker, Duration::ZERO, &runs, Verdict::Down).await,
            Verdict::Down
        );
        assert_eq!(runs.load(Ordering::SeqCst), 3, "a recheck does not wait");

        assert_eq!(
            counted(&checker, NUDGE_SPACING, &runs, Verdict::Up).await,
            Verdict::Down,
            "someone acting right after a check joins it"
        );
        tokio::time::advance(NUDGE_SPACING).await;
        assert_eq!(
            counted(&checker, NUDGE_SPACING, &runs, Verdict::Up).await,
            Verdict::Up
        );
        assert_eq!(
            runs.load(Ordering::SeqCst),
            4,
            "and asks again a few seconds on"
        );
    }

    #[test]
    fn rechecks_back_off_and_settle() {
        let waits: Vec<u64> = (0..5).map(|n| recheck_delay(n).as_secs()).collect();
        assert_eq!(waits, [15, 30, 60, 60, 60]);
    }

    fn folders(id: &str, dirs: &[&str]) -> config::SavedLocalSource {
        config::SavedLocalSource {
            id: id.into(),
            name: id.into(),
            directories: dirs.iter().map(std::path::PathBuf::from).collect(),
        }
    }

    fn saved(id: &str, url: &str) -> config::SavedServer {
        let mut server =
            config::SavedServer::new(id.into(), url.into(), config::MusicService::Jellyfin);
        server.id = id.into();
        server
    }

    #[test]
    fn the_default_library_leads_then_the_other_folders_then_the_lan_servers() {
        let mut settings = config::Settings::default();
        settings.library.local_sources = vec![
            folders("local:b", &["/b"]),
            folders(config::DEFAULT_LOCAL_ID, &["/music"]),
        ];
        let servers = [
            saved("yt", "https://music.youtube.com"),
            saved("nas", "http://192.168.1.9:8096"),
        ];

        assert_eq!(
            local_candidates(&settings, &servers),
            [
                config::Source::from_column(config::DEFAULT_LOCAL_ID),
                config::Source::from_column("local:b"),
                config::Source::Server("nas".into()),
            ]
        );
        settings.library.local_sources.clear();
        assert!(local_candidates(&settings, &servers[..1]).is_empty());
    }
}
