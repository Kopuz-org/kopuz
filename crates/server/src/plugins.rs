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

/// Name the directory installed plugins are read from. Only the first call
/// counts, and only before anything reached for the registry.
pub fn init(dir: Option<PathBuf>) {
    if DIR.set(dir).is_err() {
        tracing::debug!("the plugin directory was already set");
    }
    for plugin in registry().plugins() {
        tracing::info!(
            plugin = %plugin.id(),
            version = %plugin.manifest().version,
            origin = ?plugin.origin(),
            native = plugin.is_native(),
            "plugin loaded"
        );
    }
}

pub fn registry() -> Arc<Registry> {
    REGISTRY
        .get_or_init(|| ArcSwap::from_pointee(load()))
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

fn load() -> Registry {
    Registry::load(&official(), dir())
}

pub fn dir() -> Option<&'static std::path::Path> {
    DIR.get_or_init(|| None).as_deref()
}

/// Read the plugin directory again. A source already open keeps the plugin it
/// opened with until it is rebuilt.
pub fn reload() {
    let registry = Arc::new(load());
    match REGISTRY.get() {
        Some(current) => current.store(registry),
        None => {
            let _ = REGISTRY.set(ArcSwap::new(registry));
        }
    }
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
/// sites that rebuild a source per operation reuse warm instances.
pub(crate) fn session(connection: wit::Connection) -> Option<Arc<Session>> {
    static OPEN: OnceLock<Mutex<HashMap<String, Weak<Session>>>> = OnceLock::new();
    let registry = registry();
    let plugin = registry.for_service(&connection.service)?;
    // Keyed on the plugin's address too, so a reload never reuses old code.
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
