//! The settings files (issue #530): `<stem>.local.toml` < `settings.toml` < `<stem>.d/*.toml` drop-ins < `KOPUZ_CONFIG_*` env, merged as TOML tables into [`Settings`], with the keys an unwritable layer pins reported as locked.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;
use toml::{Table, Value as TomlValue};

use crate::Settings;

/// Env prefix for ad-hoc per-key overrides: `KOPUZ_CONFIG_THEME=nord`,
/// nested fields with `__` (`KOPUZ_CONFIG_EQUALIZER__ENABLED=true`).
pub const ENV_PREFIX: &str = "KOPUZ_CONFIG_";

/// `KOPUZ_CONFIG_PATH` relocates the settings file itself (it is not a field
/// override, so the env layer skips it).
pub const ENV_PATH_OVERRIDE: &str = "KOPUZ_CONFIG_PATH";

const NIX_STORE_PREFIX: &str = "/nix/store";

/// Folds the retired `music_directory` key into the default folder source and makes sure that source exists.
fn fold_legacy_directories(table: &mut Table, force: bool) {
    let legacy = table.remove("music_directory").and_then(|raw| match raw {
        TomlValue::String(path) => Some(vec![TomlValue::String(path)]),
        TomlValue::Array(paths) if paths.iter().all(TomlValue::is_str) => Some(paths),
        _ => {
            tracing::warn!("ignoring a music_directory that is neither a path nor a list of paths");
            None
        }
    });
    let sources = table
        .entry("local_sources")
        .or_insert_with(|| TomlValue::Array(Vec::new()));
    let Some(sources) = sources.as_array_mut() else {
        return;
    };
    let default = sources.iter().position(|source| {
        source.get("id").and_then(TomlValue::as_str) == Some(crate::DEFAULT_LOCAL_ID)
    });
    match (default, legacy) {
        (Some(index), Some(dirs)) => {
            let Some(source) = sources[index].as_table_mut() else {
                return;
            };
            let empty = source
                .get("directories")
                .and_then(TomlValue::as_array)
                .is_none_or(Vec::is_empty);
            if force || empty {
                source.insert("directories".to_owned(), TomlValue::Array(dirs));
            }
        }
        (None, dirs) => sources.insert(
            0,
            TomlValue::Table(Table::from_iter([
                (
                    "id".to_owned(),
                    TomlValue::String(crate::DEFAULT_LOCAL_ID.to_owned()),
                ),
                (
                    "name".to_owned(),
                    TomlValue::String(crate::DEFAULT_LOCAL_NAME.to_owned()),
                ),
                (
                    "directories".to_owned(),
                    TomlValue::Array(dirs.unwrap_or_else(|| {
                        vec![TomlValue::String(
                            crate::default_music_directory()
                                .to_string_lossy()
                                .into_owned(),
                        )]
                    })),
                ),
            ])),
        ),
        (Some(_), None) => {}
    }
}

/// The settings file for an app whose database lives in `db_dir`:
/// `KOPUZ_CONFIG_PATH` override, else `<db_dir>/settings.toml` (release) or
/// `settings-debug.toml` (debug builds, mirroring the `kopuz-debug.db` split
/// so `dx serve` never touches real settings).
pub fn settings_path_for(db_dir: &Path) -> PathBuf {
    if let Ok(p) = std::env::var(ENV_PATH_OVERRIDE) {
        return PathBuf::from(p);
    }
    let name = if cfg!(debug_assertions) {
        "settings-debug.toml"
    } else {
        "settings.toml"
    };
    db_dir.join(name)
}

/// The drop-in directory belonging to a settings file: `settings.toml` →
/// `settings.d/` next to it (`settings-debug.toml` → `settings-debug.d/`).
pub fn dropin_dir_for(settings_path: &Path) -> PathBuf {
    let stem = settings_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("settings");
    settings_path.with_file_name(format!("{stem}.d"))
}

/// The writable file beside a managed one, holding the settings it leaves unset: `settings.toml` → `settings.local.toml`.
pub fn local_path_for(settings_path: &Path) -> PathBuf {
    let stem = settings_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("settings");
    settings_path.with_file_name(format!("{stem}.local.toml"))
}

/// The read-side snapshot of every file/env layer, plus what the UI needs to
/// know about mutability.
#[derive(Debug, Clone, Default)]
pub struct FileLayers {
    /// The main settings file this snapshot was read from.
    pub path: PathBuf,
    /// Whether the main file existed at read time.
    pub file_exists: bool,
    /// The main file must not be written: it resolves into the Nix store or is
    /// read-only on disk.
    pub managed: bool,
    /// Top-level `Settings` keys pinned by an unwritable layer.
    pub locked_keys: BTreeSet<String>,
    /// Merged overrides (file → drop-ins → env).
    overrides: Table,
}

impl FileLayers {
    /// Read the file, its drop-ins, and the process environment.
    pub fn read(settings_path: &Path) -> Self {
        Self::read_inner(settings_path, NIX_STORE_PREFIX, std::env::vars())
    }

    fn read_inner(
        settings_path: &Path,
        store_prefix: &str,
        env: impl Iterator<Item = (String, String)>,
    ) -> Self {
        let mut overrides = Table::new();
        let mut locked_keys = BTreeSet::new();

        // Under the main file, so a managed file's keys still win over what the app saved beside it.
        if let Some(table) = read_toml_table(&local_path_for(settings_path)) {
            merge_into(&mut overrides, table);
        }

        let managed = is_managed(settings_path, store_prefix);
        let file_exists;
        if let Some(table) = read_toml_table(settings_path) {
            file_exists = true;
            if managed {
                locked_keys.extend(table.keys().cloned());
            }
            merge_into(&mut overrides, table);
        } else {
            file_exists = settings_path.exists();
        }

        // Drop-ins always lock their keys: they out-rank the main file, so a
        // UI edit saved there could never take effect.
        let dropin_dir = dropin_dir_for(settings_path);
        for path in sorted_toml_files(&dropin_dir) {
            if let Some(table) = read_toml_table(&path) {
                locked_keys.extend(table.keys().cloned());
                merge_into(&mut overrides, table);
            }
        }

        let env_layer = env_overrides(env);
        locked_keys.extend(env_layer.keys().cloned());
        merge_into(&mut overrides, env_layer);

        // The retired key's value now lives in the default folder source.
        if locked_keys.contains("music_directory") {
            locked_keys.insert("local_sources".to_owned());
        }

        Self {
            path: settings_path.to_path_buf(),
            file_exists,
            managed,
            locked_keys,
            overrides,
        }
    }

    /// Whether any layer contributes anything at all.
    pub fn has_overrides(&self) -> bool {
        !self.overrides.is_empty()
    }

    /// Whether the settings UI should treat a top-level settings key as read-only.
    pub fn is_locked(&self, key: &str) -> bool {
        self.locked_keys.contains(key)
    }

    /// Deep-merge the layered overrides over `base`.
    pub fn apply(&self, base: &mut Table) {
        merge_into(base, self.overrides.clone());
    }

    /// The settings every layer adds up to, with the default radio registry restored if a layer dropped it.
    pub fn settings(&self) -> Result<Settings, toml::de::Error> {
        let mut settings = self.merge_and_parse(Table::new())?;
        settings.integrations.migrate_registry_paths();
        Ok(settings)
    }

    /// The layers over `base` as settings; a hand-written key that breaks the parse is dropped with a warning instead of costing every other setting, but a `base` that cannot parse alone is still an error.
    pub fn merge_and_parse(&self, base: Table) -> Result<Settings, toml::de::Error> {
        let mut base = base;
        fold_legacy_directories(&mut base, false);
        let mut merged = base.clone();
        self.apply(&mut merged);
        fold_legacy_directories(&mut merged, true);
        let error = match parse(merged) {
            Ok(settings) => return Ok(settings),
            Err(error) => error,
        };
        parse(base.clone()).map_err(|_| error)?;

        let mut kept = base;
        let mut dropped: Vec<&str> = Vec::new();
        for (key, value) in &self.overrides {
            let mut candidate = kept.clone();
            merge_into(
                &mut candidate,
                Table::from_iter([(key.clone(), value.clone())]),
            );
            fold_legacy_directories(&mut candidate, true);
            if parse(candidate.clone()).is_ok() {
                kept = candidate;
            } else {
                dropped.push(key);
            }
        }
        if !dropped.is_empty() {
            tracing::warn!(
                path = %self.path.display(),
                keys = %dropped.join(", "),
                "ignoring unusable config keys from the settings file/drop-ins/env"
            );
        }
        parse(kept)
    }
}

fn parse(table: Table) -> Result<Settings, toml::de::Error> {
    table.try_into()
}

/// Write the settings to the file, or beside it when it is managed, keeping every locked key at the file's own value.
pub fn save_settings(settings_path: &Path, settings: &Settings) -> std::io::Result<bool> {
    let layers = FileLayers::read(settings_path);
    save_settings_file(settings_path, settings, &layers.locked_keys)
}

/// Write the settings, only when one changed, to the file or beside a managed one ([`local_path_for`]); answers whether anything was written.
pub fn save_settings_file(
    settings_path: &Path,
    settings: &Settings,
    locked_keys: &BTreeSet<String>,
) -> std::io::Result<bool> {
    let written_path = match is_managed(settings_path, NIX_STORE_PREFIX) {
        true => local_path_for(settings_path),
        false => settings_path.to_path_buf(),
    };

    let mut table = Table::try_from(settings).map_err(std::io::Error::other)?;
    // A pinned key's value in `settings` is the layer's, merged in on load; writing it back would bake the override into the base file.
    if !locked_keys.is_empty() {
        let existing = read_toml_table(&written_path).unwrap_or_default();
        for key in locked_keys {
            match existing.get(key) {
                Some(value) => table.insert(key.clone(), value.clone()),
                None => table.remove(key),
            };
        }
    }
    let table = written_table(table);
    let on_disk = std::fs::read_to_string(&written_path)
        .ok()
        .and_then(|text| text.parse::<Table>().ok());
    if on_disk.as_ref() == Some(&table) {
        return Ok(false);
    }
    let text = toml::to_string_pretty(&table).map_err(std::io::Error::other)?;

    // Follow a writable symlink to its target instead of replacing the link
    // with a regular file. (An hjem-managed link points into the store and
    // never gets here — this is for someone linking the file themselves.)
    let target = std::fs::canonicalize(&written_path).unwrap_or(written_path);
    if let Some(parent) = target.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = target.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &target)?;
    Ok(true)
}

/// A legacy JSON store's settings as a TOML table: nulls are dropped, which every `Option` field reads back as its default.
pub fn table_from_json(value: &JsonValue) -> Option<Table> {
    match json_to_toml(value)? {
        TomlValue::Table(table) => Some(table),
        _ => None,
    }
}

/// A file is "managed" when the app must not write it: it is (or resolves
/// through symlinks into) the Nix store, or it exists read-only.
fn is_managed(path: &Path, store_prefix: &str) -> bool {
    if let Ok(target) = std::fs::canonicalize(path)
        && target.starts_with(store_prefix)
    {
        return true;
    }
    match std::fs::metadata(path) {
        Ok(meta) => meta.permissions().readonly(),
        Err(_) => false,
    }
}

fn read_toml_table(path: &Path) -> Option<Table> {
    let text = std::fs::read_to_string(path).ok()?;
    match text.parse::<Table>() {
        Ok(table) => Some(read_table(table)),
        Err(e) => {
            tracing::warn!(path = %path.display(), "ignoring unparsable config layer: {e}");
            None
        }
    }
}

fn read_table(table: Table) -> Table {
    table.into_iter().map(|(k, v)| (k, read_value(v))).collect()
}

/// No setting is a date, so a datetime keeps its text and still deserializes into a string field.
fn read_value(value: TomlValue) -> TomlValue {
    match value {
        TomlValue::Datetime(dt) => TomlValue::String(dt.to_string()),
        TomlValue::Array(items) => TomlValue::Array(items.into_iter().map(read_value).collect()),
        TomlValue::Table(table) => TomlValue::Table(read_table(table)),
        other => other,
    }
}

fn written_table(table: Table) -> Table {
    table
        .into_iter()
        .filter_map(|(k, v)| Some((k, written_value(v)?)))
        .collect()
}

/// A non-finite float is not written, as the JSON round trip saves once took never wrote one, and a widened `f32` is written short.
fn written_value(value: TomlValue) -> Option<TomlValue> {
    Some(match value {
        TomlValue::Float(f) if f.is_finite() => TomlValue::Float(shorten_f32(f)),
        TomlValue::Float(_) => return None,
        TomlValue::Array(items) => {
            TomlValue::Array(items.into_iter().filter_map(written_value).collect())
        }
        TomlValue::Table(table) => TomlValue::Table(written_table(table)),
        other => other,
    })
}

fn sorted_toml_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "toml") && p.is_file())
        .collect();
    files.sort();
    files
}

/// The env layer: every `KOPUZ_CONFIG_<FIELD>` but the path override is a lowercased key, `__` nesting tables, its value a TOML literal or else a plain string.
fn env_overrides(env: impl Iterator<Item = (String, String)>) -> Table {
    let mut layer = Table::new();
    let mut vars: Vec<(String, String)> = env
        .filter(|(k, _)| k.starts_with(ENV_PREFIX) && k != ENV_PATH_OVERRIDE)
        .collect();
    vars.sort();
    for (key, raw) in vars {
        let path: Vec<String> = key[ENV_PREFIX.len()..]
            .split("__")
            .map(|s| s.to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let Some((leaf, parents)) = path.split_last() else {
            continue;
        };
        let mut nested = Table::from_iter([(leaf.clone(), parse_env_value(&raw))]);
        for part in parents.iter().rev() {
            nested = Table::from_iter([(part.clone(), TomlValue::Table(nested))]);
        }
        merge_into(&mut layer, nested);
    }
    layer
}

/// Parse one env value as the right-hand side of a TOML assignment, so
/// `5`/`true`/`["a"]`/`"quoted"` keep their types and anything else (`nord`)
/// stays a bare string.
fn parse_env_value(raw: &str) -> TomlValue {
    format!("v = {raw}")
        .parse::<Table>()
        .ok()
        .and_then(|mut t| t.remove("v"))
        .map(read_value)
        .unwrap_or_else(|| TomlValue::String(raw.to_string()))
}

/// TOML has no null, so a JSON null is dropped and its key reads back as the field's default.
fn json_to_toml(value: &JsonValue) -> Option<TomlValue> {
    Some(match value {
        JsonValue::Null => return None,
        JsonValue::Bool(b) => TomlValue::Boolean(*b),
        JsonValue::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => TomlValue::Integer(i),
            (None, Some(f)) if f.is_finite() => TomlValue::Float(shorten_f32(f)),
            _ => return None,
        },
        JsonValue::String(s) => TomlValue::String(s.clone()),
        JsonValue::Array(items) => {
            TomlValue::Array(items.iter().filter_map(json_to_toml).collect())
        }
        JsonValue::Object(map) => TomlValue::Table(
            map.iter()
                .filter_map(|(k, v)| Some((k.clone(), json_to_toml(v)?)))
                .collect(),
        ),
    })
}

/// Serializers widen `f32` to `f64` (`0.05` → `0.05000000074505806`), so a value that survives an `f32` round trip is written short.
fn shorten_f32(f: f64) -> f64 {
    let narrow = f as f32;
    if f64::from(narrow) != f {
        return f;
    }
    format!("{narrow}").parse().unwrap_or(f)
}

/// Deep merge: tables merge key-wise, everything else replaces.
fn merge_into(base: &mut Table, layer: Table) {
    for (key, value) in layer {
        match (base.get_mut(&key), value) {
            (Some(TomlValue::Table(slot)), TomlValue::Table(value)) => merge_into(slot, value),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_env() -> impl Iterator<Item = (String, String)> {
        std::iter::empty()
    }

    fn json(value: JsonValue) -> Table {
        table_from_json(&value).expect("a JSON object")
    }

    fn default_dirs(settings: &Settings) -> Vec<PathBuf> {
        settings
            .library
            .local_sources
            .iter()
            .find(|source| source.id == crate::DEFAULT_LOCAL_ID)
            .map(|source| source.directories.clone())
            .unwrap_or_default()
    }

    fn no_layers() -> FileLayers {
        let dir = tempfile::tempdir().unwrap();
        FileLayers::read_inner(&dir.path().join("none.toml"), NIX_STORE_PREFIX, empty_env())
    }

    fn reloaded(path: &Path) -> Settings {
        let layers = FileLayers::read_inner(path, NIX_STORE_PREFIX, empty_env());
        let mut base = Table::new();
        layers.apply(&mut base);
        base.try_into().unwrap()
    }

    const GOLDEN: &str = include_str!("../tests/fixtures/settings.toml");
    const GOLDEN_SAVED: &str = include_str!("../tests/fixtures/settings.saved.toml");
    const GOLDEN_SAVED_ENV: &str = include_str!("../tests/fixtures/settings.saved-env.toml");
    const GOLDEN_LOADED_ENV: &str = include_str!("../tests/fixtures/settings.loaded-env.json");

    fn golden_env() -> Vec<(String, String)> {
        [
            ("KOPUZ_CONFIG_THEME", "gruvbox"),
            ("KOPUZ_CONFIG_CROSSFADE_SECONDS", "4"),
            ("KOPUZ_CONFIG_EQUALIZER__PREAMP_DB", "-4.5"),
            ("KOPUZ_CONFIG_REPLAY_GAIN__MODE", "Album"),
            ("KOPUZ_CONFIG_PATH", "/elsewhere.toml"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
    }

    /// The expected files were written by the store before settings had sections, from this same fixture and env.
    #[test]
    fn the_golden_settings_file_loads_and_saves_as_before() {
        for (env, expected) in [(Vec::new(), GOLDEN_SAVED), (golden_env(), GOLDEN_SAVED_ENV)] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("settings.toml");
            std::fs::write(&path, GOLDEN).unwrap();
            let with_env = !env.is_empty();

            let layers = FileLayers::read_inner(&path, NIX_STORE_PREFIX, env.into_iter());
            let settings = layers.merge_and_parse(Table::new()).unwrap();
            if with_env {
                let loaded: JsonValue = serde_json::from_str(GOLDEN_LOADED_ENV).unwrap();
                assert_eq!(serde_json::to_value(&settings).unwrap(), loaded);
                assert_eq!(settings.ui.theme, "gruvbox");
                assert_eq!(settings.playback.crossfade_seconds, 4);
                assert_eq!(settings.playback.equalizer.preamp_db, -4.5);
                assert_eq!(settings.playback.equalizer.bands[9], 12.0);
                assert_eq!(
                    settings.playback.replay_gain.mode,
                    crate::ReplayGainMode::Album
                );
                assert_eq!(settings.playback.replay_gain.preamp_db, 1.5);
                assert_eq!(settings.ui.ui_style, crate::UiStyle::Vaxry);
                assert_eq!(settings.lyrics.lyrics_offset_ms, 1000);
                assert_eq!(
                    settings.downloads.downloader_output_dir,
                    "/home/me/Downloads/kopuz"
                );
                assert!(!settings.downloads.downloader_options.organize_by_album);
                assert_eq!(settings.ui.custom_themes.len(), 2);
                assert_eq!(settings.library.local_sources.len(), 2);
                assert_eq!(settings.integrations.radio_registries.len(), 2);
                assert_eq!(settings.integrations.discord_presence, Some(false));
                assert!(layers.is_locked("equalizer") && layers.is_locked("replay_gain"));
            }

            assert!(save_settings_file(&path, &settings, &layers.locked_keys).unwrap());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
            assert!(
                !save_settings_file(&path, &settings, &layers.locked_keys).unwrap(),
                "a second save of the same settings writes nothing"
            );
        }
    }

    #[test]
    fn a_stored_music_directory_becomes_the_default_folder_source() {
        for (stored, want) in [
            (serde_json::json!("/music"), vec!["/music"]),
            (
                serde_json::json!(["/music", "/archive"]),
                vec!["/music", "/archive"],
            ),
            (serde_json::json!([]), vec![]),
        ] {
            let settings = no_layers()
                .merge_and_parse(json(serde_json::json!({ "music_directory": stored })))
                .unwrap();
            assert_eq!(
                default_dirs(&settings),
                want.iter().map(PathBuf::from).collect::<Vec<_>>()
            );
            assert_eq!(
                settings.library.local_sources[0].id,
                crate::DEFAULT_LOCAL_ID
            );
        }
    }

    #[test]
    fn the_default_folder_source_exists_even_when_nothing_was_stored() {
        let settings = no_layers().merge_and_parse(Table::new()).unwrap();
        assert_eq!(settings.library.local_sources.len(), 1);
        assert_eq!(
            default_dirs(&settings),
            vec![crate::default_music_directory()]
        );
        assert_eq!(
            settings,
            Settings::default(),
            "nothing stored reads as the defaults"
        );
    }

    #[test]
    fn a_migrated_source_is_not_overwritten_by_a_leftover_key() {
        let settings = no_layers()
            .merge_and_parse(json(serde_json::json!({
                "music_directory": ["/old"],
                "local_sources": [
                    {"id": "local:x", "name": "X", "directories": ["/x"]},
                    {"id": "local", "name": "Local Library", "directories": ["/new"]},
                ],
            })))
            .unwrap();
        assert_eq!(default_dirs(&settings), vec![PathBuf::from("/new")]);
        assert_eq!(settings.library.local_sources.len(), 2);
    }

    #[test]
    fn a_managed_file_with_music_directory_wins_and_locks_the_folder_sources() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "music_directory = [\"/nix/music\"]\n").unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).unwrap();

        let layers = FileLayers::read_inner(&path, NIX_STORE_PREFIX, empty_env());
        assert!(layers.managed);
        assert!(layers.is_locked("local_sources"));

        let blob = json(serde_json::json!({
            "local_sources": [{"id": "local", "name": "Local Library", "directories": ["/db"]}],
        }));
        let settings = layers.merge_and_parse(blob).unwrap();
        assert_eq!(default_dirs(&settings), vec![PathBuf::from("/nix/music")]);
    }

    #[test]
    fn a_managed_empty_music_directory_clears_the_default_folder_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "music_directory = []\n").unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).unwrap();

        let layers = FileLayers::read_inner(&path, NIX_STORE_PREFIX, empty_env());
        let blob = json(serde_json::json!({
            "local_sources": [{"id": "local", "name": "Local Library", "directories": ["/db"]}],
        }));
        let settings = layers.merge_and_parse(blob).unwrap();
        assert!(default_dirs(&settings).is_empty());
    }

    #[test]
    fn a_malformed_music_directory_is_dropped_not_fatal() {
        let settings = no_layers()
            .merge_and_parse(json(serde_json::json!({ "music_directory": 7 })))
            .unwrap();
        assert_eq!(
            default_dirs(&settings),
            vec![crate::default_music_directory()]
        );
    }

    #[test]
    fn every_stored_active_source_spelling_still_loads() {
        for (stored, column) in [
            (serde_json::json!("Local"), "local"),
            (
                serde_json::json!({"LocalLibrary": "local:abc"}),
                "local:abc",
            ),
            (serde_json::json!({"Server": "srv"}), "srv"),
        ] {
            let source: crate::Source = serde_json::from_value(stored.clone()).unwrap();
            assert_eq!(source.as_str(), column);
            assert_eq!(serde_json::to_value(&source).unwrap(), stored);
            assert_eq!(crate::Source::from_column(column), source);
        }
    }

    #[test]
    fn material_style_and_system_colors_survive_a_settings_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let mut settings = Settings::default();
        settings.ui.ui_style = crate::UiStyle::Material3;
        settings.ui.theme = "system".into();
        save_settings_file(&path, &settings, &BTreeSet::new()).unwrap();
        let restored = FileLayers::read_inner(&path, NIX_STORE_PREFIX, empty_env())
            .merge_and_parse(Table::new())
            .unwrap();
        assert_eq!(restored.ui.ui_style, crate::UiStyle::Material3);
        assert_eq!(restored.ui.theme, "system");
    }

    #[test]
    fn default_settings_round_trip_through_the_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");

        assert!(save_settings_file(&path, &Settings::default(), &BTreeSet::new()).unwrap());

        assert_eq!(reloaded(&path), Settings::default());
    }

    /// Every collection/enum/nested shape at once: the default settings are mostly empty vectors and unit enums, so they would not catch a field TOML can't express.
    #[test]
    fn fully_populated_settings_round_trip_through_the_settings_file() {
        use std::collections::HashMap;

        use crate::{
            BackBehavior, ChannelMode, CustomTheme, DeviceChangeBehavior, EqPreset,
            EqualizerSettings, FetchStrategy, OfflineQuality, PlayerBarPosition, RegistryEntry,
            SampleRateMode, SavedLocalSource, SettingsLayout, TitlebarMode, UiStyle,
        };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let mut settings = Settings::default();
        settings.library.local_sources = vec![
            SavedLocalSource {
                id: "local:one".into(),
                name: "Archive".into(),
                directories: vec!["/music/a".into(), "/music/b".into()],
            },
            SavedLocalSource {
                id: "local:two".into(),
                name: "Field recordings".into(),
                directories: vec![],
            },
        ];
        settings.spotify.spotify_browser = Some("brave".into());
        settings.spotify.spotify_prefer_active_device = false;
        settings.ui.theme = "custom-one".into();
        settings.integrations.discord_presence = Some(false);
        settings.ui.language = "tr".into();
        settings.background.cover_art_darkening = 42;
        settings.ui.custom_font_path = "/fonts/Inter.ttf".into();
        settings.playback.crossfade_seconds = 7;
        settings.ui.custom_themes = HashMap::from([(
            "custom-one".to_string(),
            CustomTheme {
                name: "Custom One".into(),
                vars: HashMap::from([
                    ("--color-bg".to_string(), "#101010".to_string()),
                    ("--color-fg".to_string(), "#eeeeee".to_string()),
                ]),
            },
        )]);
        settings.playback.back_behavior = BackBehavior::AlwaysPrev;
        settings.playback.channel_mode = ChannelMode::Mono;
        settings.playback.equalizer = EqualizerSettings {
            enabled: true,
            preset: EqPreset::Custom,
            bands: [-6.0, -3.5, 0.0, 1.5, 3.0, 4.5, 6.0, -1.0, -2.0, 12.0],
            preamp_db: -2.5,
        };
        settings.playback.device_change_behavior = DeviceChangeBehavior::Resume;
        settings.playback.sample_rate_mode = SampleRateMode::Source;
        settings.downloads.downloader_output_dir = "/downloads".into();
        settings.ui.titlebar_mode = TitlebarMode::System;
        settings.library.offline_quality = OfflineQuality::Original;
        settings.ui.player_bar_position = PlayerBarPosition::Top;
        settings.ui.ui_style = UiStyle::Vaxry;
        settings.ui.settings_layout = SettingsLayout::TopBar;
        settings.library.auto_fetch_covers = true;
        settings.library.cover_fetch_strategy = FetchStrategy::LastFmOnly;
        settings.integrations.radio_registries = vec![RegistryEntry {
            url: "https://example.invalid/index.json".into(),
            enabled: false,
            is_default: false,
        }];
        settings.lyrics.prefer_local_lyrics = true;
        settings.library.sync_interval_minutes = 90;

        let written = save_settings_file(&path, &settings, &BTreeSet::new());
        assert!(written.unwrap(), "settings file not written");

        let restored = reloaded(&path);
        assert_eq!(restored, settings);
    }

    #[test]
    fn f32_fields_are_written_short_and_reload_identically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let mut settings = Settings::default();
        settings.playback.volume_scroll_step = 0.05;

        save_settings_file(&path, &settings, &BTreeSet::new()).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("volume_scroll_step = 0.05\n"),
            "f32 widening leaked into the file:\n{text}"
        );
        assert_eq!(
            reloaded(&path).playback.volume_scroll_step,
            settings.playback.volume_scroll_step
        );
    }

    #[test]
    fn a_key_an_older_build_wrote_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "hero_height = 900\ntheme = \"nord\"\n").unwrap();
        let layers = FileLayers::read_inner(&path, NIX_STORE_PREFIX, empty_env());
        let settings = layers.merge_and_parse(Table::new()).unwrap();
        assert_eq!(settings.ui.theme, "nord");

        save_settings_file(&path, &settings, &BTreeSet::new()).unwrap();
        let written: Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert!(!written.contains_key("hero_height"));
    }

    #[test]
    fn dropins_and_env_out_rank_the_file_and_lock_their_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "theme = \"nord\"\nlanguage = \"de\"\n").unwrap();
        let dropin_dir = dropin_dir_for(&path);
        std::fs::create_dir_all(&dropin_dir).unwrap();
        std::fs::write(dropin_dir.join("10-theme.toml"), "theme = \"dracula\"\n").unwrap();
        let env = vec![("KOPUZ_CONFIG_THEME".to_string(), "gruvbox".to_string())];

        let layers = FileLayers::read_inner(&path, NIX_STORE_PREFIX, env.into_iter());

        let mut base = json(serde_json::json!({ "theme": "default", "crossfade_seconds": 5 }));
        layers.apply(&mut base);
        assert_eq!(base["theme"].as_str(), Some("gruvbox"));
        assert_eq!(base["language"].as_str(), Some("de"));
        assert_eq!(base["crossfade_seconds"].as_integer(), Some(5));
        // A writable main file doesn't lock; drop-ins and env do.
        assert!(layers.is_locked("theme"));
        assert!(!layers.is_locked("language"));
        assert!(!layers.managed);
    }

    #[test]
    fn nested_toml_tables_merge_key_wise() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(
            &path,
            "[equalizer]\nenabled = true\npreamp_db = -3.0\npreset = \"Custom\"\n",
        )
        .unwrap();

        let layers = FileLayers::read_inner(&path, NIX_STORE_PREFIX, empty_env());
        let mut base =
            json(serde_json::json!({ "equalizer": { "enabled": false, "bands": [1.0] } }));
        layers.apply(&mut base);

        assert_eq!(base["equalizer"]["enabled"].as_bool(), Some(true));
        assert_eq!(base["equalizer"]["preamp_db"].as_float(), Some(-3.0));
        // Untouched sub-keys survive the merge.
        assert_eq!(
            base["equalizer"]["bands"],
            TomlValue::Array(vec![TomlValue::Float(1.0)])
        );
    }

    #[test]
    fn env_values_parse_as_toml_with_string_fallback() {
        let env = vec![
            (
                "KOPUZ_CONFIG_CROSSFADE_SECONDS".to_string(),
                "5".to_string(),
            ),
            ("KOPUZ_CONFIG_THEME".to_string(), "nord".to_string()),
            (
                "KOPUZ_CONFIG_EQUALIZER__ENABLED".to_string(),
                "true".to_string(),
            ),
            (
                "KOPUZ_CONFIG_MUSIC_DIRECTORY".to_string(),
                r#"["/music", "/archive"]"#.to_string(),
            ),
            (
                "KOPUZ_CONFIG_CUSTOM_THEMES".to_string(),
                r#"{"a":1}"#.to_string(),
            ),
            (
                "KOPUZ_CONFIG_LIVE_THEME_PATH".to_string(),
                "1979-05-27".to_string(),
            ),
            ("KOPUZ_CONFIG_PATH".to_string(), "/tmp/x.toml".to_string()),
        ];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");

        let layers = FileLayers::read_inner(&path, NIX_STORE_PREFIX, env.into_iter());

        let mut base = Table::new();
        layers.apply(&mut base);
        assert_eq!(base["crossfade_seconds"].as_integer(), Some(5));
        assert_eq!(base["theme"].as_str(), Some("nord"));
        assert_eq!(base["equalizer"]["enabled"].as_bool(), Some(true));
        assert_eq!(base["music_directory"][1].as_str(), Some("/archive"));
        // JSON is not TOML, so a JSON object stays the literal text.
        assert_eq!(base["custom_themes"].as_str(), Some(r#"{"a":1}"#));
        assert_eq!(base["live_theme_path"].as_str(), Some("1979-05-27"));
        // The path override is not a field.
        assert!(!base.contains_key("path"));
        assert!(layers.is_locked("equalizer"));
    }

    #[test]
    fn a_managed_file_is_never_written_and_the_rest_goes_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "theme = \"nord\"\n").unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).unwrap();

        let layers = FileLayers::read_inner(&path, NIX_STORE_PREFIX, empty_env());
        assert!(layers.managed);
        assert!(layers.is_locked("theme"));

        let mut settings = Settings::default();
        settings.ui.theme = "nord".into();
        settings.ui.language = "tr".into();
        assert!(save_settings_file(&path, &settings, &layers.locked_keys).unwrap());

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "theme = \"nord\"\n"
        );
        let local: Table = std::fs::read_to_string(local_path_for(&path))
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            !local.contains_key("theme"),
            "the managed key stays the managed file's"
        );
        assert_eq!(local.get("language").and_then(|v| v.as_str()), Some("tr"));

        std::fs::write(
            local_path_for(&path),
            "theme = \"gruvbox\"\nlanguage = \"tr\"\n",
        )
        .unwrap();
        let layers = FileLayers::read_inner(&path, NIX_STORE_PREFIX, empty_env());
        let restored = layers.merge_and_parse(Table::new()).unwrap();
        assert_eq!(
            (restored.ui.theme.as_str(), restored.ui.language.as_str()),
            ("nord", "tr")
        );
    }

    #[test]
    fn a_save_that_changes_no_setting_leaves_the_file_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let settings = Settings::default();
        save_settings_file(&path, &settings, &BTreeSet::new()).unwrap();
        let hand_written = format!("# mine\n{}", std::fs::read_to_string(&path).unwrap());
        std::fs::write(&path, &hand_written).unwrap();

        let saved = save_settings_file(&path, &settings, &BTreeSet::new());

        assert!(!saved.unwrap(), "no setting changed");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), hand_written);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_into_store_prefix_is_managed() {
        let dir = tempfile::tempdir().unwrap();
        // is_managed compares canonical paths, and macOS temp dirs sit behind /var -> /private/var.
        let store = dir.path().canonicalize().unwrap().join("nix-store");
        std::fs::create_dir_all(&store).unwrap();
        let target = store.join("abc-kopuz-settings.toml");
        std::fs::write(&target, "theme = \"nord\"\n").unwrap();
        let link = dir.path().join("settings.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let layers = FileLayers::read_inner(&link, store.to_str().unwrap(), empty_env());

        assert!(layers.managed);
        assert!(layers.is_locked("theme"));
    }

    /// A hand-made link to a writable file: the write lands on the target and the link survives.
    #[cfg(unix)]
    #[test]
    fn writable_symlink_is_written_through_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("dotfiles-settings.toml");
        std::fs::write(&target, "theme = \"nord\"\n").unwrap();
        let link = dir.path().join("settings.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(save_settings_file(&link, &Settings::default(), &BTreeSet::new()).unwrap());

        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        let written: Table = std::fs::read_to_string(&target).unwrap().parse().unwrap();
        assert_eq!(written["theme"].as_str(), Some("default"));
    }

    /// The exact snippet the README hands to hjem users; if a rename makes it stale, this fails instead of the user's first launch.
    #[test]
    fn the_documented_hjem_example_deserializes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(
            &path,
            "theme = \"gruvbox\"\nlanguage = \"en\"\ncrossfade_seconds = 3\n\n[equalizer]\nenabled = true\n",
        )
        .unwrap();

        let layers = FileLayers::read_inner(&path, NIX_STORE_PREFIX, empty_env());
        let settings = layers.merge_and_parse(Table::new()).unwrap();

        assert_eq!(settings.ui.theme, "gruvbox");
        assert_eq!(settings.ui.language, "en");
        assert_eq!(settings.playback.crossfade_seconds, 3);
        assert!(settings.playback.equalizer.enabled);
        // Only `enabled` was set, so the rest of the table keeps its defaults.
        assert_eq!(
            settings.playback.equalizer.bands,
            Settings::default().playback.equalizer.bands
        );
    }

    #[test]
    fn one_unusable_key_is_dropped_instead_of_failing_the_whole_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(
            &path,
            // `crossfade_seconds` wants an integer and `ui_style` a known variant — the two ways a hand-written value goes wrong.
            "theme = \"nord\"\ncrossfade_seconds = \"loud\"\nui_style = \"Fancy\"\nlanguage = \"tr\"\n",
        )
        .unwrap();

        let layers = FileLayers::read_inner(&path, NIX_STORE_PREFIX, empty_env());
        let base = json(serde_json::json!({ "theme": "default", "crossfade_seconds": 3 }));
        let settings = layers.merge_and_parse(base).expect("config still loads");

        assert_eq!(settings.ui.theme, "nord", "good keys still apply");
        assert_eq!(settings.ui.language, "tr");
        assert_eq!(
            settings.playback.crossfade_seconds, 3,
            "the base value survives"
        );
        assert_eq!(settings.ui.ui_style, crate::UiStyle::default());
    }

    #[test]
    fn a_broken_base_is_still_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "theme = \"nord\"\n").unwrap();

        let layers = FileLayers::read_inner(&path, NIX_STORE_PREFIX, empty_env());
        let base = json(serde_json::json!({ "crossfade_seconds": { "not": "a number" } }));

        assert!(
            layers.merge_and_parse(base).is_err(),
            "a corrupt blob must not be papered over — saves stay disabled"
        );
    }

    #[test]
    fn missing_file_has_no_overrides_and_is_not_managed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");

        let layers = FileLayers::read_inner(&path, NIX_STORE_PREFIX, empty_env());

        assert!(!layers.has_overrides());
        assert!(!layers.managed);
        assert!(!layers.file_exists);
        assert!(layers.locked_keys.is_empty());
    }

    #[test]
    fn unparsable_file_is_ignored_rather_than_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "this is not = = toml\n").unwrap();

        let layers = FileLayers::read_inner(&path, NIX_STORE_PREFIX, empty_env());

        assert!(!layers.has_overrides());
        assert!(layers.file_exists);
    }
}
