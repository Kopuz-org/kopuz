//! Typed settings edits: one enum per section with a variant per field, the key each variant names, and the validation an edit runs.

use std::collections::HashMap;

use garde::Validate;
use strum::{EnumDiscriminants, EnumIter, IntoEnumIterator, IntoStaticStr};

use crate::{
    BackBehavior, Background, ChannelMode, CustomTheme, DeviceChangeBehavior, DownloaderOptions,
    Downloads, EqualizerSettings, FetchStrategy, Integrations, Library, Lyrics, OfflineQuality,
    Playback, PlayerBarPosition, RegistryEntry, ReplayGainSettings, SampleRateMode,
    SavedLocalSource, Settings, SettingsLayout, Spotify, StreamQuality, System, TitlebarMode, Ui,
    UiStyle,
};

#[derive(Debug, Clone, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(PlaybackKey), derive(Hash, IntoStaticStr, EnumIter))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
pub enum PlaybackEdit {
    VolumeScrollStep(f32),
    CrossfadeSeconds(u8),
    BackBehavior(BackBehavior),
    ChannelMode(ChannelMode),
    Equalizer(EqualizerSettings),
    ReplayGain(ReplayGainSettings),
    DeviceChangeBehavior(DeviceChangeBehavior),
    SampleRateMode(SampleRateMode),
    StreamQuality(StreamQuality),
    AutoplayRadio(bool),
    SkipExplicit(bool),
    PauseWatchHistory(bool),
}

#[derive(Debug, Clone, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(UiKey), derive(Hash, IntoStaticStr, EnumIter))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
pub enum UiEdit {
    Theme(String),
    LiveThemePath(String),
    CustomThemes(CustomThemeOp),
    Language(String),
    ReduceAnimations(bool),
    FullscreenUsePlayerBar(bool),
    ShowSourceToggle(bool),
    ShowRowImages(bool),
    TitlebarMode(TitlebarMode),
    PlayerBarPosition(PlayerBarPosition),
    UiStyle(UiStyle),
    SettingsLayout(SettingsLayout),
    CustomFontPath(String),
}

#[derive(Debug, Clone, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(BackgroundKey), derive(Hash, IntoStaticStr, EnumIter))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
pub enum BackgroundEdit {
    CoverArtBackground(bool),
    CoverArtDarkening(u8),
    CoverArtBlur(u8),
    CustomBackgroundPath(String),
}

#[derive(Debug, Clone, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(LyricsKey), derive(Hash, IntoStaticStr, EnumIter))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
pub enum LyricsEdit {
    PreferLocalLyrics(bool),
    EnableMusixmatchLyrics(bool),
    LyricsOffsetMs(i32),
    LyricsOffsetAuto(bool),
    LyricsDepthBlur(bool),
    LyricsDepthBlurStrength(u8),
}

#[derive(Debug, Clone, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(LibraryKey), derive(Hash, IntoStaticStr, EnumIter))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
pub enum LibraryEdit {
    LocalSources(Vec<SavedLocalSource>),
    AutoFetchCovers(bool),
    CoverFetchStrategy(FetchStrategy),
    OfflineQuality(OfflineQuality),
    OfflineCacheEnabled(bool),
    OfflineCacheMaxBytes(u64),
    SyncIntervalMinutes(u32),
}

#[derive(Debug, Clone, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(IntegrationsKey), derive(Hash, IntoStaticStr, EnumIter))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
pub enum IntegrationsEdit {
    DiscordPresence(Option<bool>),
    DiscordPresenceSource(Option<bool>),
    RadioRegistries(RadioRegistryOp),
}

/// One member of `custom_themes`, by the name it is stored under.
#[derive(Debug, Clone, PartialEq)]
pub enum CustomThemeOp {
    /// Insert or replace the theme stored under `name`.
    Put {
        name: String,
        theme: CustomTheme,
    },
    Delete {
        name: String,
    },
}

/// One member of `radio_registries`, by its url.
#[derive(Debug, Clone, PartialEq)]
pub enum RadioRegistryOp {
    /// Insert the registry, or replace the one already listed under its url.
    Put(RegistryEntry),
    /// The default registry is never removed.
    Remove { url: String },
}

#[derive(Debug, Clone, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(DownloadsKey), derive(Hash, IntoStaticStr, EnumIter))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
pub enum DownloadsEdit {
    #[strum_discriminants(strum(serialize = "ytdlp_output_dir"))]
    DownloaderOutputDir(String),
    #[strum_discriminants(strum(serialize = "ytdlp_options"))]
    DownloaderOptions(DownloaderOptions),
}

#[derive(Debug, Clone, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(SpotifyKey), derive(Hash, IntoStaticStr, EnumIter))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
pub enum SpotifyEdit {
    SpotifyBrowser(Option<String>),
    SpotifyPreferActiveDevice(bool),
}

#[derive(Debug, Clone, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(SystemKey), derive(Hash, IntoStaticStr, EnumIter))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
pub enum SystemEdit {
    TracingEnabled(bool),
    AutoCheckUpdates(bool),
    MinimizeToTray(bool),
    ConnectivityCheckUrls(Vec<String>),
}

/// One change to one setting.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsEdit {
    Playback(PlaybackEdit),
    Ui(UiEdit),
    Background(BackgroundEdit),
    Lyrics(LyricsEdit),
    Library(LibraryEdit),
    Integrations(IntegrationsEdit),
    Downloads(DownloadsEdit),
    Spotify(SpotifyEdit),
    System(SystemEdit),
}

/// Which setting an edit names: exactly one per stored field, collections included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingKey {
    Playback(PlaybackKey),
    Ui(UiKey),
    Background(BackgroundKey),
    Lyrics(LyricsKey),
    Library(LibraryKey),
    Integrations(IntegrationsKey),
    Downloads(DownloadsKey),
    Spotify(SpotifyKey),
    System(SystemKey),
}

impl SettingKey {
    /// The key as the settings files store it.
    pub fn name(&self) -> &'static str {
        match *self {
            Self::Playback(key) => key.into(),
            Self::Ui(key) => key.into(),
            Self::Background(key) => key.into(),
            Self::Lyrics(key) => key.into(),
            Self::Library(key) => key.into(),
            Self::Integrations(key) => key.into(),
            Self::Downloads(key) => key.into(),
            Self::Spotify(key) => key.into(),
            Self::System(key) => key.into(),
        }
    }

    pub fn of(edit: &SettingsEdit) -> Self {
        match edit {
            SettingsEdit::Playback(edit) => Self::Playback(edit.into()),
            SettingsEdit::Ui(edit) => Self::Ui(edit.into()),
            SettingsEdit::Background(edit) => Self::Background(edit.into()),
            SettingsEdit::Lyrics(edit) => Self::Lyrics(edit.into()),
            SettingsEdit::Library(edit) => Self::Library(edit.into()),
            SettingsEdit::Integrations(edit) => Self::Integrations(edit.into()),
            SettingsEdit::Downloads(edit) => Self::Downloads(edit.into()),
            SettingsEdit::Spotify(edit) => Self::Spotify(edit.into()),
            SettingsEdit::System(edit) => Self::System(edit.into()),
        }
    }

    pub fn iter() -> impl Iterator<Item = Self> {
        PlaybackKey::iter()
            .map(Self::Playback)
            .chain(UiKey::iter().map(Self::Ui))
            .chain(BackgroundKey::iter().map(Self::Background))
            .chain(LyricsKey::iter().map(Self::Lyrics))
            .chain(LibraryKey::iter().map(Self::Library))
            .chain(IntegrationsKey::iter().map(Self::Integrations))
            .chain(DownloadsKey::iter().map(Self::Downloads))
            .chain(SpotifyKey::iter().map(Self::Spotify))
            .chain(SystemKey::iter().map(Self::System))
    }

    /// The key stored under `name`, or `None` for a name that is not a setting.
    pub fn named(name: &str) -> Option<Self> {
        Self::iter().find(|key| key.name() == name)
    }

    /// The API that sets this key when the daemon owns it, which is what an edit of it is refused with.
    pub fn owner(&self) -> Option<&'static str> {
        match *self {
            Self::Library(key) => match key {
                LibraryKey::LocalSources => Some("SourceApi::upsert_source"),
                LibraryKey::AutoFetchCovers | LibraryKey::CoverFetchStrategy => {
                    Some("ArtworkApi::set_artwork_settings")
                }
                LibraryKey::OfflineQuality
                | LibraryKey::OfflineCacheEnabled
                | LibraryKey::OfflineCacheMaxBytes
                | LibraryKey::SyncIntervalMinutes => None,
            },
            Self::Integrations(key) => match key {
                IntegrationsKey::DiscordPresence | IntegrationsKey::DiscordPresenceSource => {
                    Some("SourceApi::set_integration_settings")
                }
                IntegrationsKey::RadioRegistries => None,
            },
            Self::Downloads(_) => Some("JobApi::set_downloader_settings"),
            Self::Spotify(_) => Some("SourceApi::set_source_settings"),
            Self::Playback(_)
            | Self::Ui(_)
            | Self::Background(_)
            | Self::Lyrics(_)
            | Self::System(_) => None,
        }
    }

    pub fn daemon_owned(&self) -> bool {
        self.owner().is_some()
    }
}

impl SettingsEdit {
    /// Whether this edit makes `earlier` moot: the same field, and for a collection op the same member.
    pub fn supersedes(&self, earlier: &Self) -> bool {
        SettingKey::of(self) == SettingKey::of(earlier) && self.member() == earlier.member()
    }

    fn member(&self) -> Option<&str> {
        match self {
            Self::Ui(UiEdit::CustomThemes(
                CustomThemeOp::Put { name, .. } | CustomThemeOp::Delete { name },
            )) => Some(name),
            Self::Integrations(IntegrationsEdit::RadioRegistries(op)) => match op {
                RadioRegistryOp::Put(entry) => Some(&entry.url),
                RadioRegistryOp::Remove { url } => Some(url),
            },
            _ => None,
        }
    }
}

impl Settings {
    /// The keys whose stored value differs between `self` and `other`; all of them if either fails to serialize.
    pub fn changed_keys(&self, other: &Self) -> Vec<SettingKey> {
        let (Ok(serde_json::Value::Object(mine)), Ok(serde_json::Value::Object(theirs))) =
            (serde_json::to_value(self), serde_json::to_value(other))
        else {
            return SettingKey::iter().collect();
        };
        SettingKey::iter()
            .filter(|key| mine.get(key.name()) != theirs.get(key.name()))
            .collect()
    }

    pub fn apply(&mut self, edit: SettingsEdit) {
        match edit {
            SettingsEdit::Playback(edit) => self.playback.apply(edit),
            SettingsEdit::Ui(edit) => self.ui.apply(edit),
            SettingsEdit::Background(edit) => self.background.apply(edit),
            SettingsEdit::Lyrics(edit) => self.lyrics.apply(edit),
            SettingsEdit::Library(edit) => self.library.apply(edit),
            SettingsEdit::Integrations(edit) => self.integrations.apply(edit),
            SettingsEdit::Downloads(edit) => self.downloads.apply(edit),
            SettingsEdit::Spotify(edit) => self.spotify.apply(edit),
            SettingsEdit::System(edit) => self.system.apply(edit),
        }
    }

    /// Apply `edit` to a copy of its section and keep the copy unless the edited field breaks a rule; a value some other layer set out of range does not count against it.
    pub fn try_apply(&mut self, edit: SettingsEdit) -> Result<(), String> {
        let key = SettingKey::of(&edit);
        match edit {
            SettingsEdit::Playback(edit) => checked(&mut self.playback, key, |s| s.apply(edit)),
            SettingsEdit::Ui(edit) => checked(&mut self.ui, key, |s| s.apply(edit)),
            SettingsEdit::Background(edit) => checked(&mut self.background, key, |s| s.apply(edit)),
            SettingsEdit::Lyrics(edit) => checked(&mut self.lyrics, key, |s| s.apply(edit)),
            SettingsEdit::Library(edit) => checked(&mut self.library, key, |s| s.apply(edit)),
            SettingsEdit::Integrations(edit) => {
                checked(&mut self.integrations, key, |s| s.apply(edit))
            }
            SettingsEdit::Downloads(edit) => checked(&mut self.downloads, key, |s| s.apply(edit)),
            SettingsEdit::Spotify(edit) => checked(&mut self.spotify, key, |s| s.apply(edit)),
            SettingsEdit::System(edit) => checked(&mut self.system, key, |s| s.apply(edit)),
        }
    }
}

fn checked<S: Validate<Context = ()> + Clone>(
    section: &mut S,
    key: SettingKey,
    apply: impl FnOnce(&mut S),
) -> Result<(), String> {
    let mut next = section.clone();
    apply(&mut next);
    if let Err(report) = next.validate() {
        let broken: Vec<String> = report
            .iter()
            .filter(|(path, _)| field_of(&path.to_string()) == key.name())
            .map(|(path, error)| format!("{path}: {error}"))
            .collect();
        if !broken.is_empty() {
            return Err(broken.join("; "));
        }
    }
    *section = next;
    Ok(())
}

fn field_of(path: &str) -> &str {
    path.split(['.', '[']).next().unwrap_or(path)
}

impl Playback {
    fn apply(&mut self, edit: PlaybackEdit) {
        match edit {
            PlaybackEdit::VolumeScrollStep(value) => self.volume_scroll_step = value,
            PlaybackEdit::CrossfadeSeconds(value) => self.crossfade_seconds = value,
            PlaybackEdit::BackBehavior(value) => self.back_behavior = value,
            PlaybackEdit::ChannelMode(value) => self.channel_mode = value,
            PlaybackEdit::Equalizer(value) => self.equalizer = value,
            PlaybackEdit::ReplayGain(value) => self.replay_gain = value,
            PlaybackEdit::DeviceChangeBehavior(value) => self.device_change_behavior = value,
            PlaybackEdit::SampleRateMode(value) => self.sample_rate_mode = value,
            PlaybackEdit::StreamQuality(value) => self.stream_quality = value,
            PlaybackEdit::AutoplayRadio(value) => self.autoplay_radio = value,
            PlaybackEdit::SkipExplicit(value) => self.skip_explicit = value,
            PlaybackEdit::PauseWatchHistory(value) => self.pause_watch_history = value,
        }
    }
}

impl Ui {
    fn apply(&mut self, edit: UiEdit) {
        match edit {
            UiEdit::Theme(value) => self.theme = value,
            UiEdit::LiveThemePath(value) => self.live_theme_path = value,
            UiEdit::CustomThemes(CustomThemeOp::Put { name, theme }) => {
                self.custom_themes.insert(name, theme);
            }
            UiEdit::CustomThemes(CustomThemeOp::Delete { name }) => {
                self.custom_themes.remove(&name);
            }
            UiEdit::Language(value) => self.language = value,
            UiEdit::ReduceAnimations(value) => self.reduce_animations = value,
            UiEdit::FullscreenUsePlayerBar(value) => self.fullscreen_use_player_bar = value,
            UiEdit::ShowSourceToggle(value) => self.show_source_toggle = value,
            UiEdit::ShowRowImages(value) => self.show_row_images = value,
            UiEdit::TitlebarMode(value) => self.titlebar_mode = value,
            UiEdit::PlayerBarPosition(value) => self.player_bar_position = value,
            UiEdit::UiStyle(value) => self.ui_style = value,
            UiEdit::SettingsLayout(value) => self.settings_layout = value,
            UiEdit::CustomFontPath(value) => self.custom_font_path = value,
        }
    }
}

impl Background {
    fn apply(&mut self, edit: BackgroundEdit) {
        match edit {
            BackgroundEdit::CoverArtBackground(value) => self.cover_art_background = value,
            BackgroundEdit::CoverArtDarkening(value) => self.cover_art_darkening = value,
            BackgroundEdit::CoverArtBlur(value) => self.cover_art_blur = value,
            BackgroundEdit::CustomBackgroundPath(value) => self.custom_background_path = value,
        }
    }
}

impl Lyrics {
    fn apply(&mut self, edit: LyricsEdit) {
        match edit {
            LyricsEdit::PreferLocalLyrics(value) => self.prefer_local_lyrics = value,
            LyricsEdit::EnableMusixmatchLyrics(value) => self.enable_musixmatch_lyrics = value,
            LyricsEdit::LyricsOffsetMs(value) => self.lyrics_offset_ms = value,
            LyricsEdit::LyricsOffsetAuto(value) => self.lyrics_offset_auto = value,
            LyricsEdit::LyricsDepthBlur(value) => self.lyrics_depth_blur = value,
            LyricsEdit::LyricsDepthBlurStrength(value) => self.lyrics_depth_blur_strength = value,
        }
    }
}

impl Library {
    fn apply(&mut self, edit: LibraryEdit) {
        match edit {
            LibraryEdit::LocalSources(value) => self.local_sources = value,
            LibraryEdit::AutoFetchCovers(value) => self.auto_fetch_covers = value,
            LibraryEdit::CoverFetchStrategy(value) => self.cover_fetch_strategy = value,
            LibraryEdit::OfflineQuality(value) => self.offline_quality = value,
            LibraryEdit::OfflineCacheEnabled(value) => self.offline_cache_enabled = value,
            LibraryEdit::OfflineCacheMaxBytes(value) => self.offline_cache_max_bytes = value,
            LibraryEdit::SyncIntervalMinutes(value) => self.sync_interval_minutes = value,
        }
    }
}

impl Integrations {
    fn apply(&mut self, edit: IntegrationsEdit) {
        match edit {
            IntegrationsEdit::DiscordPresence(value) => self.discord_presence = value,
            IntegrationsEdit::DiscordPresenceSource(value) => self.discord_presence_source = value,
            IntegrationsEdit::RadioRegistries(RadioRegistryOp::Put(entry)) => match self
                .radio_registries
                .iter_mut()
                .find(|held| held.url == entry.url)
            {
                Some(held) => *held = entry,
                None => self.radio_registries.push(entry),
            },
            IntegrationsEdit::RadioRegistries(RadioRegistryOp::Remove { url }) => self
                .radio_registries
                .retain(|held| held.url != url || held.is_default),
        }
    }
}

impl Downloads {
    fn apply(&mut self, edit: DownloadsEdit) {
        match edit {
            DownloadsEdit::DownloaderOutputDir(value) => self.downloader_output_dir = value,
            DownloadsEdit::DownloaderOptions(value) => self.downloader_options = value,
        }
    }
}

impl Spotify {
    fn apply(&mut self, edit: SpotifyEdit) {
        match edit {
            SpotifyEdit::SpotifyBrowser(value) => self.spotify_browser = value,
            SpotifyEdit::SpotifyPreferActiveDevice(value) => {
                self.spotify_prefer_active_device = value
            }
        }
    }
}

impl System {
    fn apply(&mut self, edit: SystemEdit) {
        match edit {
            SystemEdit::TracingEnabled(value) => self.tracing_enabled = value,
            SystemEdit::AutoCheckUpdates(value) => self.auto_check_updates = value,
            SystemEdit::MinimizeToTray(value) => self.minimize_to_tray = value,
            SystemEdit::ConnectivityCheckUrls(value) => self.connectivity_check_urls = value,
        }
    }
}

/// One edit per key, each with a value the defaults do not hold, so a test can see it land.
#[doc(hidden)]
pub fn edit_samples() -> Vec<SettingsEdit> {
    let theme = CustomTheme {
        name: "Nord".into(),
        vars: HashMap::from([("bg".to_string(), "#2e3440".to_string())]),
    };
    let equalizer = EqualizerSettings {
        enabled: true,
        preset: crate::EqPreset::Custom,
        bands: [1.0, -2.0, 3.0, 0.0, 0.5, 0.0, 0.0, -1.0, 2.0, 4.0],
        preamp_db: -3.0,
    };
    let replay_gain = ReplayGainSettings {
        mode: crate::ReplayGainMode::Album,
        prevent_clipping: false,
        preamp_db: 2.0,
        fallback_gain_db: -4.0,
        normalize_loudness: false,
    };
    let registry = RegistryEntry {
        url: "https://example.org/registry.json".into(),
        enabled: false,
        is_default: false,
    };
    vec![
        SettingsEdit::Playback(PlaybackEdit::VolumeScrollStep(0.1)),
        SettingsEdit::Playback(PlaybackEdit::CrossfadeSeconds(6)),
        SettingsEdit::Playback(PlaybackEdit::BackBehavior(BackBehavior::AlwaysPrev)),
        SettingsEdit::Playback(PlaybackEdit::ChannelMode(ChannelMode::Mono)),
        SettingsEdit::Playback(PlaybackEdit::Equalizer(equalizer)),
        SettingsEdit::Playback(PlaybackEdit::ReplayGain(replay_gain)),
        SettingsEdit::Playback(PlaybackEdit::DeviceChangeBehavior(
            DeviceChangeBehavior::Resume,
        )),
        SettingsEdit::Playback(PlaybackEdit::SampleRateMode(SampleRateMode::Source)),
        SettingsEdit::Playback(PlaybackEdit::StreamQuality(StreamQuality::Low)),
        SettingsEdit::Playback(PlaybackEdit::AutoplayRadio(true)),
        SettingsEdit::Playback(PlaybackEdit::SkipExplicit(true)),
        SettingsEdit::Playback(PlaybackEdit::PauseWatchHistory(true)),
        SettingsEdit::Ui(UiEdit::Theme("nord".into())),
        SettingsEdit::Ui(UiEdit::LiveThemePath("/tmp/palette.json".into())),
        SettingsEdit::Ui(UiEdit::CustomThemes(CustomThemeOp::Put {
            name: "custom-nord".into(),
            theme,
        })),
        SettingsEdit::Ui(UiEdit::CustomThemes(CustomThemeOp::Delete {
            name: "custom-gone".into(),
        })),
        SettingsEdit::Ui(UiEdit::Language("tr".into())),
        SettingsEdit::Ui(UiEdit::ReduceAnimations(true)),
        SettingsEdit::Ui(UiEdit::FullscreenUsePlayerBar(true)),
        SettingsEdit::Ui(UiEdit::ShowSourceToggle(false)),
        SettingsEdit::Ui(UiEdit::ShowRowImages(false)),
        SettingsEdit::Ui(UiEdit::TitlebarMode(TitlebarMode::System)),
        SettingsEdit::Ui(UiEdit::PlayerBarPosition(PlayerBarPosition::Top)),
        SettingsEdit::Ui(UiEdit::UiStyle(UiStyle::Material3)),
        SettingsEdit::Ui(UiEdit::SettingsLayout(SettingsLayout::TopBar)),
        SettingsEdit::Ui(UiEdit::CustomFontPath("/fonts/a.ttf".into())),
        SettingsEdit::Background(BackgroundEdit::CoverArtBackground(true)),
        SettingsEdit::Background(BackgroundEdit::CoverArtDarkening(35)),
        SettingsEdit::Background(BackgroundEdit::CoverArtBlur(20)),
        SettingsEdit::Background(BackgroundEdit::CustomBackgroundPath("/bg.png".into())),
        SettingsEdit::Lyrics(LyricsEdit::PreferLocalLyrics(true)),
        SettingsEdit::Lyrics(LyricsEdit::EnableMusixmatchLyrics(true)),
        SettingsEdit::Lyrics(LyricsEdit::LyricsOffsetMs(-250)),
        SettingsEdit::Lyrics(LyricsEdit::LyricsOffsetAuto(false)),
        SettingsEdit::Lyrics(LyricsEdit::LyricsDepthBlur(false)),
        SettingsEdit::Lyrics(LyricsEdit::LyricsDepthBlurStrength(150)),
        SettingsEdit::Library(LibraryEdit::LocalSources(vec![SavedLocalSource {
            id: "local:a".into(),
            name: "a".into(),
            directories: vec!["/music".into()],
        }])),
        SettingsEdit::Library(LibraryEdit::AutoFetchCovers(true)),
        SettingsEdit::Library(LibraryEdit::CoverFetchStrategy(FetchStrategy::LastFmOnly)),
        SettingsEdit::Library(LibraryEdit::OfflineQuality(OfflineQuality::Kbps160)),
        SettingsEdit::Library(LibraryEdit::OfflineCacheEnabled(false)),
        SettingsEdit::Library(LibraryEdit::OfflineCacheMaxBytes(2_000_000_000)),
        SettingsEdit::Library(LibraryEdit::SyncIntervalMinutes(60)),
        SettingsEdit::Integrations(IntegrationsEdit::DiscordPresence(Some(false))),
        SettingsEdit::Integrations(IntegrationsEdit::DiscordPresenceSource(None)),
        SettingsEdit::Integrations(IntegrationsEdit::RadioRegistries(RadioRegistryOp::Put(
            registry,
        ))),
        SettingsEdit::Integrations(IntegrationsEdit::RadioRegistries(RadioRegistryOp::Remove {
            url: "https://example.org/gone.json".into(),
        })),
        SettingsEdit::Downloads(DownloadsEdit::DownloaderOutputDir("/downloads".into())),
        SettingsEdit::Downloads(DownloadsEdit::DownloaderOptions(DownloaderOptions {
            embed_metadata: false,
            embed_thumbnail: false,
            write_thumbnail: true,
            organize_by_album: false,
            overwrite_existing: true,
        })),
        SettingsEdit::Spotify(SpotifyEdit::SpotifyBrowser(Some("brave".into()))),
        SettingsEdit::Spotify(SpotifyEdit::SpotifyPreferActiveDevice(false)),
        SettingsEdit::System(SystemEdit::TracingEnabled(true)),
        SettingsEdit::System(SystemEdit::AutoCheckUpdates(false)),
        SettingsEdit::System(SystemEdit::MinimizeToTray(true)),
        SettingsEdit::System(SystemEdit::ConnectivityCheckUrls(vec![
            "https://check.example/up".into(),
        ])),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serialized_keys(settings: &Settings) -> Vec<String> {
        match serde_json::to_value(settings).expect("settings serialize") {
            serde_json::Value::Object(map) => map.keys().cloned().collect(),
            other => panic!("settings are not an object: {other}"),
        }
    }

    /// A field renamed on one side and not the other fails here rather than editing nothing.
    #[test]
    fn keys_and_stored_fields_are_one_to_one() {
        let mut stored = serialized_keys(&Settings::default());
        stored.sort();
        let mut names: Vec<&str> = SettingKey::iter().map(|key| key.name()).collect();
        names.sort();
        assert_eq!(names, stored, "one key per stored field, none left over");
        for name in &stored {
            let key = SettingKey::named(name).expect("a stored field has a key");
            assert_eq!(key.name(), name);
        }
    }

    #[test]
    fn the_samples_cover_every_key() {
        let mut keys: Vec<SettingKey> = edit_samples().iter().map(SettingKey::of).collect();
        keys.dedup();
        assert_eq!(keys, SettingKey::iter().collect::<Vec<_>>());
    }

    fn json_of(settings: &Settings) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(settings).expect("settings serialize") {
            serde_json::Value::Object(map) => map,
            other => panic!("settings are not an object: {other}"),
        }
    }

    #[test]
    fn apply_changes_exactly_the_named_field() {
        let mut base = Settings::default();
        base.ui
            .custom_themes
            .insert("custom-gone".into(), CustomTheme::default());
        base.integrations.radio_registries.push(RegistryEntry {
            url: "https://example.org/gone.json".into(),
            enabled: true,
            is_default: false,
        });
        let before = json_of(&base);
        for edit in edit_samples() {
            let name = SettingKey::of(&edit).name();
            let mut next = base.clone();
            next.apply(edit.clone());
            let after = json_of(&next);
            let moved: Vec<&String> = after
                .iter()
                .filter(|(key, value)| before.get(*key) != Some(value))
                .map(|(key, _)| key)
                .collect();
            assert_eq!(moved, vec![name], "{edit:?}");
            assert_eq!(base.changed_keys(&next), vec![SettingKey::of(&edit)]);
        }
    }

    #[test]
    fn collection_ops_change_only_their_member() {
        let theme = |op| SettingsEdit::Ui(UiEdit::CustomThemes(op));
        let registry = |op| SettingsEdit::Integrations(IntegrationsEdit::RadioRegistries(op));
        let mut settings = Settings::default();
        for name in ["a", "b"] {
            settings.apply(theme(CustomThemeOp::Put {
                name: name.into(),
                theme: CustomTheme::default(),
            }));
        }
        settings.apply(theme(CustomThemeOp::Delete { name: "a".into() }));
        assert_eq!(
            settings.ui.custom_themes.keys().collect::<Vec<_>>(),
            vec!["b"]
        );

        let url = "https://example.org/r.json".to_string();
        let entry = |enabled| RegistryEntry {
            url: url.clone(),
            enabled,
            is_default: false,
        };
        settings.apply(registry(RadioRegistryOp::Put(entry(true))));
        settings.apply(registry(RadioRegistryOp::Put(entry(false))));
        assert_eq!(settings.integrations.radio_registries.len(), 2);
        assert_eq!(settings.integrations.radio_registries[1], entry(false));
        settings.apply(registry(RadioRegistryOp::Remove { url: url.clone() }));
        settings.apply(registry(RadioRegistryOp::Remove {
            url: crate::DEFAULT_REGISTRY_URL.into(),
        }));
        assert_eq!(
            settings.integrations.radio_registries,
            crate::default_radio_registries()
        );
    }

    #[test]
    fn validation_rejects_out_of_range_values() {
        for edit in [
            SettingsEdit::Playback(PlaybackEdit::CrossfadeSeconds(13)),
            SettingsEdit::Playback(PlaybackEdit::VolumeScrollStep(0.0)),
            SettingsEdit::Playback(PlaybackEdit::VolumeScrollStep(0.75)),
            SettingsEdit::Playback(PlaybackEdit::VolumeScrollStep(f32::NAN)),
            SettingsEdit::Playback(PlaybackEdit::Equalizer(EqualizerSettings {
                bands: [20.0; 10],
                ..EqualizerSettings::default()
            })),
            SettingsEdit::Playback(PlaybackEdit::ReplayGain(ReplayGainSettings {
                preamp_db: 30.0,
                ..ReplayGainSettings::default()
            })),
            SettingsEdit::Background(BackgroundEdit::CoverArtDarkening(96)),
            SettingsEdit::Background(BackgroundEdit::CoverArtBlur(101)),
            SettingsEdit::Lyrics(LyricsEdit::LyricsDepthBlurStrength(9)),
            SettingsEdit::Lyrics(LyricsEdit::LyricsDepthBlurStrength(201)),
            SettingsEdit::Lyrics(LyricsEdit::LyricsOffsetMs(
                crate::LYRICS_OFFSET_LIMIT_MS + 1,
            )),
            SettingsEdit::Lyrics(LyricsEdit::LyricsOffsetMs(
                -crate::LYRICS_OFFSET_LIMIT_MS - 1,
            )),
            SettingsEdit::Library(LibraryEdit::SyncIntervalMinutes(
                crate::MAX_SYNC_INTERVAL_MINUTES + 1,
            )),
            SettingsEdit::Library(LibraryEdit::OfflineCacheMaxBytes(
                crate::MIN_OFFLINE_CACHE_BYTES - 1,
            )),
            SettingsEdit::Library(LibraryEdit::OfflineCacheMaxBytes(
                crate::MAX_OFFLINE_CACHE_BYTES + 1,
            )),
            SettingsEdit::System(SystemEdit::ConnectivityCheckUrls(vec![
                "cp.cloudflare.com".into(),
            ])),
            SettingsEdit::System(SystemEdit::ConnectivityCheckUrls(vec!["https://".into()])),
        ] {
            let mut settings = Settings::default();
            assert!(settings.try_apply(edit.clone()).is_err(), "{edit:?}");
            assert_eq!(settings, Settings::default(), "{edit:?} left a trace");
        }
        let mut settings = Settings::default();
        for edit in edit_samples() {
            settings.try_apply(edit.clone()).expect("an in-range edit");
        }
    }

    /// Values are only checked on edit, so a file that pins one out of range does not block its neighbours.
    #[test]
    fn an_out_of_range_neighbour_does_not_refuse_an_edit() {
        let mut settings = Settings::default();
        settings.playback.crossfade_seconds = 40;
        settings
            .try_apply(SettingsEdit::Playback(PlaybackEdit::ChannelMode(
                ChannelMode::Mono,
            )))
            .expect("the edited field is in range");
        assert_eq!(settings.playback.channel_mode, ChannelMode::Mono);
    }

    #[test]
    fn a_later_edit_supersedes_only_its_own_field_or_member() {
        let crossfade = |s| SettingsEdit::Playback(PlaybackEdit::CrossfadeSeconds(s));
        let put = |name: &str| {
            SettingsEdit::Ui(UiEdit::CustomThemes(CustomThemeOp::Put {
                name: name.into(),
                theme: CustomTheme::default(),
            }))
        };
        let delete = |name: &str| {
            SettingsEdit::Ui(UiEdit::CustomThemes(CustomThemeOp::Delete {
                name: name.into(),
            }))
        };
        let registry = |url: &str| {
            SettingsEdit::Integrations(IntegrationsEdit::RadioRegistries(RadioRegistryOp::Remove {
                url: url.into(),
            }))
        };
        assert!(crossfade(3).supersedes(&crossfade(2)));
        assert!(!crossfade(3).supersedes(&SettingsEdit::Ui(UiEdit::Theme("x".into()))));
        assert!(put("a").supersedes(&put("a")));
        assert!(!put("a").supersedes(&put("b")));
        assert!(delete("a").supersedes(&put("a")));
        assert!(!delete("a").supersedes(&put("b")));
        assert!(
            !registry("a").supersedes(&delete("a")),
            "another collection"
        );
        assert!(registry("a").supersedes(&registry("a")));
    }

    #[test]
    fn daemon_owned_keys_name_their_api() {
        let owned: Vec<&str> = SettingKey::iter()
            .filter(SettingKey::daemon_owned)
            .map(|key| key.name())
            .collect();
        assert_eq!(
            owned,
            vec![
                "local_sources",
                "auto_fetch_covers",
                "cover_fetch_strategy",
                "discord_presence",
                "discord_presence_source",
                "ytdlp_output_dir",
                "ytdlp_options",
                "spotify_browser",
                "spotify_prefer_active_device",
            ]
        );
    }
}
