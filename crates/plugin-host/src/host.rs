//! What a plugin can reach: the host side of its imports.

use std::sync::Arc;
use std::time::Duration;

use wasmtime::component::ResourceTable;
use wasmtime::{StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::bindings::kopuz::plugin::{http, log};
use crate::manifest::Permissions;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

const MAX_BODY_BYTES: usize = 512 * 1024 * 1024;

const MAX_MEMORY_BYTES: usize = 512 * 1024 * 1024;

pub(crate) struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    pub(crate) limits: StoreLimits,
    plugin_id: Arc<str>,
    permissions: Arc<Permissions>,
    /// What `$server` names.
    server_host: Option<String>,
    client: reqwest::Client,
}

impl State {
    pub(crate) fn new(
        plugin_id: Arc<str>,
        permissions: Arc<Permissions>,
        server_url: &str,
    ) -> Self {
        let server_host = host_of(server_url);
        let client = client_for(&permissions, server_host.clone());
        // Linked so `std` runs, with no files, sockets or environment.
        let wasi = WasiCtx::builder().build();
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
        }
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
fn client_for(permissions: &Arc<Permissions>, server_host: Option<String>) -> reqwest::Client {
    let permissions = Arc::clone(permissions);
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        let allowed = attempt
            .url()
            .host_str()
            .is_some_and(|host| permissions.allows(host, server_host.as_deref()));
        if !allowed {
            attempt.error("redirected to a host the plugin's manifest does not allow")
        } else if attempt.previous().len() >= 10 {
            attempt.error("too many redirects")
        } else {
            attempt.follow()
        }
    });
    utils::http::builder()
        .redirect(policy)
        .build()
        .unwrap_or_else(|_| utils::http::client())
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
        let host = url.host_str().unwrap_or_default();
        if !self.permissions.allows(host, self.server_host.as_deref()) {
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

        let mut builder = self.client.request(method, url).timeout(timeout);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        let mut response = builder.send().await.map_err(request_error)?;

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

impl crate::bindings::kopuz::plugin::types::Host for State {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_host_is_read_from_its_url() {
        assert_eq!(
            host_of("https://Music.Home.test:4533/navidrome").as_deref(),
            Some("music.home.test")
        );
        assert_eq!(host_of("not a url"), None);
    }
}
