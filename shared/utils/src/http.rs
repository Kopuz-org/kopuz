//! Every HTTP client starts here, because reqwest prefers native-tls whenever any crate compiles it in.
#![allow(clippy::disallowed_methods)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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
    internet_builder().dns_resolver(Arc::new(OfflineGate))
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
}

/// A client builder on the platform's TLS (OpenSSL on Linux), for servers that reset rustls's handshake.
#[cfg(not(target_os = "android"))]
pub fn native_tls_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .tls_backend_native()
        .dns_resolver(Arc::new(OfflineGate))
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

/// A plugin's [`Reach`], carried with each stream it hands out.
pub type Guard = Arc<dyn Reach>;

/// Whether `reach` lets `url` be fetched. A host written as an address is
/// held to the local-network rule here, since no lookup sees it.
pub fn within(reach: &dyn Reach, url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let address = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
        .ok();
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && reach.allows_host(host)
        && address.is_none_or(|ip| !local_or_unspecified(ip) || reach.may_be_local(host))
}

fn local_or_unspecified(ip: IpAddr) -> bool {
    ip.is_unspecified() || local_ip(ip)
}

/// [`builder`], or with `guard` one whose every redirect and every name it
/// looks up is held to that plugin's reach.
pub fn guarded_builder(guard: Option<Guard>) -> reqwest::ClientBuilder {
    match guard {
        None => builder(),
        Some(guard) => internet_builder()
            .dns_resolver(Arc::new(GuardedGate(Arc::clone(&guard))))
            .redirect(held_to(guard)),
    }
}

/// [`blocking_builder`], held to `guard` as [`guarded_builder`] is.
pub fn guarded_blocking_builder(guard: Option<Guard>) -> reqwest::blocking::ClientBuilder {
    match guard {
        None => blocking_builder(),
        Some(guard) => reqwest::blocking::Client::builder()
            .tls_backend_rustls()
            .dns_resolver(Arc::new(GuardedGate(Arc::clone(&guard))))
            .redirect(held_to(guard)),
    }
}

fn held_to(guard: Guard) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 10 {
            attempt.error("too many redirects")
        } else if within(guard.as_ref(), attempt.url()) {
            attempt.follow()
        } else {
            attempt.error("redirected to a host its plugin may not reach")
        }
    })
}

/// [`OfflineGate`], and a name may lead to this machine or its network only
/// where the plugin may reach it there: its configured server.
struct GuardedGate(Guard);

impl reqwest::dns::Resolve for GuardedGate {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        let local_allowed = self.0.may_be_local(&host);
        Box::pin(async move {
            if offline() && !local_host(&host) {
                return Err(Box::new(KeptOffline) as Box<dyn std::error::Error + Send + Sync>);
            }
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|addr| local_allowed || !local_or_unspecified(addr.ip()))
                .collect();
            if addrs.is_empty() {
                return Err(Box::new(KeptPublic) as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
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
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
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

    fn only(allowed: &'static str, server: Option<&'static str>) -> Guard {
        Arc::new(Only { allowed, server })
    }

    /// Skipped where 127.0.0.2 is not a loopback address, as on macOS by default.
    #[tokio::test]
    async fn a_guarded_fetch_never_redirects_off_its_hosts() {
        if std::net::TcpListener::bind(("127.0.0.2", 0)).is_err() {
            return;
        }
        let (elsewhere, elsewhere_hits) = redirecting("127.0.0.2", "/".into());
        let (start, _) = redirecting("127.0.0.1", format!("{elsewhere}/x"));
        let client = guarded_builder(Some(only("127.0.0.1", Some("127.0.0.1"))))
            .build()
            .expect("client");
        let refused = client.get(format!("{start}/stream")).send().await;
        assert!(refused.is_err(), "{refused:?}");
        assert_eq!(elsewhere_hits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn what_a_plugin_may_fetch() {
        let reach = only("cdn.test", None);
        let url = |raw: &str| reqwest::Url::parse(raw).expect("a URL");
        assert!(within(reach.as_ref(), &url("https://cdn.test:443/0.m4s")));
        assert!(!within(reach.as_ref(), &url("https://other.test/0.m4s")));
        assert!(!within(reach.as_ref(), &url("https://user:pw@cdn.test/")));
        assert!(!within(reach.as_ref(), &url("file:///etc/passwd")));

        let listed_address = only("192.168.1.1", None);
        assert!(
            !within(listed_address.as_ref(), &url("http://192.168.1.1/")),
            "an address on this network is refused unless it is the server"
        );
        let server = only("192.168.1.1", Some("192.168.1.1"));
        assert!(within(server.as_ref(), &url("http://192.168.1.1/")));
    }

    #[tokio::test]
    async fn a_plugins_name_may_lead_here_only_for_its_server() {
        use reqwest::dns::Resolve;
        let name = || "localhost".parse().expect("a name");
        let refused = GuardedGate(only("localhost", None)).resolve(name()).await;
        assert!(refused.is_err(), "a plugin's name may not lead here");
        let server = GuardedGate(only("localhost", Some("localhost")))
            .resolve(name())
            .await;
        assert!(server.is_ok_and(|mut addrs| addrs.next().is_some()));
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
