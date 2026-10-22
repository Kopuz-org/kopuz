//! The plugins this process loaded, and the sessions open on them.
//!
//! The registry is built once: the official plugins embedded in the binary,
//! then whatever is installed in the plugin directory the daemon names at
//! boot. Services a plugin serves are dispatched to it before any built-in
//! backend is considered.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use arc_swap::ArcSwap;
use plugin_host::{Registry, Session, wit};

use crate::server_ops::ServerConn;

static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
static REGISTRY: OnceLock<ArcSwap<Registry>> = OnceLock::new();

/// Name the directory installed plugins are read from. Only the first call
/// counts, and only before anything reached for the registry: plugins are
/// loaded once per process.
pub fn init(dir: Option<PathBuf>) {
    if DIR.set(dir).is_err() {
        tracing::debug!("the plugin directory was already set");
    }
    let registry = registry();
    for plugin in registry.plugins() {
        tracing::info!(
            plugin = %plugin.id(),
            version = %plugin.manifest().version,
            origin = ?plugin.origin(),
            "plugin loaded"
        );
    }
}

pub fn registry() -> Arc<Registry> {
    REGISTRY
        .get_or_init(|| ArcSwap::from_pointee(load()))
        .load_full()
}

fn load() -> Registry {
    Registry::load(bundled_plugins::all(), dir())
}

/// Where installed plugins live, if the daemon named a directory.
pub fn dir() -> Option<&'static std::path::Path> {
    DIR.get_or_init(|| None).as_deref()
}

/// Read the plugin directory again, after a plugin was installed or removed.
/// A source already open keeps the plugin it opened with until it is rebuilt.
pub fn reload() {
    let registry = Arc::new(load());
    match REGISTRY.get() {
        Some(current) => current.store(registry),
        None => {
            let _ = REGISTRY.set(ArcSwap::new(registry));
        }
    }
}

/// The plugin connection for a resolved server connection.
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

/// The session for `connection`, shared while anything holds it.
///
/// A source is rebuilt on every switch and credential change, and some call
/// sites build one per operation. Sharing the session keeps their calls on
/// one pool of warm instances instead of instantiating the plugin afresh.
pub(crate) fn session(connection: wit::Connection) -> Option<Arc<Session>> {
    static OPEN: OnceLock<Mutex<HashMap<String, Weak<Session>>>> = OnceLock::new();
    let registry = registry();
    let plugin = registry.for_service(&connection.service)?;
    // The plugin's address is part of the key, so a reload that replaced it
    // opens a fresh session rather than reusing one on the old code.
    let key = format!(
        "{:p}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
        Arc::as_ptr(plugin),
        connection.service,
        connection.url,
        connection.device_id,
        connection.user_id,
        connection.token,
        connection.account.as_deref().unwrap_or_default(),
        connection.folders.join("\u{1}"),
    );
    let mut open = OPEN
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    open.retain(|_, session| session.strong_count() > 0);
    if let Some(session) = open.get(&key).and_then(Weak::upgrade) {
        return Some(session);
    }
    let session = Arc::new(Session::new(Arc::clone(plugin), connection));
    open.insert(key, Arc::downgrade(&session));
    Some(session)
}

/// A service a plugin adds of its own, as its manifest describes it.
#[derive(Debug, Clone)]
pub struct OwnService {
    pub service: plugin_host::manifest::Service,
    pub origin: plugin_host::Origin,
}

/// Every service a loaded plugin adds that kopuz has no built-in for.
pub fn own_services() -> Vec<OwnService> {
    let registry = registry();
    let mut services: Vec<OwnService> = registry
        .plugins()
        .iter()
        .flat_map(|plugin| {
            plugin
                .manifest()
                .services
                .iter()
                .filter(|service| config::MusicService::from_id(&service.id).is_none())
                .filter(|service| {
                    // Only the plugin that won the slug speaks for it.
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

/// The plugin service `slug` names, when it is one and not a built-in.
pub fn own_service(slug: &str) -> Option<OwnService> {
    own_services()
        .into_iter()
        .find(|own| own.service.id == slug)
}
