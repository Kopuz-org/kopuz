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

#[cfg(test)]
mod tests {
    use super::*;

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
