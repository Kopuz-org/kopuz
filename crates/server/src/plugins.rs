//! The plugins this process loaded, and the sessions open on them: the native
//! ones, the sandboxed ones embedded in the binary, then whatever is installed
//! in the plugin directory the daemon names at boot.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use arc_swap::ArcSwap;
use plugin_host::{Bundled, Registry, Session, wit};

use crate::server_ops::ServerConn;

static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
static REGISTRY: OnceLock<ArcSwap<Registry>> = OnceLock::new();

/// Name the directory installed plugins are read from and load them. Only the
/// first call counts.
pub async fn init(dir: Option<PathBuf>) {
    if DIR.set(dir).is_err() {
        tracing::debug!("the plugin directory was already set");
        return;
    }
    let _one_at_a_time = CHANGING.lock().await;
    let loaded = Arc::new(Registry::load(&official(), self::dir()).await);
    for plugin in loaded.plugins() {
        tracing::info!(
            plugin = %plugin.id(),
            version = %plugin.manifest().version,
            origin = ?plugin.origin(),
            native = plugin.is_native(),
            "plugin loaded"
        );
    }
    let current = registry();
    // `commit` saves only what differs from `current`, and the file on disk
    // may not match it yet.
    if current.same_approvals(&loaded) {
        save_approved(&loaded);
    }
    commit(&current, loaded);
}

/// What is loaded now. Before [`init`] that is only the plugins kopuz ships.
pub fn registry() -> Arc<Registry> {
    REGISTRY
        .get_or_init(|| ArcSwap::from_pointee(Registry::bundled(&official())))
        .load_full()
}

fn official() -> Vec<Bundled> {
    crate::source::native_plugins()
        .map(|manifest| Bundled {
            manifest,
            wasm: None,
        })
        .chain(bundled_plugins::all().iter().copied())
        .collect()
}

pub fn dir() -> Option<&'static std::path::Path> {
    DIR.get_or_init(|| None).as_deref()
}

/// Held by whatever writes the plugin directory or swaps the registry, so a
/// rescan never reads an install half done.
static CHANGING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Read the plugin directory again and swap in what changed, keeping the old
/// version of a plugin whose new files fail to load. Whether anything did.
pub async fn rescan() -> bool {
    let Some(dir) = dir() else {
        return false;
    };
    let _one_at_a_time = CHANGING.lock().await;
    let current = registry();
    let next = current.reload(dir).await;
    commit(&current, Arc::new(next))
}

/// Run `write`, which puts `plugin`'s files in the plugin directory and says
/// where, then serve `plugin`, already checked and compiled, from there.
pub async fn adopt<E: Send + 'static>(
    plugin: Arc<plugin_host::Plugin>,
    write: impl FnOnce() -> Result<PathBuf, E> + Send + 'static,
) -> Result<(), E> {
    let _one_at_a_time = CHANGING.lock().await;
    let dir = tokio::task::spawn_blocking(write)
        .await
        .unwrap_or_else(|panicked| std::panic::resume_unwind(panicked.into_panic()))?;
    let current = registry();
    commit(&current, Arc::new(current.with_installed(dir, plugin)));
    Ok(())
}

/// Run `remove`, which deletes plugin files, then read the directory again.
pub async fn remove<E: Send + 'static>(
    remove: impl FnOnce() -> Result<(), E> + Send + 'static,
) -> Result<(), E> {
    let _one_at_a_time = CHANGING.lock().await;
    tokio::task::spawn_blocking(remove)
        .await
        .unwrap_or_else(|panicked| std::panic::resume_unwind(panicked.into_panic()))?;
    let current = registry();
    if let Some(dir) = dir() {
        let next = current.reload(dir).await;
        commit(&current, Arc::new(next));
    }
    Ok(())
}

/// Load the update of `id` that waits for its new hosts, if it reaches
/// exactly `network`. `None` when no such update waits or the hosts differ.
pub async fn approve(id: &str, network: &[String]) -> Option<()> {
    let _one_at_a_time = CHANGING.lock().await;
    let current = registry();
    let next = current.approve(id, network)?;
    commit(&current, Arc::new(next));
    Some(())
}

/// Swap `next` in for `current`, the caller holding [`CHANGING`].
fn commit(current: &Arc<Registry>, next: Arc<Registry>) -> bool {
    if !current.same_approvals(&next) {
        save_approved(&next);
    }
    if current.same_as(&next) {
        return false;
    }
    REGISTRY
        .get_or_init(|| ArcSwap::new(Arc::clone(current)))
        .store(Arc::clone(&next));
    swap_sessions(&next);
    true
}

fn save_approved(registry: &Registry) {
    let Some(dir) = dir() else {
        return;
    };
    if let Err(error) = registry.save_approved(dir) {
        tracing::warn!(dir = %dir.display(), %error, "agreed plugin hosts not saved");
    }
}

type OpenSessions = Mutex<HashMap<String, Weak<Session>>>;

fn open_sessions() -> &'static OpenSessions {
    static OPEN: OnceLock<OpenSessions> = OnceLock::new();
    OPEN.get_or_init(Default::default)
}

pub(crate) fn connection(conn: &ServerConn) -> wit::Connection {
    let service = match (conn.service, conn.plugin_service.as_deref()) {
        (config::MusicService::Plugin, Some(slug)) => slug,
        (service, _) => service.id(),
    };
    wit::Connection {
        service: service.to_string(),
        url: conn.url.clone(),
        token: conn.token.clone(),
        user_id: conn.user_id.clone(),
        device_id: conn.device_id.clone(),
        account: conn.account.clone(),
        folders: conn.folders.clone(),
    }
}

/// The session for `connection`, shared while anything holds it, so call
/// sites that rebuild a source per operation reuse warm instances. A reload
/// swaps the plugin under it instead of opening another.
pub(crate) fn session(connection: wit::Connection) -> Option<Arc<Session>> {
    // Locked before the registry is read, so a commit in between swaps this
    // session forward after it rather than this call swapping it back.
    let mut open = open_sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let registry = registry();
    let plugin = registry
        .for_service(&connection.service)
        .filter(|plugin| !plugin.is_native())?;
    let key = format!(
        "{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
        connection.service,
        connection.url,
        connection.device_id,
        connection.user_id,
        connection.token,
        connection.account.as_deref().unwrap_or_default(),
        connection.folders.join("\u{1}"),
    );
    open.retain(|_, session| session.strong_count() > 0);
    if let Some(session) = open.get(&key).and_then(Weak::upgrade) {
        if !Arc::ptr_eq(&session.plugin(), plugin) {
            session.swap(Arc::clone(plugin));
        }
        return Some(session);
    }
    let session = Arc::new(Session::new(Arc::clone(plugin), connection));
    open.insert(key, Arc::downgrade(&session));
    Some(session)
}

/// Point every open session at the plugin now serving its service. A session
/// whose service lost its sandboxed plugin keeps the old code until its source
/// is rebuilt.
fn swap_sessions(registry: &Registry) {
    let open = open_sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for session in open.values().filter_map(Weak::upgrade) {
        let Some(plugin) = registry
            .for_service(session.service())
            .filter(|plugin| !plugin.is_native())
        else {
            continue;
        };
        let current = session.plugin();
        if !Arc::ptr_eq(&current, plugin) {
            tracing::info!(
                plugin = %plugin.id(),
                from = %current.manifest().version,
                to = %plugin.manifest().version,
                "open session moved to the new plugin"
            );
            session.swap(Arc::clone(plugin));
        }
    }
}

/// The plugins shipped inside kopuz, which the marketplace may not offer or replace.
pub fn official_ids() -> Vec<String> {
    official()
        .iter()
        .filter_map(|bundled| plugin_host::manifest::Manifest::parse(bundled.manifest).ok())
        .map(|manifest| manifest.id)
        .collect()
}

#[derive(Debug, Clone)]
pub struct OwnService {
    pub service: plugin_host::manifest::Service,
    pub origin: plugin_host::Origin,
}

/// Every service a sandboxed plugin adds that kopuz has no built-in for.
pub fn own_services() -> Vec<OwnService> {
    let registry = registry();
    let mut services: Vec<OwnService> = registry
        .plugins()
        .iter()
        .filter(|plugin| !plugin.is_native())
        .flat_map(|plugin| {
            plugin
                .manifest()
                .services
                .iter()
                .filter(|service| config::MusicService::from_id(&service.id).is_none())
                .filter(|service| {
                    registry
                        .for_service(&service.id)
                        .is_some_and(|serving| Arc::ptr_eq(serving, plugin))
                })
                .map(|service| OwnService {
                    service: service.clone(),
                    origin: plugin.origin(),
                })
        })
        .collect();
    services.sort_by(|a, b| a.service.id.cmp(&b.service.id));
    services
}

pub fn own_service(slug: &str) -> Option<OwnService> {
    own_services()
        .into_iter()
        .find(|own| own.service.id == slug)
}

/// What a plugin may reach. The pictures it names are fetched or read by the
/// host, outside the sandbox, so they are held to the same hosts.
pub struct Reach {
    /// Shipped inside kopuz, so its references are ours to trust.
    trusted: bool,
    permissions: plugin_host::manifest::Permissions,
    server_host: Option<String>,
}

impl Reach {
    pub(crate) fn new(plugin: &plugin_host::Plugin, server_url: &str) -> Self {
        Self::from_parts(
            plugin.origin() == plugin_host::Origin::Bundled,
            plugin.manifest().permissions.clone(),
            server_url,
        )
    }

    pub fn from_parts(
        trusted: bool,
        permissions: plugin_host::manifest::Permissions,
        server_url: &str,
    ) -> Self {
        Self {
            trusted,
            permissions,
            server_host: reqwest::Url::parse(server_url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_string)),
        }
    }

    /// The reach of the sandboxed plugin serving `server`, unless kopuz ships it.
    pub fn of_installed(server: &config::MusicServer) -> Option<Self> {
        let registry = registry();
        let plugin = registry
            .for_service(server.service_id())
            .filter(|plugin| !plugin.is_native())?;
        let reach = Self::new(plugin, &server.url);
        (!reach.trusted).then_some(reach)
    }

    /// `raw` if the host may follow it for this plugin, else nothing.
    pub(crate) fn cover(&self, raw: Option<String>) -> Option<String> {
        let raw = raw?;
        let keep = match reader::CoverRef::parse(&raw) {
            reader::CoverRef::Local(_) => false,
            reader::CoverRef::EmbeddedUrl(url) => self.allows(&url),
            reader::CoverRef::JellyfinItem { .. } | reader::CoverRef::SubsonicItem { .. } => {
                self.trusted
            }
            reader::CoverRef::None => self.trusted || raw == reader::CoverRef::NO_COVER,
        };
        keep.then_some(raw)
    }

    /// Whether the host may fetch `url` for this plugin.
    pub fn allows(&self, url: &str) -> bool {
        let Ok(url) = reqwest::Url::parse(url) else {
            return false;
        };
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
            && url.host_str().is_some_and(|host| {
                self.trusted || self.permissions.allows(host, self.server_host.as_deref())
            })
    }
}
