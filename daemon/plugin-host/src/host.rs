//! What a plugin can reach: the host side of its imports.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use wasmtime::component::ResourceTable;
use wasmtime::{StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxView, WasiView};

use crate::bindings::kopuz::plugin::{browser, decryption, details, http, log, tcp};
use crate::manifest::Permissions;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

const MAX_BODY_BYTES: usize = 512 * 1024 * 1024;

/// Room for the largest upload a plugin takes (Clippsly's 500 MiB) twice over:
/// wrapping it in a request body can move it to a bigger allocation, and the
/// old one is still live while it is copied.
const MAX_MEMORY_BYTES: usize = 1280 * 1024 * 1024;

const MAX_CONNECTIONS: usize = 8;

const MAX_READ_BYTES: usize = 64 * 1024;

/// How long the person has to finish a sign-in in their browser.
const SIGN_IN_LIMIT: Duration = Duration::from_secs(5 * 60);

/// Opens a URL in the person's browser.
pub type Opener = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// How the host reads a stream its plugin said is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decryption {
    pub key: [u8; 16],
    pub iv: [u8; 16],
    /// Decrypted bytes at the start of the file that are not audio.
    pub skip: u32,
}

pub(crate) struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    pub(crate) limits: StoreLimits,
    plugin_id: Arc<str>,
    permissions: Arc<Permissions>,
    /// What `$server` names.
    server_host: Option<String>,
    /// Nothing when it failed to build, since a client without the gate and
    /// redirect policy is no substitute.
    client: Option<reqwest::Client>,
    /// Bit depths the running call reported, by item id; the session takes them once it returns.
    pub(crate) bit_depths: HashMap<String, u8>,
    /// Decryptions the running call reported, by stream URL; taken like `bit_depths`.
    pub(crate) decryptions: HashMap<String, Decryption>,
    connections: HashMap<u32, tokio::net::TcpStream>,
    next_connection: u32,
    /// Set only for `login`.
    opener: Option<Opener>,
}

impl State {
    pub(crate) fn new(
        plugin_id: Arc<str>,
        permissions: Arc<Permissions>,
        server_url: &str,
        folders: &[String],
    ) -> Self {
        let server_host = host_of(server_url);
        let client = client_for(&permissions, server_host.clone());
        // Linked so `std` runs, with no sockets or environment, and no files
        // beyond the source's folders for a plugin allowed to read them.
        let mut wasi = WasiCtx::builder();
        if permissions.reads_folders() {
            for folder in folders {
                let guest = crate::folders::guest_path(folder);
                if let Err(error) = wasi.preopened_dir(folder, &guest, FsPerms::ReadOnly) {
                    tracing::warn!(plugin = %plugin_id, %folder, error = %format!("{error:#}"), "a source folder could not be opened for the plugin");
                }
            }
        }
        let wasi = wasi.build();
        Self {
            wasi,
            table: ResourceTable::new(),
            limits: StoreLimitsBuilder::new()
                .memory_size(MAX_MEMORY_BYTES)
                .build(),
            plugin_id,
            permissions,
            server_host,
            client,
            bit_depths: HashMap::new(),
            decryptions: HashMap::new(),
            connections: HashMap::new(),
            next_connection: 0,
            opener: None,
        }
    }

    pub(crate) fn allow_sign_in(&mut self, opener: Opener) {
        self.opener = Some(opener);
    }

    fn reaches(&self, host: &str) -> bool {
        self.permissions.allows(host, self.server_host.as_deref())
            && !local_literal(host, self.server_host.as_deref())
    }
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// Redirects are held to the same allowlist, so an allowed server cannot
/// bounce a plugin elsewhere.
fn client_for(
    permissions: &Arc<Permissions>,
    server_host: Option<String>,
) -> Option<reqwest::Client> {
    let permissions = Arc::clone(permissions);
    let gate = Gate {
        server_host: server_host.clone(),
    };
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        let allowed = attempt.url().host_str().is_some_and(|host| {
            permissions.allows(host, server_host.as_deref())
                && !local_literal(host, server_host.as_deref())
        });
        if !allowed {
            attempt.error("redirected to a host the plugin's manifest does not allow")
        } else if attempt.previous().len() >= 10 {
            attempt.error("too many redirects")
        } else {
            attempt.follow()
        }
    });
    // Not `utils::http::builder`, whose resolver this one stands in for.
    utils::http::internet_builder()
        .dns_resolver(Arc::new(gate))
        .redirect(policy)
        .build()
        .inspect_err(|error| tracing::warn!(%error, "a plugin's HTTP client failed to build"))
        .ok()
}

/// A host written as an address skips the lookup [`Gate`] filters, so it is
/// held to the same rule here: on this machine or its network, only the
/// configured server.
fn local_literal(host: &str, server_host: Option<&str>) -> bool {
    let normal = host.trim_end_matches('.').to_ascii_lowercase();
    normal
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
        .is_ok_and(local)
        && server_host != Some(normal.as_str())
}

/// Allowed names are checked before the lookup; this checks where they lead.
/// Only the configured server may be on this machine or its network, so a name
/// a plugin may reach cannot be pointed at the router or a local service.
struct Gate {
    server_host: Option<String>,
}

impl reqwest::dns::Resolve for Gate {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().trim_end_matches('.').to_ascii_lowercase();
        let local_allowed = self.server_host.as_deref() == Some(host.as_str());
        Box::pin(async move {
            let addrs = lookup(&host, 0, local_allowed).await?;
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Where `host` leads, without this machine and its network unless it is the
/// configured server.
async fn lookup(
    host: &str,
    port: u16,
    local_allowed: bool,
) -> Result<Vec<std::net::SocketAddr>, Box<dyn std::error::Error + Send + Sync>> {
    if utils::http::offline() && !utils::http::local_host(host) {
        return Err(gate_error("offline mode keeps requests off the internet"));
    }
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port))
        .await?
        .filter(|addr| local_allowed || !local(addr.ip()))
        .collect();
    if addrs.is_empty() {
        return Err(gate_error(
            "the host leads only to this machine or its network",
        ));
    }
    Ok(addrs)
}

fn local(ip: std::net::IpAddr) -> bool {
    ip.is_unspecified() || utils::http::local_ip(ip)
}

fn gate_error(message: &str) -> Box<dyn std::error::Error + Send + Sync> {
    message.into()
}

fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()?
        .host_str()
        .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
}

impl http::Host for State {
    async fn send(&mut self, request: http::Request) -> Result<http::Response, http::HttpError> {
        let url = reqwest::Url::parse(&request.url)
            .map_err(|error| http::HttpError::Other(format!("invalid URL: {error}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(http::HttpError::Denied(format!(
                "the {} scheme is not allowed",
                url.scheme()
            )));
        }
        let host = url.host_str().unwrap_or_default().to_string();
        if !self.reaches(&host) {
            tracing::warn!(plugin = %self.plugin_id, %host, "plugin request to an undeclared host refused");
            return Err(http::HttpError::Denied(format!(
                "the plugin's manifest does not allow {host}"
            )));
        }
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| http::HttpError::Other(format!("invalid method {}", request.method)))?;
        let timeout = request
            .timeout_ms
            .map_or(DEFAULT_TIMEOUT, |ms| Duration::from_millis(ms.into()));

        let client = self
            .client
            .as_ref()
            .ok_or_else(|| http::HttpError::Other("the plugin has no HTTP client".into()))?;
        let mut builder = client.request(method, url).timeout(timeout);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        let mut response = builder.send().await.map_err(|error| {
            tracing::debug!(plugin = %self.plugin_id, %host, error = %format!("{error:#}"), "plugin request failed");
            request_error(error)
        })?;

        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                Some((name.as_str().to_string(), value.to_str().ok()?.to_string()))
            })
            .collect();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            if body.len() + chunk.len() > MAX_BODY_BYTES {
                return Err(http::HttpError::Other(
                    "the response body is too large".to_string(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(http::Response {
            status,
            headers,
            body,
        })
    }
}

fn request_error(error: reqwest::Error) -> http::HttpError {
    if error.is_timeout() {
        http::HttpError::Timeout
    } else if error.is_connect() {
        http::HttpError::Connect(error.without_url().to_string())
    } else {
        http::HttpError::Other(error.without_url().to_string())
    }
}

impl log::Host for State {
    async fn log(&mut self, level: log::Level, message: String) {
        let plugin = &*self.plugin_id;
        match level {
            log::Level::Trace => tracing::trace!(target: "kopuz_plugin", %plugin, "{message}"),
            log::Level::Debug => tracing::debug!(target: "kopuz_plugin", %plugin, "{message}"),
            log::Level::Info => tracing::info!(target: "kopuz_plugin", %plugin, "{message}"),
            log::Level::Warn => tracing::warn!(target: "kopuz_plugin", %plugin, "{message}"),
            log::Level::Error => tracing::error!(target: "kopuz_plugin", %plugin, "{message}"),
        }
    }
}

/// More than any one library holds, so a plugin cannot grow the host's memory without bound.
const MAX_BIT_DEPTHS: usize = 1_000_000;

impl details::Host for State {
    async fn bit_depth(&mut self, item_id: String, bits: u8) {
        if bits > 0 && self.bit_depths.len() < MAX_BIT_DEPTHS {
            self.bit_depths.insert(item_id, bits);
        }
    }
}

impl decryption::Host for State {
    async fn aes_ctr(&mut self, url: String, key: Vec<u8>, iv: Vec<u8>, skip: u32) {
        let (Ok(key), Ok(iv)) = (key.try_into(), iv.try_into()) else {
            tracing::warn!(plugin = %self.plugin_id, "plugin stream key ignored: a key and an IV are 16 bytes each");
            return;
        };
        self.decryptions.insert(url, Decryption { key, iv, skip });
    }
}

impl tcp::Host for State {
    async fn connect(
        &mut self,
        host: String,
        port: u16,
        timeout_ms: Option<u32>,
    ) -> Result<u32, tcp::TcpError> {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if !self.reaches(&host) {
            tracing::warn!(plugin = %self.plugin_id, %host, "plugin connection to an undeclared host refused");
            return Err(tcp::TcpError::Denied(format!(
                "the plugin's manifest does not allow {host}"
            )));
        }
        if self.connections.len() >= MAX_CONNECTIONS {
            return Err(tcp::TcpError::Other(
                "the plugin has too many connections open".into(),
            ));
        }
        let local_allowed = self.server_host.as_deref() == Some(host.as_str());
        let addrs = lookup(&host, port, local_allowed)
            .await
            .map_err(|error| tcp::TcpError::Connect(error.to_string()))?;
        let limit = timeout_of(timeout_ms);
        let attempt = async {
            let mut last = None;
            for addr in addrs {
                match tokio::net::TcpStream::connect(addr).await {
                    Ok(stream) => return Ok(stream),
                    Err(error) => last = Some(error),
                }
            }
            Err(last.map_or_else(|| "the host has no address".to_string(), |e| e.to_string()))
        };
        let stream = tokio::time::timeout(limit, attempt)
            .await
            .map_err(|_| tcp::TcpError::Timeout)?
            .map_err(tcp::TcpError::Connect)?;
        // Protocols over this are small request and answer frames.
        let _ = stream.set_nodelay(true);
        self.next_connection = self.next_connection.wrapping_add(1);
        let id = self.next_connection;
        self.connections.insert(id, stream);
        Ok(id)
    }

    async fn write(&mut self, connection: u32, bytes: Vec<u8>) -> Result<(), tcp::TcpError> {
        use tokio::io::AsyncWriteExt as _;
        let stream = self.connection(connection)?;
        tokio::time::timeout(DEFAULT_TIMEOUT, stream.write_all(&bytes))
            .await
            .map_err(|_| tcp::TcpError::Timeout)?
            .map_err(io_error)
    }

    async fn read(
        &mut self,
        connection: u32,
        max: u32,
        timeout_ms: Option<u32>,
    ) -> Result<Vec<u8>, tcp::TcpError> {
        use tokio::io::AsyncReadExt as _;
        let stream = self.connection(connection)?;
        let mut bytes = vec![0; (max as usize).clamp(1, MAX_READ_BYTES)];
        let read = tokio::time::timeout(timeout_of(timeout_ms), stream.read(&mut bytes))
            .await
            .map_err(|_| tcp::TcpError::Timeout)?
            .map_err(io_error)?;
        if read == 0 {
            return Err(tcp::TcpError::Closed);
        }
        bytes.truncate(read);
        Ok(bytes)
    }

    async fn close(&mut self, connection: u32) {
        self.connections.remove(&connection);
    }
}

impl State {
    fn connection(&mut self, id: u32) -> Result<&mut tokio::net::TcpStream, tcp::TcpError> {
        self.connections
            .get_mut(&id)
            .ok_or_else(|| tcp::TcpError::Other("no such connection".into()))
    }
}

fn timeout_of(timeout_ms: Option<u32>) -> Duration {
    timeout_ms.map_or(DEFAULT_TIMEOUT, |ms| Duration::from_millis(ms.into()))
}

fn io_error(error: std::io::Error) -> tcp::TcpError {
    use std::io::ErrorKind::{BrokenPipe, ConnectionAborted, ConnectionReset, UnexpectedEof};
    match error.kind() {
        BrokenPipe | ConnectionAborted | ConnectionReset | UnexpectedEof => tcp::TcpError::Closed,
        _ => tcp::TcpError::Other(error.to_string()),
    }
}

impl browser::Host for State {
    async fn sign_in(
        &mut self,
        authorize_url: String,
        port: u16,
        path: String,
    ) -> Result<String, browser::SignInError> {
        use browser::SignInError;
        let Some(open) = self.opener.clone() else {
            return Err(SignInError::Unavailable(
                "a browser sign-in runs only while signing in".into(),
            ));
        };
        let url = reqwest::Url::parse(&authorize_url)
            .map_err(|error| SignInError::Other(format!("invalid URL: {error}")))?;
        let host = url.host_str().unwrap_or_default();
        if url.scheme() != "https" || !self.reaches(host) {
            tracing::warn!(plugin = %self.plugin_id, %host, "plugin sign-in on an undeclared host refused");
            return Err(SignInError::Denied(format!(
                "the plugin's manifest does not allow {host}"
            )));
        }
        if port < 1024 || !path.starts_with('/') {
            return Err(SignInError::Other(
                "the redirect needs an unprivileged port and a path".into(),
            ));
        }
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|error| {
                SignInError::Unavailable(format!("port {port} is not free: {error}"))
            })?;
        open(url.as_str()).map_err(SignInError::Unavailable)?;
        tokio::time::timeout(SIGN_IN_LIMIT, redirected(&listener, &path))
            .await
            .map_err(|_| SignInError::Timeout)?
            .map_err(|error| SignInError::Other(error.to_string()))
    }
}

/// The path and query of the first request for `path`. A browser also asks
/// for a favicon, which is not the redirect.
async fn redirected(listener: &tokio::net::TcpListener, path: &str) -> std::io::Result<String> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    const DONE: &str = "<!doctype html><meta charset=\"utf-8\"><title>Kopuz</title>\
        <body style=\"font-family:sans-serif;text-align:center;padding-top:4rem\">\
        <h2>Signed in.</h2><p>You can close this tab and return to Kopuz.</p></body>";
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut request = Vec::new();
        let mut chunk = [0; 2048];
        let head = async {
            while !request.windows(4).any(|window| window == b"\r\n\r\n") && request.len() < 16384 {
                match stream.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => request.extend_from_slice(&chunk[..read]),
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(5), head).await;
        let request = String::from_utf8_lossy(&request);
        let target = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or_default();
        let asked = target.split_once('?').map_or(target, |(asked, _)| asked);
        let answer = if asked == path {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{DONE}",
                DONE.len()
            )
        } else {
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
        };
        let _ = stream.write_all(answer.as_bytes()).await;
        let _ = stream.shutdown().await;
        if asked == path {
            return Ok(target.to_string());
        }
    }
}

impl crate::bindings::kopuz::plugin::types::Host for State {}

#[cfg(test)]
mod tests {
    use super::*;

    async fn resolves(gate: &Gate, host: &str) -> bool {
        use reqwest::dns::Resolve as _;
        gate.resolve(host.parse().expect("a host name"))
            .await
            .is_ok()
    }

    #[tokio::test]
    async fn only_the_server_may_lead_to_this_machine_or_its_network() {
        let elsewhere = Gate {
            server_host: Some("music.example".into()),
        };
        assert!(!resolves(&elsewhere, "localhost").await);
        let server = Gate {
            server_host: Some("localhost".into()),
        };
        assert!(resolves(&server, "localhost").await);
        assert!(resolves(&server, "LOCALHOST.").await);
    }

    #[test]
    fn a_local_address_written_into_a_url_is_only_the_servers() {
        assert!(local_literal("192.168.1.20", None));
        assert!(local_literal("127.0.0.1", Some("music.example")));
        assert!(local_literal("[::1]", None));
        assert!(local_literal("0.0.0.0", None));
        assert!(!local_literal("192.168.1.20", Some("192.168.1.20")));
        assert!(!local_literal("93.184.216.34", None));
        assert!(!local_literal("music.example", None));
    }

    fn state(network: &[&str], server_url: &str) -> State {
        let permissions = Permissions {
            network: network.iter().map(|host| host.to_string()).collect(),
            ..Permissions::default()
        };
        State::new("test".into(), Arc::new(permissions), server_url, &[])
    }

    #[tokio::test]
    async fn a_connection_is_held_to_the_manifests_hosts() {
        use tcp::Host as _;
        let mut state = state(&["ap.example"], "");
        for host in ["evil.example", "127.0.0.1", "localhost", "[::1]"] {
            assert!(
                matches!(
                    state.connect(host.into(), 4070, None).await,
                    Err(tcp::TcpError::Denied(_))
                ),
                "{host} is not on the plugin's hosts"
            );
        }
        // Allowed by name, but a name may not lead to this machine.
        let mut local = state_named_localhost();
        assert!(matches!(
            local.connect("localhost".into(), 4070, None).await,
            Err(tcp::TcpError::Connect(_))
        ));
    }

    fn state_named_localhost() -> State {
        state(&["localhost"], "")
    }

    #[tokio::test]
    async fn a_connection_to_the_server_writes_and_reads() {
        use tcp::Host as _;
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("address").port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut asked = [0; 4];
            stream.read_exact(&mut asked).await.expect("read");
            stream.write_all(&asked).await.expect("write");
        });
        let mut state = state(&["$server"], &format!("http://127.0.0.1:{port}"));
        let id = state
            .connect("127.0.0.1".into(), port, Some(5000))
            .await
            .expect("the server is the one local address allowed");
        state.write(id, b"ping".to_vec()).await.expect("write");
        assert_eq!(state.read(id, 16, Some(5000)).await.expect("read"), b"ping");
        assert!(matches!(
            state.read(id, 16, Some(5000)).await,
            Err(tcp::TcpError::Closed)
        ));
        state.close(id).await;
        assert!(matches!(
            state.read(id, 16, None).await,
            Err(tcp::TcpError::Other(_))
        ));
    }

    #[tokio::test]
    async fn a_browser_sign_in_runs_only_for_login_and_on_the_manifests_hosts() {
        use browser::Host as _;
        let mut state = state(&["accounts.example"], "");
        let url = "https://accounts.example/authorize?state=1";
        assert!(matches!(
            state.sign_in(url.into(), 18898, "/login".into()).await,
            Err(browser::SignInError::Unavailable(_))
        ));

        // Stands in for the browser: asks for a favicon, then follows the redirect.
        state.allow_sign_in(Arc::new(|opened| {
            assert_eq!(opened, "https://accounts.example/authorize?state=1");
            tokio::spawn(async {
                use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
                for target in ["/favicon.ico", "/login?code=abc&state=1"] {
                    let mut stream = tokio::net::TcpStream::connect("127.0.0.1:18898")
                        .await
                        .expect("connect");
                    let request = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
                    stream.write_all(request.as_bytes()).await.expect("write");
                    let mut answer = Vec::new();
                    let _ = stream.read_to_end(&mut answer).await;
                }
            });
            Ok(())
        }));
        assert!(matches!(
            state
                .sign_in(
                    "https://elsewhere.example/authorize".into(),
                    18898,
                    "/login".into()
                )
                .await,
            Err(browser::SignInError::Denied(_))
        ));
        assert_eq!(
            state.sign_in(url.into(), 18898, "/login".into()).await.ok(),
            Some("/login?code=abc&state=1".to_string())
        );
    }

    #[tokio::test]
    async fn a_stream_key_of_the_wrong_length_is_ignored() {
        use decryption::Host as _;
        let mut state = state(&[], "");
        state
            .aes_ctr(
                "https://cdn.example/a".into(),
                vec![1; 16],
                vec![2; 16],
                167,
            )
            .await;
        state
            .aes_ctr("https://cdn.example/b".into(), vec![1; 8], vec![2; 16], 0)
            .await;
        assert_eq!(
            state.decryptions.get("https://cdn.example/a"),
            Some(&Decryption {
                key: [1; 16],
                iv: [2; 16],
                skip: 167
            })
        );
        assert!(!state.decryptions.contains_key("https://cdn.example/b"));
    }

    #[test]
    fn the_server_host_is_read_from_its_url() {
        assert_eq!(
            host_of("https://Music.Home.test:4533/navidrome").as_deref(),
            Some("music.home.test")
        );
        assert_eq!(host_of("not a url"), None);
    }
}
