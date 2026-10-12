//! ConfigService: authority over the running `Settings`, loaded from and saved to the settings files; it applies typed edits one by one, refuses the daemon-owned and layer-locked keys, and saves each change at once rather than debounced.

use std::path::PathBuf;
use std::sync::OnceLock;

use api::{ApiError, ConfigView, EditOutcome, EditReport};
use config::{SettingKey, SettingsEdit};
use tokio::sync::{RwLock, RwLockWriteGuard};

use crate::session::SessionHandle;

pub struct ConfigService {
    db: db::Db,
    settings_path: PathBuf,
    current: RwLock<Held>,
    session: OnceLock<SessionHandle>,
}

/// The running settings and the revision of their last saved change.
struct Held {
    settings: config::Settings,
    revision: u64,
}

/// The settings every layer at `settings_path` adds up to.
pub fn load_settings(settings_path: &std::path::Path) -> Result<config::Settings, ApiError> {
    let layers = config::store::FileLayers::read(settings_path);
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
        Self {
            db,
            settings_path,
            current: RwLock::new(Held {
                settings: current,
                revision,
            }),
            session: OnceLock::new(),
        }
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

    #[tracing::instrument(name = "config.save", skip_all)]
    async fn save(&self, settings: &config::Settings) -> Result<(), ApiError> {
        let failed = |error: String| ApiError::internal(format!("config save failed: {error}"));
        self.db
            .ensure_app_state()
            .await
            .map_err(|error| failed(error.to_string()))?;
        config::store::save_settings(&self.settings_path, settings)
            .map_err(|error| failed(format!("{}: {error}", self.settings_path.display())))?;
        Ok(())
    }

    /// Every key a managed settings layer pins; a pinned name that is no setting is ignored.
    fn locked_keys(&self) -> Vec<SettingKey> {
        config::store::FileLayers::read(&self.settings_path)
            .locked_keys
            .iter()
            .filter_map(|name| SettingKey::named(name))
            .collect()
    }

    /// Refuse a write that a managed settings file has pinned, naming the keys the caller is about to change.
    pub fn ensure_unlocked(&self, keys: &[SettingKey]) -> Result<(), ApiError> {
        let locked = self.locked_keys();
        let refused: Vec<&str> = keys
            .iter()
            .filter(|key| locked.contains(key))
            .map(SettingKey::name)
            .collect();
        if refused.is_empty() {
            Ok(())
        } else {
            Err(ApiError::invalid_input(format!(
                "these settings are pinned by a managed file: {}",
                refused.join(", ")
            )))
        }
    }

    /// Change settings from inside the daemon, daemon-owned keys included, persist them, pass them to the session under `keys`, and hand back the result.
    pub async fn mutate_state(
        &self,
        keys: &[SettingKey],
        mutate: impl FnOnce(&mut config::Settings) + Send,
    ) -> Result<config::Settings, ApiError> {
        let mut held = self.current.write().await;
        let mut next = held.settings.clone();
        mutate(&mut next);
        if next == held.settings {
            return Ok(next);
        }
        self.save(&next).await?;
        held.settings = next;
        self.publish(&mut held, keys.to_vec());
        Ok(held.settings.clone())
    }

    pub async fn snapshot(&self) -> config::Settings {
        self.current.read().await.settings.clone()
    }

    pub async fn view(&self) -> Result<ConfigView, ApiError> {
        let held = self.current.read().await;
        Ok(view_of(&held, self.locked_keys()))
    }

    /// Answer each edit in order: a daemon-owned key names the API that sets it, a pinned one is locked, and the rest apply unless their value breaks a rule; what applied is saved and published once.
    pub async fn edit(&self, edits: Vec<SettingsEdit>) -> Result<EditReport, ApiError> {
        let locked = self.locked_keys();
        let mut held = self.current.write().await;
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
            self.save(&next).await?;
            held.settings = next;
            self.publish(&mut held, changed);
        }
        Ok(EditReport {
            outcomes,
            view: view_of(&held, locked),
        })
    }
}

fn view_of(held: &Held, locked: Vec<SettingKey>) -> ConfigView {
    ConfigView {
        settings: held.settings.clone(),
        locked,
        revision: held.revision,
    }
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
        assert!(view.locked.is_empty());
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
        assert!(report.view.locked.contains(&SettingKey::Ui(UiKey::Theme)));

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
        let err = service.ensure_unlocked(&[local]).expect_err("pinned");
        assert_eq!(err.code, api::ErrorCode::InvalidInput);
        assert!(err.message.contains("local_sources"), "{}", err.message);
        service
            .ensure_unlocked(&[SettingKey::Playback(PlaybackKey::CrossfadeSeconds)])
            .expect("a neighbour is free");
    }
}
