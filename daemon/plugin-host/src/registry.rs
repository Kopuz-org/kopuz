//! Every loaded plugin, and which one serves each service. A registry is
//! never changed in place: a reload builds the next one, reusing every plugin
//! whose files did not change, so a holder of the old one keeps a whole view.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::manifest::Manifest;
use crate::plugin::{Code, Origin, Plugin};

pub const MANIFEST_FILE: &str = "kopuz-plugin.toml";
pub const COMPONENT_FILE: &str = "plugin.wasm";
/// The hosts agreed to for each installed plugin, kept in the plugin
/// directory so an update waiting for its hosts still waits after a restart.
pub const APPROVED_FILE: &str = ".approved-hosts.toml";

type Approved = BTreeMap<String, BTreeSet<String>>;

/// A plugin shipped inside kopuz: a component, or none for a native plugin.
#[derive(Debug, Clone, Copy)]
pub struct Bundled {
    pub manifest: &'static str,
    pub wasm: Option<&'static [u8]>,
}

/// New files for a loaded plugin that reach hosts the loaded version may not.
/// They are compiled and held, and the loaded version keeps serving, until
/// someone agrees to the hosts.
#[derive(Debug, Clone)]
pub struct Pending {
    pub plugin: Arc<Plugin>,
    /// What it reaches that the loaded version does not.
    pub added_hosts: Vec<String>,
    dir: PathBuf,
}

/// The plugins Flatpak extensions provide. Read once at load, since Flatpak
/// mounts an extension only when the app starts.
#[derive(Debug, Default, Clone)]
struct Extensions {
    plugins: Vec<(PathBuf, Arc<Plugin>)>,
    unreadable: bool,
}

#[derive(Debug, Default, Clone)]
pub struct Registry {
    plugins: Vec<Arc<Plugin>>,
    by_service: HashMap<String, Arc<Plugin>>,
    bundled: Vec<Arc<Plugin>>,
    extensions: Extensions,
    /// Each installed plugin and the directory it was read from, in load order.
    installed: Vec<(PathBuf, Arc<Plugin>)>,
    pending: Vec<Pending>,
    approved: Approved,
    /// A plugin directory failed to read, so any recorded id may be its.
    unreadable: bool,
}

enum Reread {
    Load(Arc<Plugin>),
    /// Keep what the directory had loaded, for this reason.
    Keep(String),
    Wait(Pending),
}

impl Registry {
    /// The bundled plugins, then each plugin in the read-only `extensions`
    /// directories (a Flatpak extension's, a system package's), in order,
    /// then each plugin installed under `dir`. A later layer's
    /// plugin with an earlier one's id replaces it, which is how a fix or a
    /// fork of an official plugin is tried without rebuilding kopuz. A plugin
    /// is compiled first, so one that does not compile never displaces the
    /// plugin it would replace. One that reaches hosts beyond those agreed to
    /// for its id waits in [`Self::pending`], as on a reload; one with no
    /// record is taken as agreed, as before records.
    pub async fn load(bundled: &[Bundled], dir: Option<&Path>, extensions: &[PathBuf]) -> Self {
        let approved = dir.map(read_approved).unwrap_or_default();
        let mut provided = Extensions::default();
        let mut roots = Vec::new();
        for root in extensions {
            match read_plugin_dirs(root) {
                Ok(paths) => roots.extend(paths),
                Err(_) => provided.unreadable = true,
            }
        }
        for path in roots {
            match compile(&path, Origin::Extension).await {
                Ok(plugin) => provided.plugins.push((path, Arc::new(plugin))),
                Err(error) => {
                    provided.unreadable = true;
                    tracing::warn!(plugin = %path.display(), %error, "plugin not loaded");
                }
            }
        }
        let mut installed = Vec::new();
        let mut pending = Vec::new();
        let mut unreadable = false;
        let paths = match dir.map(read_plugin_dirs).transpose() {
            Ok(paths) => paths.unwrap_or_default(),
            Err(_) => {
                unreadable = true;
                Vec::new()
            }
        };
        for path in paths {
            match compile(&path, Origin::Installed).await {
                Ok(plugin) => {
                    let plugin = Arc::new(plugin);
                    let added_hosts = approved
                        .get(plugin.id())
                        .map(|agreed| added_hosts(agreed, &plugin))
                        .unwrap_or_default();
                    if added_hosts.is_empty() {
                        installed.push((path, plugin));
                    } else {
                        tracing::warn!(
                            plugin = %plugin.id(),
                            hosts = ?added_hosts,
                            "a plugin reaches hosts not yet allowed and waits"
                        );
                        pending.push(Pending {
                            plugin,
                            added_hosts,
                            dir: path,
                        });
                    }
                }
                Err(error) => {
                    unreadable = true;
                    tracing::warn!(plugin = %path.display(), %error, "plugin not loaded");
                }
            }
        }
        Self::assemble(
            Self::parse_bundled(bundled),
            provided,
            installed,
            pending,
            &approved,
            unreadable,
        )
    }

    /// Only the bundled plugins, for a process that names no plugin directory.
    pub fn bundled(bundled: &[Bundled]) -> Self {
        Self::assemble(
            Self::parse_bundled(bundled),
            Extensions::default(),
            Vec::new(),
            Vec::new(),
            &Approved::new(),
            false,
        )
    }

    fn parse_bundled(bundled: &[Bundled]) -> Vec<Arc<Plugin>> {
        bundled
            .iter()
            .filter_map(|plugin| {
                let code = match plugin.wasm {
                    Some(wasm) => Code::Wasm(Cow::Borrowed(wasm)),
                    None => Code::Native,
                };
                match Manifest::parse(plugin.manifest) {
                    Ok(manifest) => Some(Arc::new(Plugin::new(manifest, code, Origin::Bundled))),
                    Err(error) => {
                        tracing::error!(%error, "a bundled plugin's manifest is invalid");
                        None
                    }
                }
            })
            .collect()
    }

    /// `pending` holds only installed copies; which extension plugins serve
    /// or wait is worked out here from `approved`, so it follows every change
    /// to the installed layer above them.
    fn assemble(
        bundled: Vec<Arc<Plugin>>,
        extensions: Extensions,
        installed: Vec<(PathBuf, Arc<Plugin>)>,
        mut pending: Vec<Pending>,
        approved: &Approved,
        unreadable: bool,
    ) -> Self {
        let mut serving_extensions = Vec::new();
        for (dir, plugin) in &extensions.plugins {
            if installed.iter().any(|(_, other)| other.id() == plugin.id()) {
                continue;
            }
            let added_hosts = approved
                .get(plugin.id())
                .map(|agreed| added_hosts(agreed, plugin))
                .unwrap_or_default();
            if added_hosts.is_empty() {
                serving_extensions.push(Arc::clone(plugin));
                continue;
            }
            let waiting = Pending {
                plugin: Arc::clone(plugin),
                added_hosts,
                dir: dir.clone(),
            };
            match pending
                .iter_mut()
                .find(|other| other.plugin.id() == plugin.id())
            {
                None => pending.push(waiting),
                Some(other) if other.plugin.origin() == Origin::Extension => *other = waiting,
                Some(_) => {}
            }
        }
        // A record is dropped only once no directory can hold its plugin: a
        // waiting plugin keeps it, and while a directory fails to read every
        // record stays, since which plugin it holds is unknown.
        let keep_all = unreadable || extensions.unreadable;
        let agreed: Approved = approved
            .iter()
            .filter(|(id, _)| keep_all || pending.iter().any(|waiting| waiting.plugin.id() == *id))
            .map(|(id, hosts)| (id.clone(), hosts.clone()))
            .collect();
        let mut registry = Self {
            bundled: bundled.clone(),
            extensions,
            installed: installed.clone(),
            pending,
            approved: agreed,
            unreadable,
            ..Self::default()
        };
        for plugin in bundled.into_iter().chain(serving_extensions) {
            registry.add(plugin);
        }
        for (_, plugin) in installed {
            registry.add(plugin);
        }
        // Bundled ones too, so an override reaching more waits after a restart.
        for plugin in &registry.plugins {
            registry.approved.insert(
                plugin.id().to_string(),
                host_set(&plugin.manifest().permissions.network),
            );
        }
        registry
    }

    fn add(&mut self, plugin: Arc<Plugin>) {
        if let Some(replaced) = self
            .plugins
            .iter()
            .position(|loaded| loaded.id() == plugin.id())
        {
            let old = self.plugins.remove(replaced);
            self.by_service
                .retain(|_, serving| !Arc::ptr_eq(serving, &old));
            tracing::info!(
                plugin = %plugin.id(),
                version = %plugin.manifest().version,
                origin = ?plugin.origin(),
                replaced = ?old.origin(),
                "a plugin replaces another with its id"
            );
        }
        for service in &plugin.manifest().services {
            if let Some(other) = self.by_service.get(&service.id) {
                tracing::warn!(
                    service = %service.id,
                    serving = %other.id(),
                    ignored = %plugin.id(),
                    "two plugins serve one service; keeping the first"
                );
                continue;
            }
            self.by_service
                .insert(service.id.clone(), Arc::clone(&plugin));
        }
        self.plugins.push(plugin);
    }

    /// Read `dir` again. A plugin whose files are unchanged is reused as it
    /// is. One whose new files fail to read or compile, or name another id,
    /// keeps the version it had loaded; one whose new files reach more hosts
    /// waits in [`Self::pending`] for [`Self::approve`].
    pub async fn reload(&self, dir: &Path) -> Self {
        let Ok(paths) = read_plugin_dirs(dir) else {
            return self.clone();
        };
        let mut installed = Vec::new();
        let mut pending = Vec::new();
        let mut unreadable = false;
        for path in paths {
            let here = self
                .installed
                .iter()
                .find(|(loaded, _)| *loaded == path)
                .map(|(_, plugin)| Arc::clone(plugin));
            match self.reread(&path, here.as_ref()).await {
                Reread::Load(plugin) => installed.push((path, plugin)),
                Reread::Keep(reason) => {
                    tracing::warn!(plugin = %path.display(), %reason, "plugin files not loaded");
                    match here {
                        Some(here) => installed.push((path, here)),
                        None => unreadable = true,
                    }
                }
                Reread::Wait(waiting) => {
                    tracing::warn!(
                        plugin = %waiting.plugin.id(),
                        hosts = ?waiting.added_hosts,
                        "a plugin update reaches new hosts and waits to be allowed"
                    );
                    if let Some(here) = here {
                        installed.push((path, here));
                    }
                    pending.push(waiting);
                }
            }
        }
        Self::assemble(
            self.bundled.clone(),
            self.extensions.clone(),
            installed,
            pending,
            &self.approved,
            unreadable,
        )
    }

    async fn reread(&self, path: &Path, here: Option<&Arc<Plugin>>) -> Reread {
        let (manifest, wasm) = match read_plugin(path) {
            Ok(files) => files,
            Err(reason) => return Reread::Keep(reason),
        };
        let digest = crate::market::sha256_hex(&wasm);
        let unchanged =
            |plugin: &Plugin| plugin.manifest() == &manifest && plugin.digest() == Some(&*digest);
        if let Some(here) = here.filter(|here| unchanged(here)) {
            return Reread::Load(Arc::clone(here));
        }
        if let Some(waiting) = self
            .pending
            .iter()
            .find(|waiting| waiting.dir == path && unchanged(&waiting.plugin))
        {
            return Reread::Wait(waiting.clone());
        }
        if let Some(here) = here.filter(|here| here.id() != manifest.id) {
            return Reread::Keep(format!(
                "its manifest changed the plugin's id from `{}` to `{}`",
                here.id(),
                manifest.id
            ));
        }
        let candidate = Plugin::new(manifest, Code::Wasm(Cow::Owned(wasm)), Origin::Installed);
        if let Err(error) = candidate.prepare().await {
            return Reread::Keep(error.to_string());
        }
        // A directory that loaded nothing before is a new install, as at start.
        let loaded = here.or_else(|| {
            self.plugins
                .iter()
                .find(|plugin| plugin.id() == candidate.id())
        });
        let agreed = self
            .approved
            .get(candidate.id())
            .cloned()
            .or_else(|| loaded.map(|loaded| host_set(&loaded.manifest().permissions.network)));
        let added_hosts = agreed
            .map(|agreed| added_hosts(&agreed, &candidate))
            .unwrap_or_default();
        let candidate = Arc::new(candidate);
        if added_hosts.is_empty() {
            Reread::Load(candidate)
        } else {
            Reread::Wait(Pending {
                plugin: candidate,
                added_hosts,
                dir: path.to_path_buf(),
            })
        }
    }

    /// Load the waiting update of `id`, if what it reaches is exactly
    /// `network`, the hosts someone was shown and agreed to.
    pub fn approve(&self, id: &str, network: &[String]) -> Option<Self> {
        let waiting = self
            .pending
            .iter()
            .find(|waiting| waiting.plugin.id() == id)?;
        let agreed = host_set(network) == host_set(&waiting.plugin.manifest().permissions.network);
        if !agreed {
            return None;
        }
        if waiting.plugin.origin() != Origin::Extension {
            return Some(self.with_installed(waiting.dir.clone(), Arc::clone(&waiting.plugin)));
        }
        let mut approved = self.approved.clone();
        approved.insert(id.to_string(), host_set(network));
        Some(Self::assemble(
            self.bundled.clone(),
            self.extensions.clone(),
            self.installed.clone(),
            self.waiting_installs().collect(),
            &approved,
            self.unreadable,
        ))
    }

    /// The waiting copies in the plugin directory, without the extension
    /// plugins [`Self::assemble`] works out again.
    fn waiting_installs(&self) -> impl Iterator<Item = Pending> + '_ {
        self.pending
            .iter()
            .filter(|waiting| waiting.plugin.origin() != Origin::Extension)
            .cloned()
    }

    /// `plugin`, read from `dir`, in place of whatever `dir` held and of any
    /// other installed copy of its id.
    pub fn with_installed(&self, dir: PathBuf, plugin: Arc<Plugin>) -> Self {
        let mut installed: Vec<_> = self
            .installed
            .iter()
            .filter(|(loaded, other)| *loaded != dir && other.id() != plugin.id())
            .cloned()
            .collect();
        let pending = self
            .waiting_installs()
            .filter(|waiting| waiting.dir != dir && waiting.plugin.id() != plugin.id())
            .collect();
        installed.push((dir, plugin));
        installed.sort_by(|a, b| a.0.cmp(&b.0));
        Self::assemble(
            self.bundled.clone(),
            self.extensions.clone(),
            installed,
            pending,
            &self.approved,
            self.unreadable,
        )
    }

    /// The plugin serving `service`, if any does.
    pub fn for_service(&self, service: &str) -> Option<&Arc<Plugin>> {
        self.by_service.get(service)
    }

    pub fn plugins(&self) -> &[Arc<Plugin>] {
        &self.plugins
    }

    pub fn pending(&self) -> &[Pending] {
        &self.pending
    }

    /// Every directory in the plugin directory holding a copy of `id`, loaded
    /// or waiting. The directory name need not be the id. A Flatpak
    /// extension's copy is never one: it is read-only and Flatpak owns it.
    pub fn dirs_of(&self, id: &str) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = self
            .installed
            .iter()
            .filter(|(_, plugin)| plugin.id() == id)
            .map(|(dir, _)| dir.clone())
            .chain(
                self.waiting_installs()
                    .filter(|waiting| waiting.plugin.id() == id)
                    .map(|waiting| waiting.dir),
            )
            .collect();
        dirs.sort();
        dirs.dedup();
        dirs
    }

    /// Whether a Flatpak extension provides `id`, serving or not.
    pub fn from_extension(&self, id: &str) -> bool {
        self.extensions
            .plugins
            .iter()
            .any(|(_, plugin)| plugin.id() == id)
    }

    /// Whether `other` loaded the very same plugins and holds the same updates.
    pub fn same_as(&self, other: &Self) -> bool {
        fn same(a: &[Arc<Plugin>], b: &[Arc<Plugin>]) -> bool {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| Arc::ptr_eq(a, b))
        }
        let waiting = |registry: &Self| -> Vec<Arc<Plugin>> {
            registry
                .pending
                .iter()
                .map(|waiting| Arc::clone(&waiting.plugin))
                .collect()
        };
        same(&self.plugins, &other.plugins) && same(&waiting(self), &waiting(other))
    }

    /// Whether `other` records the same agreed hosts.
    pub fn same_approvals(&self, other: &Self) -> bool {
        self.approved == other.approved
    }

    /// Write the agreed hosts to [`APPROVED_FILE`] in `dir`, through a
    /// rename so a crash never leaves half a record.
    pub fn save_approved(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let text = toml::to_string(&self.approved).map_err(std::io::Error::other)?;
        let staging = dir.join(format!("{APPROVED_FILE}.writing"));
        std::fs::write(&staging, text)?;
        std::fs::rename(&staging, dir.join(APPROVED_FILE))
    }
}

fn read_approved(dir: &Path) -> Approved {
    let text = match std::fs::read_to_string(dir.join(APPROVED_FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Approved::new(),
        Err(error) => {
            tracing::warn!(dir = %dir.display(), %error, "agreed plugin hosts unreadable");
            return Approved::new();
        }
    };
    toml::from_str(&text).unwrap_or_else(|error| {
        tracing::warn!(dir = %dir.display(), %error, "agreed plugin hosts unreadable");
        Approved::new()
    })
}

/// A host allowlist as a set, so two lists agree whatever their order or case.
pub fn host_set(network: &[String]) -> BTreeSet<String> {
    network
        .iter()
        .map(|host| host.trim().to_ascii_lowercase())
        .collect()
}

fn added_hosts(agreed: &BTreeSet<String>, candidate: &Plugin) -> Vec<String> {
    host_set(&candidate.manifest().permissions.network)
        .into_iter()
        .filter(|host| !agreed.contains(host))
        .collect()
}

/// Each `<dir>/<name>/`, sorted, so which of two plugins wins a service does
/// not depend on directory order. A dot directory is an install in progress.
/// Nothing where `dir` cannot be read; see [`read_plugin_dirs`] to tell that
/// apart from an empty directory.
pub fn plugin_dirs(dir: &Path) -> Vec<PathBuf> {
    read_plugin_dirs(dir).unwrap_or_default()
}

/// [`plugin_dirs`], or the error that kept `dir` from being read. A missing
/// directory holds nothing; any other failure leaves which plugins it holds
/// unknown.
pub fn read_plugin_dirs(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            tracing::warn!(dir = %dir.display(), %error, "plugin directory unreadable");
            return Err(error);
        }
    };
    let mut paths: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    paths.sort();
    Ok(paths)
}

async fn compile(dir: &Path, origin: Origin) -> Result<Plugin, String> {
    let (manifest, wasm) = read_plugin(dir)?;
    let plugin = Plugin::new(manifest, Code::Wasm(Cow::Owned(wasm)), origin);
    plugin.prepare().await.map_err(|error| error.to_string())?;
    Ok(plugin)
}

fn read_plugin(dir: &Path) -> Result<(Manifest, Vec<u8>), String> {
    let manifest = std::fs::read_to_string(dir.join(MANIFEST_FILE))
        .map_err(|error| format!("{MANIFEST_FILE}: {error}"))?;
    let manifest = Manifest::parse(&manifest).map_err(|error| error.to_string())?;
    let wasm = std::fs::read(dir.join(COMPONENT_FILE))
        .map_err(|error| format!("{COMPONENT_FILE}: {error}"))?;
    Ok((manifest, wasm))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str, version: &str, services: &[&str]) -> String {
        let mut text =
            format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\napi = \"0.1\"\n");
        for service in services {
            text.push_str(&format!("[[service]]\nid = \"{service}\"\n"));
        }
        text
    }

    #[tokio::test]
    async fn an_installed_plugin_replaces_the_bundled_one_with_its_id() {
        let bundled = manifest("subsonic", "0.1.0", &["subsonic", "custom"]);
        let bundled = [Bundled {
            manifest: Box::leak(bundled.into_boxed_str()),
            wasm: Some(b""),
        }];
        let dir = std::env::temp_dir().join(format!("kopuz-plugins-{}", std::process::id()));
        let fork = dir.join("subsonic-fork");
        std::fs::create_dir_all(&fork).unwrap();
        std::fs::write(
            fork.join(MANIFEST_FILE),
            manifest("subsonic", "0.2.0", &["subsonic"]),
        )
        .unwrap();
        std::fs::write(fork.join(COMPONENT_FILE), EMPTY_COMPONENT).unwrap();
        let broken = dir.join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join(MANIFEST_FILE), "id = ").unwrap();

        let registry = Registry::load(&bundled, Some(&dir), &[]).await;
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(registry.plugins().len(), 1);
        let serving = registry.for_service("subsonic").expect("subsonic served");
        assert_eq!(serving.manifest().version, "0.2.0");
        assert_eq!(serving.origin(), Origin::Installed);
        assert!(
            registry.for_service("custom").is_none(),
            "the fork dropped the custom service, so nothing serves it"
        );
    }

    #[test]
    fn the_first_plugin_to_claim_a_service_keeps_it() {
        let first = manifest("a", "1.0.0", &["jellyfin"]);
        let second = manifest("b", "1.0.0", &["jellyfin"]);
        let bundled = [
            Bundled {
                manifest: Box::leak(first.into_boxed_str()),
                wasm: Some(b""),
            },
            Bundled {
                manifest: Box::leak(second.into_boxed_str()),
                wasm: Some(b""),
            },
        ];
        let registry = Registry::bundled(&bundled);
        assert_eq!(registry.plugins().len(), 2);
        assert_eq!(registry.for_service("jellyfin").map(|p| p.id()), Some("a"));
    }

    #[tokio::test]
    async fn a_native_plugin_has_no_component_and_a_component_can_replace_it() {
        let native = [Bundled {
            manifest: Box::leak(manifest("ytmusic", "1.0.0", &["ytmusic"]).into_boxed_str()),
            wasm: None,
        }];
        let registry = Registry::bundled(&native);
        assert!(registry.for_service("ytmusic").unwrap().is_native());

        let dir = std::env::temp_dir().join(format!("kopuz-plugins-native-{}", std::process::id()));
        let port = dir.join("ytmusic");
        std::fs::create_dir_all(&port).unwrap();
        std::fs::write(
            port.join(MANIFEST_FILE),
            manifest("ytmusic", "2.0.0", &["ytmusic"]),
        )
        .unwrap();
        std::fs::write(port.join(COMPONENT_FILE), EMPTY_COMPONENT).unwrap();
        let registry = Registry::load(&native, Some(&dir), &[]).await;
        std::fs::remove_dir_all(&dir).ok();
        assert!(!registry.for_service("ytmusic").unwrap().is_native());
    }

    #[tokio::test]
    async fn a_component_that_does_not_compile_leaves_the_bundled_plugin() {
        let bundled = [Bundled {
            manifest: Box::leak(manifest("subsonic", "0.1.0", &["subsonic"]).into_boxed_str()),
            wasm: Some(EMPTY_COMPONENT),
        }];
        let dir = std::env::temp_dir().join(format!("kopuz-plugins-broken-{}", std::process::id()));
        let fork = dir.join("subsonic");
        std::fs::create_dir_all(&fork).unwrap();
        std::fs::write(
            fork.join(MANIFEST_FILE),
            manifest("subsonic", "0.2.0", &["subsonic"]),
        )
        .unwrap();
        std::fs::write(fork.join(COMPONENT_FILE), b"not a component").unwrap();
        let registry = Registry::load(&bundled, Some(&dir), &[]).await;
        std::fs::remove_dir_all(&dir).ok();
        let serving = registry.for_service("subsonic").expect("subsonic served");
        assert_eq!(serving.origin(), Origin::Bundled);
    }

    #[tokio::test]
    async fn staging_directories_are_not_plugins_and_any_directory_name_is_found() {
        let dir =
            std::env::temp_dir().join(format!("kopuz-plugins-staging-{}", std::process::id()));
        for name in [".radio.installing", ".radio.removing", "my-radio-fork"] {
            let path = dir.join(name);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(
                path.join(MANIFEST_FILE),
                manifest("radio", "1.0.0", &["radio"]),
            )
            .unwrap();
            std::fs::write(path.join(COMPONENT_FILE), EMPTY_COMPONENT).unwrap();
        }
        let registry = Registry::load(&[], Some(&dir), &[]).await;
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(registry.plugins().len(), 1);
        assert_eq!(registry.dirs_of("radio"), [dir.join("my-radio-fork")]);
    }

    /// The smallest valid component: a header and nothing else.
    const EMPTY_COMPONENT: &[u8] = b"\0asm\x0d\x00\x01\x00";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kopuz-reload-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("radio")).unwrap();
        dir
    }

    fn write(dir: &Path, manifest: &str, wasm: &[u8]) {
        std::fs::write(dir.join("radio").join(MANIFEST_FILE), manifest).unwrap();
        std::fs::write(dir.join("radio").join(COMPONENT_FILE), wasm).unwrap();
    }

    fn radio(id: &str, version: &str, network: &[&str]) -> String {
        let network: Vec<String> = network.iter().map(|host| format!("\"{host}\"")).collect();
        format!(
            "{}[permissions]\nnetwork = [{}]\n[[service]]\nid = \"radio\"\n",
            manifest(id, version, &[]),
            network.join(", ")
        )
    }

    fn serving(registry: &Registry) -> &Arc<Plugin> {
        registry.for_service("radio").expect("radio served")
    }

    #[tokio::test]
    async fn unchanged_files_keep_the_loaded_plugin() {
        let dir = scratch("unchanged");
        write(&dir, &radio("radio", "1.0.0", &["a.test"]), EMPTY_COMPONENT);
        let first = Registry::load(&[], Some(&dir), &[]).await;
        let again = first.reload(&dir).await;
        std::fs::remove_dir_all(&dir).ok();
        assert!(Arc::ptr_eq(serving(&first), serving(&again)));
        assert!(first.same_as(&again));
    }

    #[tokio::test]
    async fn an_update_that_compiles_replaces_the_loaded_plugin() {
        let dir = scratch("update");
        write(&dir, &radio("radio", "1.0.0", &["a.test"]), EMPTY_COMPONENT);
        let first = Registry::load(&[], Some(&dir), &[]).await;
        write(&dir, &radio("radio", "1.0.1", &["a.test"]), EMPTY_COMPONENT);
        let next = first.reload(&dir).await;
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(serving(&next).manifest().version, "1.0.1");
        assert!(!first.same_as(&next));
    }

    #[tokio::test]
    async fn a_failed_load_keeps_the_old_module() {
        let dir = scratch("failed");
        write(&dir, &radio("radio", "1.0.0", &["a.test"]), EMPTY_COMPONENT);
        let first = Registry::load(&[], Some(&dir), &[]).await;

        write(
            &dir,
            &radio("radio", "1.0.1", &["a.test"]),
            b"not a component",
        );
        let broken_component = first.reload(&dir).await;
        write(&dir, "id = ", EMPTY_COMPONENT);
        let broken_manifest = first.reload(&dir).await;
        std::fs::remove_file(dir.join("radio").join(COMPONENT_FILE)).unwrap();
        let half_copied = first.reload(&dir).await;
        std::fs::remove_dir_all(&dir).ok();

        for kept in [&broken_component, &broken_manifest, &half_copied] {
            assert!(Arc::ptr_eq(serving(&first), serving(kept)));
            assert!(kept.pending().is_empty());
        }
    }

    #[tokio::test]
    async fn new_files_may_not_change_the_plugin_id() {
        let dir = scratch("id");
        write(&dir, &radio("radio", "1.0.0", &["a.test"]), EMPTY_COMPONENT);
        let first = Registry::load(&[], Some(&dir), &[]).await;
        write(&dir, &radio("other", "1.0.0", &["a.test"]), EMPTY_COMPONENT);
        let next = first.reload(&dir).await;
        std::fs::remove_dir_all(&dir).ok();
        assert!(Arc::ptr_eq(serving(&first), serving(&next)));
        assert_eq!(next.plugins().len(), 1);
    }

    #[tokio::test]
    async fn an_update_reaching_new_hosts_waits_until_they_are_allowed() {
        let dir = scratch("hosts");
        write(&dir, &radio("radio", "1.0.0", &["a.test"]), EMPTY_COMPONENT);
        let first = Registry::load(&[], Some(&dir), &[]).await;
        write(
            &dir,
            &radio("radio", "1.1.0", &["a.test", "B.test"]),
            EMPTY_COMPONENT,
        );
        let waiting = first.reload(&dir).await;
        let still_waiting = waiting.reload(&dir).await;
        std::fs::remove_dir_all(&dir).ok();

        assert!(Arc::ptr_eq(serving(&first), serving(&waiting)));
        let [pending] = waiting.pending() else {
            panic!("one update waits");
        };
        assert_eq!(pending.added_hosts, ["b.test"]);
        assert!(
            waiting.same_as(&still_waiting),
            "a waiting update is not compiled again"
        );

        assert!(waiting.approve("radio", &["a.test".into()]).is_none());
        let allowed = waiting
            .approve("radio", &["b.test".into(), "a.test".into()])
            .expect("the hosts shown are the hosts asked for");
        assert_eq!(serving(&allowed).manifest().version, "1.1.0");
        assert!(allowed.pending().is_empty());
    }

    #[tokio::test]
    async fn an_approved_update_in_another_directory_outlives_a_rescan_once_the_old_copy_goes() {
        let dir = scratch("other-dir");
        write(&dir, &radio("radio", "1.0.0", &["a.test"]), EMPTY_COMPONENT);
        let first = Registry::load(&[], Some(&dir), &[]).await;
        let update = dir.join("radio-update");
        std::fs::create_dir_all(&update).unwrap();
        std::fs::write(
            update.join(MANIFEST_FILE),
            radio("radio", "1.1.0", &["a.test", "b.test"]),
        )
        .unwrap();
        std::fs::write(update.join(COMPONENT_FILE), EMPTY_COMPONENT).unwrap();
        let waiting = first.reload(&dir).await;
        assert_eq!(serving(&waiting).manifest().version, "1.0.0");

        let allowed = waiting
            .approve("radio", &["a.test".into(), "b.test".into()])
            .expect("the hosts shown are the hosts asked for");
        let kept = allowed.dirs_of("radio");
        let stale: Vec<PathBuf> = waiting
            .dirs_of("radio")
            .into_iter()
            .filter(|copy| !kept.contains(copy))
            .collect();
        assert_eq!(kept, std::slice::from_ref(&update));
        assert_eq!(stale, [dir.join("radio")]);

        for copy in &stale {
            crate::market::uninstall(&dir, copy).unwrap();
        }
        let rescanned = allowed.reload(&dir).await;
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(serving(&rescanned).manifest().version, "1.1.0");
        assert!(rescanned.pending().is_empty());
    }

    #[tokio::test]
    async fn an_update_reaching_new_hosts_still_waits_after_a_restart() {
        let dir = scratch("restart");
        write(&dir, &radio("radio", "1.0.0", &["a.test"]), EMPTY_COMPONENT);
        Registry::load(&[], Some(&dir), &[])
            .await
            .save_approved(&dir)
            .unwrap();
        write(
            &dir,
            &radio("radio", "1.1.0", &["a.test", "b.test"]),
            EMPTY_COMPONENT,
        );

        let restarted = Registry::load(&[], Some(&dir), &[]).await;
        assert!(restarted.for_service("radio").is_none());
        let [pending] = restarted.pending() else {
            panic!("the update still waits");
        };
        assert_eq!(pending.added_hosts, ["b.test"]);
        assert!(
            restarted.reload(&dir).await.same_as(&restarted),
            "a rescan does not load it either"
        );

        restarted
            .approve("radio", &["a.test".into(), "b.test".into()])
            .expect("the hosts shown are the hosts asked for")
            .save_approved(&dir)
            .unwrap();
        let allowed = Registry::load(&[], Some(&dir), &[]).await;
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(serving(&allowed).manifest().version, "1.1.0");
        assert!(allowed.pending().is_empty());
    }

    #[tokio::test]
    async fn an_override_of_a_bundled_plugin_reaching_new_hosts_waits_after_a_restart() {
        let bundled = [Bundled {
            manifest: Box::leak(radio("radio", "1.0.0", &["a.test"]).into_boxed_str()),
            wasm: Some(EMPTY_COMPONENT),
        }];
        let dir = scratch("bundled");
        std::fs::remove_dir_all(dir.join("radio")).unwrap();
        Registry::load(&bundled, Some(&dir), &[])
            .await
            .save_approved(&dir)
            .unwrap();
        std::fs::create_dir_all(dir.join("radio")).unwrap();
        write(
            &dir,
            &radio("radio", "1.1.0", &["a.test", "b.test"]),
            EMPTY_COMPONENT,
        );

        let restarted = Registry::load(&bundled, Some(&dir), &[]).await;
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(serving(&restarted).origin(), Origin::Bundled);
        let [pending] = restarted.pending() else {
            panic!("the override waits");
        };
        assert_eq!(pending.added_hosts, ["b.test"]);
    }

    #[tokio::test]
    async fn a_waiting_plugin_whose_files_break_for_a_while_still_waits() {
        let dir = scratch("broken-wait");
        write(&dir, &radio("radio", "1.0.0", &["a.test"]), EMPTY_COMPONENT);
        Registry::load(&[], Some(&dir), &[])
            .await
            .save_approved(&dir)
            .unwrap();
        let widened = radio("radio", "1.1.0", &["a.test", "b.test"]);
        write(&dir, &widened, EMPTY_COMPONENT);
        let restarted = Registry::load(&[], Some(&dir), &[]).await;
        assert_eq!(restarted.pending().len(), 1);

        write(&dir, "id = ", EMPTY_COMPONENT);
        let broken = restarted.reload(&dir).await;
        let broken_again = broken.reload(&dir).await;
        write(&dir, &widened, EMPTY_COMPONENT);
        let fixed = broken_again.reload(&dir).await;
        std::fs::remove_dir_all(&dir).ok();
        assert!(fixed.for_service("radio").is_none());
        let [pending] = fixed.pending() else {
            panic!("the update still waits");
        };
        assert_eq!(pending.added_hosts, ["b.test"]);
    }

    #[tokio::test]
    async fn an_extension_replaces_the_bundled_plugin_and_an_installed_one_wins() {
        let bundled = [Bundled {
            manifest: Box::leak(radio("radio", "1.0.0", &["a.test"]).into_boxed_str()),
            wasm: Some(EMPTY_COMPONENT),
        }];
        let extensions = scratch("ext-layers");
        write(
            &extensions,
            &radio("radio", "2.0.0", &["a.test"]),
            EMPTY_COMPONENT,
        );
        let dir = scratch("ext-layers-installed");
        std::fs::remove_dir_all(dir.join("radio")).unwrap();

        let extended =
            Registry::load(&bundled, Some(&dir), std::slice::from_ref(&extensions)).await;
        assert_eq!(serving(&extended).origin(), Origin::Extension);
        assert_eq!(serving(&extended).manifest().version, "2.0.0");
        assert!(extended.dirs_of("radio").is_empty());

        std::fs::create_dir_all(dir.join("radio")).unwrap();
        write(&dir, &radio("radio", "3.0.0", &["a.test"]), EMPTY_COMPONENT);
        let installed =
            Registry::load(&bundled, Some(&dir), std::slice::from_ref(&extensions)).await;
        assert_eq!(serving(&installed).origin(), Origin::Installed);
        assert_eq!(installed.plugins().len(), 1);
        assert_eq!(installed.dirs_of("radio"), [dir.join("radio")]);
        assert!(installed.from_extension("radio"));

        std::fs::remove_dir_all(dir.join("radio")).unwrap();
        let removed = installed.reload(&dir).await;
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&extensions).ok();
        assert_eq!(serving(&removed).origin(), Origin::Extension);
        assert!(removed.pending().is_empty());
    }

    #[tokio::test]
    async fn every_extension_root_is_read() {
        let first = scratch("ext-root-one");
        write(
            &first,
            &radio("radio", "1.0.0", &["a.test"]),
            EMPTY_COMPONENT,
        );
        let second = scratch("ext-root-two");
        write(
            &second,
            &radio("other", "1.0.0", &["b.test"]),
            EMPTY_COMPONENT,
        );
        let registry = Registry::load(&[], None, &[first.clone(), second.clone()]).await;
        std::fs::remove_dir_all(&first).ok();
        std::fs::remove_dir_all(&second).ok();
        let mut ids: Vec<&str> = registry
            .plugins()
            .iter()
            .map(|plugin| plugin.id())
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, ["other", "radio"]);
    }

    #[tokio::test]
    async fn a_missing_extension_directory_provides_nothing() {
        let dir = scratch("ext-missing");
        write(&dir, &radio("radio", "1.0.0", &["a.test"]), EMPTY_COMPONENT);
        let missing = dir.join("no-such-extensions");
        let registry = Registry::load(&[], Some(&dir), std::slice::from_ref(&missing)).await;
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(serving(&registry).origin(), Origin::Installed);
        assert!(!registry.from_extension("radio"));
        assert!(registry.pending().is_empty());
    }

    #[tokio::test]
    async fn of_two_extension_copies_waiting_for_hosts_the_later_root_waits() {
        let dir = scratch("ext-two-waiting");
        write(&dir, &radio("radio", "1.0.0", &["a.test"]), EMPTY_COMPONENT);
        Registry::load(&[], Some(&dir), &[])
            .await
            .save_approved(&dir)
            .unwrap();
        std::fs::remove_dir_all(dir.join("radio")).unwrap();
        let lower = scratch("ext-two-waiting-lower");
        write(
            &lower,
            &radio("radio", "1.1.0", &["a.test", "b.test"]),
            EMPTY_COMPONENT,
        );
        let upper = scratch("ext-two-waiting-upper");
        write(
            &upper,
            &radio("radio", "1.2.0", &["a.test", "c.test"]),
            EMPTY_COMPONENT,
        );

        let registry = Registry::load(&[], Some(&dir), &[lower.clone(), upper.clone()]).await;
        for root in [&dir, &lower, &upper] {
            std::fs::remove_dir_all(root).ok();
        }
        let [pending] = registry.pending() else {
            panic!("one copy waits");
        };
        assert_eq!(pending.plugin.manifest().version, "1.2.0");
        assert_eq!(pending.added_hosts, ["c.test"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_extension_root_that_cannot_be_read_keeps_every_approval() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("ext-unreadable");
        write(&dir, &radio("radio", "1.0.0", &["a.test"]), EMPTY_COMPONENT);
        Registry::load(&[], Some(&dir), &[])
            .await
            .save_approved(&dir)
            .unwrap();
        std::fs::remove_dir_all(dir.join("radio")).unwrap();
        let locked = scratch("ext-unreadable-root");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let readable = std::fs::read_dir(&locked).is_ok();

        let registry = Registry::load(&[], Some(&dir), std::slice::from_ref(&locked)).await;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&locked).ok();
        std::fs::remove_dir_all(&dir).ok();
        if readable {
            return;
        }
        assert!(
            registry.approved.contains_key("radio"),
            "which plugin the root holds is unknown, so its record stays"
        );
    }

    #[tokio::test]
    async fn an_extension_reaching_hosts_not_agreed_to_waits_until_they_are_allowed() {
        let dir = scratch("ext-hosts");
        write(&dir, &radio("radio", "1.0.0", &["a.test"]), EMPTY_COMPONENT);
        Registry::load(&[], Some(&dir), &[])
            .await
            .save_approved(&dir)
            .unwrap();
        std::fs::remove_dir_all(dir.join("radio")).unwrap();
        let extensions = scratch("ext-hosts-mounted");
        write(
            &extensions,
            &radio("radio", "1.1.0", &["a.test", "b.test"]),
            EMPTY_COMPONENT,
        );

        let restarted = Registry::load(&[], Some(&dir), std::slice::from_ref(&extensions)).await;
        assert!(restarted.for_service("radio").is_none());
        let [pending] = restarted.pending() else {
            panic!("the extension waits");
        };
        assert_eq!(pending.added_hosts, ["b.test"]);
        assert!(restarted.reload(&dir).await.same_as(&restarted));
        assert!(restarted.dirs_of("radio").is_empty());

        assert!(restarted.approve("radio", &["a.test".into()]).is_none());
        let allowed = restarted
            .approve("radio", &["a.test".into(), "b.test".into()])
            .expect("the hosts shown are the hosts asked for");
        allowed.save_approved(&dir).unwrap();
        assert_eq!(serving(&allowed).origin(), Origin::Extension);
        assert!(allowed.pending().is_empty());
        assert!(allowed.reload(&dir).await.same_as(&allowed));

        let again = Registry::load(&[], Some(&dir), std::slice::from_ref(&extensions)).await;
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&extensions).ok();
        assert_eq!(serving(&again).manifest().version, "1.1.0");
        assert!(again.pending().is_empty());
    }
}
