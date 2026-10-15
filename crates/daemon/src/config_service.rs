//! ConfigService: authority over the running `Settings`, loaded from and saved to the settings files; it applies typed edits one by one, refuses the daemon-owned and layer-locked keys, saves each change at once rather than debounced, and takes in what changes on disk.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use api::{ApiError, ConfigView, EditOutcome, EditReport, SettingLock};
use config::store::FileLayers;
use config::{SettingKey, SettingsEdit};
use tokio::sync::{RwLock, RwLockWriteGuard};

use crate::session::SessionHandle;

/// How often the settings files are checked for a change made outside the daemon.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

pub struct ConfigService {
    db: db::Db,
    settings_path: PathBuf,
    current: RwLock<Held>,
    session: OnceLock<SessionHandle>,
}

/// The running settings, the layers they were read from with what those pin, and the revision of their last change.
struct Held {
    settings: config::Settings,
    layers: FileLayers,
    locks: Vec<SettingLock>,
    revision: u64,
}

/// The settings every layer at `settings_path` adds up to.
pub fn load_settings(settings_path: &Path) -> Result<config::Settings, ApiError> {
    settings_of(&FileLayers::read(settings_path))
}

fn settings_of(layers: &FileLayers) -> Result<config::Settings, ApiError> {
    layers
        .settings()
        .map_err(|error| ApiError::internal(format!("settings load failed: {error}")))
}

impl ConfigService {
    pub fn new(db: db::Db, settings_path: PathBuf, current: config::Settings) -> Self {
        // Seeded from the clock so a restarted daemon never hands out a revision a client already passed.
        let revision = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or_default();
        let layers = FileLayers::read(&settings_path);
        Self {
            db,
            settings_path,
            current: RwLock::new(Held {
                settings: current,
                locks: layers.locks(),
                layers,
                revision,
            }),
            session: OnceLock::new(),
        }
    }

    /// Re-read the layers and, only when the settings or what is pinned differ from what is held, publish them as a new revision; answers whether anything changed.
    #[tracing::instrument(name = "config.reload", skip_all)]
    pub async fn reload(&self) -> Result<bool, ApiError> {
        let mut held = self.current.write().await;
        self.take_in_disk(&mut held)
    }

    /// Bring what is held up to the files as they are now; every write decides on this rather than the last poll, so a pin that landed since is honoured.
    fn take_in_disk(&self, held: &mut RwLockWriteGuard<'_, Held>) -> Result<bool, ApiError> {
        let layers = held.layers.reread();
        let settings = settings_of(&layers)?;
        let locks = layers.locks();
        held.layers = layers;
        if settings == held.settings && locks == held.locks {
            return Ok(false);
        }
        let changed = held.settings.changed_keys(&settings);
        tracing::info!(
            keys = %changed.iter().map(SettingKey::name).collect::<Vec<_>>().join(", "),
            "settings changed on disk"
        );
        held.settings = settings;
        held.locks = locks;
        self.publish(held, changed);
        Ok(true)
    }

    /// Reload whenever a settings file appears, goes or is rewritten, until the session ends.
    pub fn spawn_file_watch(
        self: &Arc<Self>,
        mut settings: tokio::sync::watch::Receiver<config::Settings>,
    ) {
        let service = self.clone();
        tokio::spawn(async move {
            let mut seen = file_stamps(&service.settings_path);
            let mut ticks = tokio::time::interval(POLL_INTERVAL);
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = ticks.tick() => {}
                    changed = settings.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        continue;
                    }
                }
                let stamps = file_stamps(&service.settings_path);
                if stamps == seen {
                    continue;
                }
                seen = stamps;
                if let Err(error) = service.reload().await {
                    tracing::warn!(%error, "re-reading the settings files failed");
                }
            }
        });
    }

    /// Hand every later change to `session`, which then always holds what was saved last.
    pub fn attach_session(&self, session: SessionHandle) {
        let _ = self.session.set(session);
    }

    /// Mark a saved change and pass it on while the write guard is still held, so the session sees changes in the order they were saved.
    fn publish(&self, held: &mut RwLockWriteGuard<'_, Held>, changed: Vec<SettingKey>) {
        held.revision += 1;
        let Some(session) = self.session.get() else {
            return;
        };
        session.set_settings(held.settings.clone(), changed, held.revision);
    }

    /// Write `keys` of `settings` into the settings file and take the layers in again, so what is held matches the disk.
    #[tracing::instrument(name = "config.save", skip_all)]
    async fn save(
        &self,
        held: &mut RwLockWriteGuard<'_, Held>,
        settings: &config::Settings,
        keys: &[SettingKey],
    ) -> Result<(), ApiError> {
        let failed = |error: String| ApiError::internal(format!("config save failed: {error}"));
        self.db
            .ensure_app_state()
            .await
            .map_err(|error| failed(error.to_string()))?;
        held.layers
            .save(settings, keys)
            .map_err(|error| failed(format!("{}: {error}", self.settings_path.display())))?;
        held.layers = held.layers.reread();
        held.locks = held.layers.locks();
        Ok(())
    }

    /// Refuse a write that a settings layer has pinned, naming each key the caller is about to change and what pins it.
    pub async fn ensure_unlocked(&self, keys: &[SettingKey]) -> Result<(), ApiError> {
        let mut held = self.current.write().await;
        self.take_in_disk(&mut held)?;
        let refused: Vec<String> = keys
            .iter()
            .filter_map(|key| {
                let lock = held.locks.iter().find(|lock| lock.key == *key)?;
                Some(format!("{} (by {})", key.name(), lock.by))
            })
            .collect();
        if refused.is_empty() {
            Ok(())
        } else {
            Err(ApiError::invalid_input(format!(
                "these settings are pinned: {}",
                refused.join(", ")
            ))
            .because(api::ErrorReason::SettingLocked))
        }
    }

    /// Change settings from inside the daemon, daemon-owned keys included, persist them, pass them to the session under `keys`, and hand back the result.
    pub async fn mutate_state(
        &self,
        keys: &[SettingKey],
        mutate: impl FnOnce(&mut config::Settings) + Send,
    ) -> Result<config::Settings, ApiError> {
        let mut held = self.current.write().await;
        self.take_in_disk(&mut held)?;
        let mut next = held.settings.clone();
        mutate(&mut next);
        if next == held.settings {
            return Ok(next);
        }
        let changed = held.settings.changed_keys(&next);
        self.save(&mut held, &next, &changed).await?;
        held.settings = next;
        self.publish(&mut held, keys.to_vec());
        Ok(held.settings.clone())
    }

    pub async fn snapshot(&self) -> config::Settings {
        self.current.read().await.settings.clone()
    }

    pub async fn view(&self) -> Result<ConfigView, ApiError> {
        Ok(view_of(&*self.current.read().await))
    }

    /// Answer each edit in order: a daemon-owned key names the API that sets it, a pinned one is locked, and the rest apply unless their value breaks a rule; what applied is saved and published once.
    pub async fn edit(&self, edits: Vec<SettingsEdit>) -> Result<EditReport, ApiError> {
        let mut held = self.current.write().await;
        self.take_in_disk(&mut held)?;
        let locked: Vec<SettingKey> = held.locks.iter().map(|lock| lock.key).collect();
        let mut next = held.settings.clone();
        let mut changed: Vec<SettingKey> = Vec::new();
        let outcomes = edits
            .into_iter()
            .map(|edit| {
                let key = SettingKey::of(&edit);
                if let Some(api) = key.owner() {
                    return EditOutcome::Invalid {
                        reason: format!("{} is the daemon's; set it through {api}", key.name()),
                    };
                }
                if locked.contains(&key) {
                    return EditOutcome::Locked;
                }
                let before = next.clone();
                match next.try_apply(edit) {
                    Err(reason) => EditOutcome::Invalid { reason },
                    Ok(()) => {
                        if next != before && !changed.contains(&key) {
                            changed.push(key);
                        }
                        EditOutcome::Applied
                    }
                }
            })
            .collect();
        if next != held.settings {
            self.save(&mut held, &next, &changed).await?;
            held.settings = next;
            self.publish(&mut held, changed);
        }
        Ok(EditReport {
            outcomes,
            view: view_of(&held),
        })
    }
}

fn view_of(held: &Held) -> ConfigView {
    ConfigView {
        settings: held.settings.clone(),
        locks: held.locks.clone(),
        revision: held.revision,
    }
}

/// What the poller compares per layer file: the link's and the target's modification time, size, resolved path and writability, so an hjem switch between two store files (whose times are all the epoch) still shows.
type FileStamp = (
    PathBuf,
    Option<SystemTime>,
    Option<SystemTime>,
    Option<u64>,
    Option<PathBuf>,
    bool,
);

fn file_stamps(settings_path: &Path) -> Vec<FileStamp> {
    let mut paths = vec![
        settings_path.to_path_buf(),
        config::store::local_path_for(settings_path),
    ];
    if let Ok(entries) = std::fs::read_dir(config::store::dropin_dir_for(settings_path)) {
        let mut dropins: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .collect();
        dropins.sort();
        paths.extend(dropins);
    }
    paths
        .into_iter()
        .map(|path| {
            let link = std::fs::symlink_metadata(&path).ok();
            let target = std::fs::metadata(&path).ok();
            (
                path.clone(),
                link.and_then(|meta| meta.modified().ok()),
                target.as_ref().and_then(|meta| meta.modified().ok()),
                target.as_ref().map(std::fs::Metadata::len),
                std::fs::canonicalize(&path).ok(),
                target.is_some_and(|meta| meta.permissions().readonly()),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{
        BackgroundEdit, LibraryEdit, PlaybackEdit, PlaybackKey, SpotifyEdit, UiEdit, UiKey,
    };

    async fn service(dir: &tempfile::TempDir) -> ConfigService {
        let database = db::init(&dir.path().join("cfg.db")).await.expect("db");
        ConfigService::new(
            database,
            dir.path().join("settings.toml"),
            config::Settings::default(),
        )
    }

    #[cfg(unix)]
    fn pin(path: &std::path::Path, toml: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, toml).expect("write settings");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444)).expect("chmod");
    }

    fn crossfade(seconds: u8) -> SettingsEdit {
        SettingsEdit::Playback(PlaybackEdit::CrossfadeSeconds(seconds))
    }

    fn theme(name: &str) -> SettingsEdit {
        SettingsEdit::Ui(UiEdit::Theme(name.into()))
    }

    #[tokio::test]
    async fn the_view_is_every_setting_and_carries_no_credential() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("cfg.db")).await.expect("db");
        database
            .set_integration_credential("lastfm_session_key", "secret")
            .await
            .expect("seed credential");
        let mut held = config::Settings::default();
        held.spotify.spotify_browser = Some("brave".into());
        held.downloads.downloader_output_dir = "/downloads".into();
        let service = ConfigService::new(database, dir.path().join("settings.toml"), held.clone());

        let view = service.view().await.expect("view");
        assert_eq!(view.settings, held, "nothing is stripped from the view");
        assert!(view.locks.is_empty());
        let rendered = serde_json::to_string(&view.settings).expect("json");
        assert!(!rendered.contains("secret"), "{rendered}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn each_edit_in_a_batch_gets_its_own_outcome() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = dir.path().join("settings.toml");
        pin(&settings, "theme = \"pinned\"\n");
        let database = db::init(&dir.path().join("cfg.db")).await.expect("db");
        let loaded = load_settings(&settings).expect("load");
        let service = ConfigService::new(database.clone(), settings.clone(), loaded);

        let report = service
            .edit(vec![
                crossfade(6),
                theme("other"),
                SettingsEdit::Background(BackgroundEdit::CoverArtDarkening(99)),
                SettingsEdit::Spotify(SpotifyEdit::SpotifyPreferActiveDevice(false)),
                SettingsEdit::Library(LibraryEdit::LocalSources(Vec::new())),
                SettingsEdit::Ui(UiEdit::ReduceAnimations(true)),
            ])
            .await
            .expect("edit");

        assert_eq!(report.outcomes.len(), 6);
        assert_eq!(report.outcomes[0], EditOutcome::Applied);
        assert_eq!(report.outcomes[1], EditOutcome::Locked);
        assert!(
            matches!(&report.outcomes[2], EditOutcome::Invalid { reason } if reason.contains("cover_art_darkening")),
            "{:?}",
            report.outcomes[2]
        );
        for owned in &report.outcomes[3..5] {
            assert!(
                matches!(owned, EditOutcome::Invalid { reason } if reason.contains("set it through")),
                "{owned:?}"
            );
        }
        assert_eq!(report.outcomes[5], EditOutcome::Applied);
        assert_eq!(
            report.view.locks,
            vec![SettingLock {
                key: SettingKey::Ui(UiKey::Theme),
                by: api::LockSource::Managed(settings.clone()),
            }]
        );

        let held = &report.view.settings;
        assert_eq!(held.playback.crossfade_seconds, 6);
        assert!(held.ui.reduce_animations);
        assert_eq!(held.ui.theme, "pinned");
        assert_eq!(
            held.background.cover_art_darkening,
            config::Background::default().cover_art_darkening
        );
        assert!(held.spotify.spotify_prefer_active_device);
        let reloaded = load_settings(&settings).expect("reload");
        assert_eq!(reloaded.playback.crossfade_seconds, 6);
    }

    #[tokio::test]
    async fn an_edit_that_changes_nothing_neither_saves_nor_publishes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = service(&dir).await;
        let before = service.view().await.expect("view");

        let same = before.settings.playback.crossfade_seconds;
        let report = service
            .edit(vec![crossfade(same), crossfade(40)])
            .await
            .expect("no-op edit");
        assert_eq!(report.outcomes[0], EditOutcome::Applied);
        assert!(matches!(report.outcomes[1], EditOutcome::Invalid { .. }));
        assert_eq!(report.view.revision, before.revision);
        assert!(!dir.path().join("settings.toml").exists(), "nothing saved");

        let report = service.edit(vec![crossfade(4)]).await.expect("edit");
        assert_eq!(report.view.revision, before.revision + 1);
    }

    /// An edit names only its own key, so a client holding an old view cannot roll back what another writer changed.
    #[tokio::test]
    async fn a_stale_client_cannot_revert_a_key_it_did_not_send() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = service(&dir).await;
        let stale = service.view().await.expect("view");

        service
            .edit(vec![crossfade(9)])
            .await
            .expect("another writer");
        service
            .mutate_state(
                &[SettingKey::Playback(PlaybackKey::ReplayGain)],
                |settings| settings.playback.replay_gain.preamp_db = 3.0,
            )
            .await
            .expect("the daemon's own write");

        let report = service.edit(vec![theme("nord")]).await.expect("stale edit");
        assert_eq!(report.view.settings.ui.theme, "nord");
        assert_eq!(report.view.settings.playback.crossfade_seconds, 9);
        assert_eq!(report.view.settings.playback.replay_gain.preamp_db, 3.0);
        assert!(report.view.revision > stale.revision + 1);
    }

    #[tokio::test]
    async fn collection_ops_from_two_writers_both_land() {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = service(&dir).await;
        let put = |name: &str| {
            SettingsEdit::Ui(UiEdit::CustomThemes(config::CustomThemeOp::Put {
                name: name.into(),
                theme: config::CustomTheme {
                    name: name.into(),
                    vars: Default::default(),
                },
            }))
        };
        service.edit(vec![put("a")]).await.expect("first writer");
        let report = service.edit(vec![put("b")]).await.expect("second writer");
        let mut names: Vec<_> = report
            .view
            .settings
            .ui
            .custom_themes
            .keys()
            .cloned()
            .collect();
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);

        let report = service
            .edit(vec![SettingsEdit::Ui(UiEdit::CustomThemes(
                config::CustomThemeOp::Delete { name: "a".into() },
            ))])
            .await
            .expect("delete one");
        assert_eq!(
            report
                .view
                .settings
                .ui
                .custom_themes
                .keys()
                .collect::<Vec<_>>(),
            vec!["b"]
        );
    }

    /// An edit saves the state row too, and must not reset the playback state the session persists beside it.
    #[tokio::test]
    async fn an_edit_leaves_the_playback_state_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("vol.db")).await.expect("db");
        let service = ConfigService::new(
            database.clone(),
            dir.path().join("settings.toml"),
            config::Settings::default(),
        );
        database
            .set_playback_state(0.2, true)
            .await
            .expect("playback state");
        service.edit(vec![theme("nord")]).await.expect("edit");
        assert_eq!(database.playback_state().await.expect("load"), (0.2, true));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_pinned_key_is_refused_to_the_daemon_too() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = dir.path().join("settings.toml");
        pin(&settings, "local_sources = []\n");
        let database = db::init(&dir.path().join("cfg.db")).await.expect("db");
        let service = ConfigService::new(database, settings, config::Settings::default());

        let local = SettingKey::Library(config::LibraryKey::LocalSources);
        let err = service.ensure_unlocked(&[local]).await.expect_err("pinned");
        assert_eq!(err.code, api::ErrorCode::InvalidInput);
        assert!(err.message.contains("local_sources"), "{}", err.message);
        service
            .ensure_unlocked(&[SettingKey::Playback(PlaybackKey::CrossfadeSeconds)])
            .await
            .expect("a neighbour is free");
    }

    #[tokio::test]
    async fn a_refusal_names_the_drop_in_that_pins_the_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dropins = dir.path().join("settings.d");
        std::fs::create_dir_all(&dropins).expect("drop-in dir");
        let dropin = dropins.join("10-folders.toml");
        std::fs::write(&dropin, "local_sources = []\n").expect("drop-in");
        let database = db::init(&dir.path().join("cfg.db")).await.expect("db");
        let service = ConfigService::new(
            database,
            dir.path().join("settings.toml"),
            config::Settings::default(),
        );

        let local = SettingKey::Library(config::LibraryKey::LocalSources);
        let err = service.ensure_unlocked(&[local]).await.expect_err("pinned");
        let named = format!("local_sources (by the drop-in {})", dropin.display());
        assert!(err.message.contains(&named), "{}", err.message);
        assert!(!err.message.contains("managed"), "{}", err.message);
    }

    /// An install loaded the way boot loads it, with the sources service storing its device id in the state row first.
    async fn loaded(dir: &tempfile::TempDir) -> (ConfigService, PathBuf) {
        let path = dir.path().join("settings.toml");
        let database = db::init(&dir.path().join("cfg.db")).await.expect("db");
        crate::sources::load(&database).await.expect("sources");
        let settings = load_settings(&path).expect("load");
        (ConfigService::new(database, path.clone(), settings), path)
    }

    #[tokio::test]
    async fn a_fresh_install_keeps_its_default_folder_through_a_save_and_a_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let defaults = config::Settings::default().library.local_sources;
        assert!(
            defaults.iter().all(|source| !source.directories.is_empty()),
            "{defaults:?}"
        );
        let (service, path) = loaded(&dir).await;
        assert_eq!(service.snapshot().await.library.local_sources, defaults);
        assert_eq!(
            load_settings(&path).expect("restart").library.local_sources,
            defaults,
            "a restart before any save"
        );

        service.edit(vec![theme("nord")]).await.expect("edit");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "theme = \"nord\"\n",
            "the first save is as sparse as any other"
        );
        assert!(!service.reload().await.expect("reload"));
        let restarted = load_settings(&path).expect("restart");
        assert_eq!(restarted.library.local_sources, defaults);
        assert_eq!(restarted.ui.theme, "nord");
    }

    /// The poller sees the daemon's own writes too, so taking them in again must neither publish nor move the revision.
    #[tokio::test]
    async fn reloading_the_daemons_own_save_changes_nothing() {
        for booted in [false, true] {
            let dir = tempfile::tempdir().expect("tempdir");
            let service = match booted {
                true => loaded(&dir).await.0,
                false => service(&dir).await,
            };
            let saved = service
                .edit(vec![theme("nord"), crossfade(4)])
                .await
                .expect("edit")
                .view;
            assert!(!service.reload().await.expect("reload"), "{booted}");
            assert_eq!(service.view().await.expect("view"), saved);
        }
    }

    #[tokio::test]
    async fn a_save_writes_only_the_keys_that_applied() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (service, path) = loaded(&dir).await;
        std::fs::write(&path, "# mine\nlanguage = \"tr\"\nhero_height = 640\n").expect("write");
        assert!(service.reload().await.expect("reload"));

        service.edit(vec![theme("nord")]).await.expect("edit");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "# mine\nlanguage = \"tr\"\nhero_height = 640\ntheme = \"nord\"\n"
        );
    }

    #[tokio::test]
    async fn a_hand_edit_and_a_new_drop_in_reach_the_view_on_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (service, path) = loaded(&dir).await;
        let before = service.view().await.expect("view");
        std::fs::write(&path, "crossfade_seconds = 7\n").expect("write");
        let dropins = config::store::dropin_dir_for(&path);
        std::fs::create_dir_all(&dropins).expect("mkdir");
        let dropin = dropins.join("10-theme.toml");
        std::fs::write(&dropin, "theme = \"dracula\"\n").expect("write");

        assert!(service.reload().await.expect("reload"));
        let view = service.view().await.expect("view");
        assert_eq!(view.settings.playback.crossfade_seconds, 7);
        assert_eq!(view.settings.ui.theme, "dracula");
        assert_eq!(
            view.locks,
            vec![SettingLock {
                key: SettingKey::Ui(UiKey::Theme),
                by: api::LockSource::DropIn(dropin),
            }]
        );
        assert!(view.revision > before.revision);
        let report = service.edit(vec![theme("nord")]).await.expect("edit");
        assert_eq!(report.outcomes, vec![EditOutcome::Locked]);

        std::fs::remove_dir_all(&dropins).expect("unpin");
        assert!(service.reload().await.expect("reload"));
        let view = service.view().await.expect("view");
        assert!(view.locks.is_empty());
        assert_eq!(view.settings.ui.theme, config::Settings::default().ui.theme);
        assert!(
            !service.reload().await.expect("reload"),
            "nothing moved since"
        );
    }

    /// A pin that lands between two polls must still refuse the next edit, not be found by the poll after the write.
    #[tokio::test]
    async fn a_drop_in_written_since_the_last_poll_locks_the_next_edit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (service, path) = loaded(&dir).await;
        std::fs::write(&path, "crossfade_seconds = 3\n").expect("write");
        assert!(service.reload().await.expect("reload"));
        let dropins = config::store::dropin_dir_for(&path);
        std::fs::create_dir_all(&dropins).expect("mkdir");
        std::fs::write(dropins.join("40-race.toml"), "crossfade_seconds = 1\n").expect("pin");

        let report = service.edit(vec![crossfade(9)]).await.expect("edit");

        assert_eq!(report.outcomes, vec![EditOutcome::Locked]);
        assert_eq!(report.view.settings.playback.crossfade_seconds, 1);
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "crossfade_seconds = 3\n"
        );
        let crossfade_key = SettingKey::Playback(PlaybackKey::CrossfadeSeconds);
        service
            .ensure_unlocked(&[crossfade_key])
            .await
            .expect_err("pinned");
    }

    #[tokio::test]
    async fn a_torn_settings_file_keeps_what_was_held() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (service, path) = loaded(&dir).await;
        std::fs::write(&path, "theme = \"nord\"\n").expect("write");
        assert!(service.reload().await.expect("reload"));

        std::fs::write(&path, "theme = \"dra").expect("torn write");
        assert!(!service.reload().await.expect("reload"));
        assert_eq!(service.snapshot().await.ui.theme, "nord");
    }
}
