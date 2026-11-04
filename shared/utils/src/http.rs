//! Every HTTP client starts here, because reqwest prefers native-tls whenever any crate compiles it in.
#![allow(clippy::disallowed_methods)]

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

static OFFLINE: AtomicBool = AtomicBool::new(false);

/// Offline mode: every client built here stops reaching past this machine and its network.
pub fn set_offline(on: bool) {
    OFFLINE.store(on, Ordering::SeqCst);
}

pub fn offline() -> bool {
    OFFLINE.load(Ordering::SeqCst)
}

/// A client builder on rustls, kept off the internet in offline mode.
pub fn builder() -> reqwest::ClientBuilder {
    internet_builder()
        .dns_resolver(Arc::new(OfflineGate))
        .redirect(redirects())
}

/// A client builder on rustls that reaches the internet in offline mode too, for the connectivity checks that tell when it is back.
pub fn internet_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().tls_backend_rustls()
}

/// A rustls client with reqwest's defaults; panics where `reqwest::Client::new` would.
pub fn client() -> reqwest::Client {
    match builder().build() {
        Ok(client) => client,
        Err(error) => panic!("the TLS backend failed to initialize: {error}"),
    }
}

/// A blocking client builder on rustls.
pub fn blocking_builder() -> reqwest::blocking::ClientBuilder {
    reqwest::blocking::Client::builder()
        .tls_backend_rustls()
        .dns_resolver(Arc::new(OfflineGate))
        .redirect(redirects())
}

/// A client builder on the platform's TLS (OpenSSL on Linux), for servers that reset rustls's handshake.
#[cfg(not(target_os = "android"))]
pub fn native_tls_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .tls_backend_native()
        .dns_resolver(Arc::new(OfflineGate))
        .redirect(redirects())
}

/// The hosts a URL someone else named may lead to. The daemon fetches a
/// plugin's stream outside the plugin's sandbox, so it holds the fetch to what
/// the sandbox would have allowed.
pub trait Reach: Send + Sync {
    /// Whether `host` may be asked.
    fn allows_host(&self, host: &str) -> bool;
    /// Whether `host` may lead to this machine or its network.
    fn may_be_local(&self, host: &str) -> bool;
}

/// Enough for the streams open or queued at once.
const MAX_GUARDED: usize = 256;

static GUARDED: Mutex<VecDeque<(String, Arc<dyn Reach>)>> = Mutex::new(VecDeque::new());

fn guarded() -> std::sync::MutexGuard<'static, VecDeque<(String, Arc<dyn Reach>)>> {
    GUARDED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Hold every client built here to `reach` when it fetches `url`: each
/// redirect from it, and every name those lead to.
pub fn guard(url: &str, reach: Arc<dyn Reach>) {
    let mut guarded = guarded();
    guarded.retain(|(held, _)| held != url);
    if guarded.len() >= MAX_GUARDED {
        guarded.pop_front();
    }
    guarded.push_back((url.to_string(), reach));
}

fn guard_of(url: &str) -> Option<Arc<dyn Reach>> {
    guarded()
        .iter()
        .find(|(held, _)| held == url)
        .map(|(_, reach)| Arc::clone(reach))
}

/// Whether `reach` lets `url` be fetched at all.
fn within(reach: &dyn Reach, url: &reqwest::Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| reach.allows_host(host))
}

/// Whether `next`, found while fetching `from`, may be fetched too; if so it
/// is held to the same reach. Unguarded URLs lead anywhere.
pub fn follows(from: &str, next: &str) -> bool {
    let Some(reach) = guard_of(from) else {
        return true;
    };
    let allowed = reqwest::Url::parse(next).is_ok_and(|url| within(reach.as_ref(), &url));
    if allowed {
        guard(next, reach);
    }
    allowed
}

/// Reqwest's limit, and each hop from a guarded URL held to its reach.
fn redirects() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error("too many redirects");
        }
        let Some(from) = attempt.previous().first() else {
            return attempt.follow();
        };
        match guard_of(from.as_str()) {
            Some(reach) if !within(reach.as_ref(), attempt.url()) => {
                attempt.error("redirected to a host its plugin may not reach")
            }
            Some(reach) => {
                guard(attempt.url().as_str(), reach);
                attempt.follow()
            }
            None => attempt.follow(),
        }
    })
}

/// Whether a guarded fetch could have led to `host`, which then may not lead
/// to this machine or its network unless every reach naming it allows that.
fn kept_public(host: &str) -> bool {
    let guarded = guarded();
    let naming: Vec<_> = guarded
        .iter()
        .filter(|(_, reach)| reach.allows_host(host))
        .collect();
    !naming.is_empty() && naming.iter().any(|(_, reach)| !reach.may_be_local(host))
}

/// Whether `host` is this machine or on its network: loopback, private and link-local addresses, `localhost` and `.local` names.
pub fn local_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    match host.parse::<IpAddr>() {
        Ok(ip) => local_ip(ip),
        Err(_) => {
            let name = host.trim_end_matches('.').to_ascii_lowercase();
            name == "localhost" || name.ends_with(".local")
        }
    }
}

pub fn local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => local_v4(ip),
        IpAddr::V6(ip) => local_v6(ip),
    }
}

fn local_v4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

fn local_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    let unique_local = first & 0xfe00 == 0xfc00;
    let link_local = first & 0xffc0 == 0xfe80;
    ip.is_loopback() || unique_local || link_local || ip.to_ipv4_mapped().is_some_and(local_v4)
}

/// The lookup every guarded client resolves through; a host named by its address skips lookups, so only a name is gated.
struct OfflineGate;

impl reqwest::dns::Resolve for OfflineGate {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            if offline() && !local_host(&host) {
                tracing::debug!(host, "offline mode kept a request off the internet");
                return Err(Box::new(KeptOffline) as Box<dyn std::error::Error + Send + Sync>);
            }
            let public_only = kept_public(&host);
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|addr| !public_only || !(addr.ip().is_unspecified() || local_ip(addr.ip())))
                .collect();
            if addrs.is_empty() && public_only {
                return Err(Box::new(KeptPublic) as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

#[derive(Debug)]
struct KeptOffline;

impl std::fmt::Display for KeptOffline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("offline mode keeps requests off the internet")
    }
}

impl std::error::Error for KeptOffline {}

#[derive(Debug)]
struct KeptPublic;

impl std::fmt::Display for KeptPublic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a plugin's host may not lead to this machine or its network")
    }
}

impl std::error::Error for KeptPublic {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plugin allowed `allowed`, whose configured server is `server`.
    struct Only {
        allowed: &'static str,
        server: Option<&'static str>,
    }

    impl Reach for Only {
        fn allows_host(&self, host: &str) -> bool {
            host == self.allowed
        }

        fn may_be_local(&self, host: &str) -> bool {
            self.server == Some(host)
        }
    }

    /// Answers every request with a redirect to `location`, counting them.
    fn redirecting(host: &str, location: String) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind((host, 0)).expect("bind");
        let address = listener.local_addr().expect("address");
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = Arc::clone(&hits);
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                seen.fetch_add(1, Ordering::SeqCst);
                let _ = stream.read(&mut [0u8; 2048]);
                let reply = format!(
                    "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (format!("http://{address}"), hits)
    }

    #[tokio::test]
    async fn a_guarded_url_never_redirects_off_its_hosts() {
        let (elsewhere, elsewhere_hits) = redirecting("127.0.0.2", "/".into());
        let (start, _) = redirecting("127.0.0.1", format!("{elsewhere}/x"));
        guard(
            &format!("{start}/stream"),
            Arc::new(Only {
                allowed: "127.0.0.1",
                server: None,
            }),
        );
        let refused = client().get(format!("{start}/stream")).send().await;
        assert!(refused.is_err(), "{refused:?}");
        assert_eq!(elsewhere_hits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn what_a_guarded_url_lists_is_held_to_its_hosts() {
        let reach = Arc::new(Only {
            allowed: "cdn.test",
            server: None,
        });
        guard("https://cdn.test/list.m3u8", reach);
        assert!(follows(
            "https://cdn.test/list.m3u8",
            "https://cdn.test/0.m4s"
        ));
        assert!(!follows(
            "https://cdn.test/list.m3u8",
            "http://192.168.1.1/0.m4s"
        ));
        assert!(!follows("https://cdn.test/list.m3u8", "file:///etc/passwd"));
        assert!(follows(
            "https://other.test/list.m3u8",
            "http://192.168.1.1/"
        ));
    }

    #[test]
    fn a_plugins_name_may_lead_here_only_for_its_server() {
        guard(
            "https://stream.plugin.test/a",
            Arc::new(Only {
                allowed: "stream.plugin.test",
                server: None,
            }),
        );
        guard(
            "http://nas.plugin.test/a",
            Arc::new(Only {
                allowed: "nas.plugin.test",
                server: Some("nas.plugin.test"),
            }),
        );
        assert!(kept_public("stream.plugin.test"));
        assert!(!kept_public("nas.plugin.test"));
        assert!(!kept_public("unguarded.plugin.test"));
    }

    #[test]
    fn this_machine_and_its_network_are_local() {
        for host in [
            "localhost",
            "LOCALHOST.",
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.20",
            "169.254.10.1",
            "[::1]",
            "[fd12:3456:789a::1]",
            "[fe80::1]",
            "[::ffff:192.168.1.2]",
            "nas.local",
        ] {
            assert!(local_host(host), "{host} is local");
        }
        for host in [
            "music.youtube.com",
            "8.8.8.8",
            "172.32.0.1",
            "[2001:db8::1]",
            "local.example.com",
            "notlocal",
            "",
        ] {
            assert!(!local_host(host), "{host} is not local");
        }
    }

    #[tokio::test]
    async fn offline_mode_refuses_names_past_this_network_and_resolves_the_rest() {
        use reqwest::dns::Resolve;
        let resolve = |host: &str| OfflineGate.resolve(host.parse().expect("a name"));
        set_offline(true);
        let refused = resolve("music.youtube.com").await;
        let local = resolve("localhost").await;
        set_offline(false);
        assert!(refused.is_err(), "kept off the internet");
        assert!(local.is_ok_and(|mut addrs| addrs.next().is_some()));
    }
}
