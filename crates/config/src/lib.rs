//! Configuration management for Kopuz: loads, saves, and migrates user settings
//! (audio, theme, media servers, shortcuts) from a JSON config file.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

mod source;
mod views;
pub use source::{
    Browser, JellyfinServer, MusicServer, MusicService, SavedLocalSource, SavedServer, Source,
};
pub use views::{IntegrationConfig, LibraryConfig, PlaybackConfig, ServerAuth, UiConfig};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum FetchStrategy {
    #[default]
    MusicBrainzFirst,
    LastFmFirst,
    MusicBrainzOnly,
    LastFmOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegistryEntry {
    pub url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub is_default: bool,
}

pub const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/Kopuz-org/kopuz/refs/heads/master/radio-registry/index.json";

pub fn default_radio_registries() -> Vec<RegistryEntry> {
    vec![RegistryEntry {
        url: DEFAULT_REGISTRY_URL.to_string(),
        enabled: true,
        is_default: true,
    }]
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct YtdlpOptions {
    #[serde(default = "default_true")]
    pub embed_metadata: bool,
    #[serde(default = "default_true")]
    pub embed_thumbnail: bool,
    #[serde(default)]
    pub postprocess_thumbnail_square: bool,
    #[serde(default)]
    pub embed_chapters: bool,
    #[serde(default)]
    pub embed_subs: bool,
    #[serde(default)]
    pub embed_info_json: bool,
    #[serde(default)]
    pub write_thumbnail: bool,
    #[serde(default)]
    pub write_description: bool,
    #[serde(default)]
    pub write_info_json: bool,
    #[serde(default)]
    pub write_subs: bool,
    #[serde(default)]
    pub write_auto_subs: bool,
    #[serde(default)]
    pub write_comments: bool,
    #[serde(default)]
    pub sponsorblock: bool,
    #[serde(default)]
    pub sponsorblock_mark: bool,
    #[serde(default)]
    pub split_chapters: bool,
    #[serde(default)]
    pub convert_thumbnail: String,
    #[serde(default)]
    pub no_playlist: bool,
    #[serde(default)]
    pub xattrs: bool,
    #[serde(default)]
    pub no_mtime: bool,
    #[serde(default)]
    pub rate_limit: String,
    #[serde(default)]
    pub cookies_from_browser: String,
    #[serde(default)]
    pub js_runtimes: String,
    #[serde(default = "default_audio_quality")]
    pub audio_quality: u8,
}

impl Default for YtdlpOptions {
    fn default() -> Self {
        Self {
            embed_metadata: true,
            embed_thumbnail: true,
            postprocess_thumbnail_square: false,
            embed_chapters: false,
            embed_subs: false,
            embed_info_json: false,
            write_thumbnail: false,
            write_description: false,
            write_info_json: false,
            write_subs: false,
            write_auto_subs: false,
            write_comments: false,
            sponsorblock: false,
            sponsorblock_mark: false,
            split_chapters: false,
            convert_thumbnail: String::new(),
            no_playlist: false,
            xattrs: false,
            no_mtime: false,
            rate_limit: String::new(),
            cookies_from_browser: String::new(),
            js_runtimes: String::new(),
            audio_quality: 0,
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_audio_quality() -> u8 {
    0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct YtdlpHistoryEntry {
    pub url: String,
    pub title: String,
    pub format: String,
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomTheme {
    pub name: String,
    pub vars: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SortOrder {
    Title,
    Artist,
    Album,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SortDirection {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AlbumSortField {
    Title,
    Artist,
    Year,
    Genre,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SortCriterion<F> {
    pub field: F,
    pub direction: SortDirection,
}

impl<F> SortCriterion<F> {
    pub fn new(field: F, direction: SortDirection) -> Self {
        Self { field, direction }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TrackSortField {
    Title,
    Artist,
    Album,
    Duration,
    DateAdded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ArtistSortField {
    Name,
    /// Track count (primary-artist credits).
    Tracks,
    /// Album count.
    Albums,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ArtistViewOrder {
    Tracks,
    Albums,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum AlbumViewMode {
    #[default]
    Grid,
    List,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum BackBehavior {
    #[default]
    RewindThenPrev,
    AlwaysPrev,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ChannelMode {
    #[default]
    Stereo,
    Mono,
    LeftOnly,
    RightOnly,
    SwapLeftRight,
}

impl ChannelMode {
    pub const ALL: &'static [Self] = &[
        Self::Stereo,
        Self::Mono,
        Self::LeftOnly,
        Self::RightOnly,
        Self::SwapLeftRight,
    ];

    pub const fn value_str(self) -> &'static str {
        match self {
            Self::Stereo => "stereo",
            Self::Mono => "mono",
            Self::LeftOnly => "left-only",
            Self::RightOnly => "right-only",
            Self::SwapLeftRight => "swap-left-right",
        }
    }

    pub fn from_value_str(value: &str) -> Self {
        match value {
            "mono" => Self::Mono,
            "left-only" => Self::LeftOnly,
            "right-only" => Self::RightOnly,
            "swap-left-right" => Self::SwapLeftRight,
            _ => Self::Stereo,
        }
    }
}

/// What playback does after the output device changes (unplugged headphones,
/// OS default switched) and the engine has migrated to the new device.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum DeviceChangeBehavior {
    /// Keep playing on the new device at the same position.
    Resume,
    /// Migrate to the new device but hold paused until the user resumes.
    #[default]
    Pause,
}

impl DeviceChangeBehavior {
    pub const ALL: &'static [Self] = &[Self::Resume, Self::Pause];

    pub const fn value_str(self) -> &'static str {
        match self {
            Self::Resume => "resume",
            Self::Pause => "pause",
        }
    }

    pub fn from_value_str(value: &str) -> Self {
        match value {
            "pause" => Self::Pause,
            _ => Self::Resume,
        }
    }
}

/// Which sample rate the output stream is opened at.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SampleRateMode {
    /// Keep the device at its default rate and resample every source to it.
    #[default]
    System,
    /// Reopen the device at each track's native rate (switches the DAC per
    /// track when rates differ).
    Source,
}

impl SampleRateMode {
    pub const ALL: &'static [Self] = &[Self::System, Self::Source];

    pub const fn value_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Source => "source",
        }
    }

    pub fn from_value_str(value: &str) -> Self {
        match value {
            "source" => Self::Source,
            _ => Self::System,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum EqPreset {
    #[default]
    Flat,
    BassBoost,
    TrebleBoost,
    VocalBoost,
    Loudness,
    Custom,
}

impl EqPreset {
    pub const fn all() -> [Self; 6] {
        [
            Self::Flat,
            Self::BassBoost,
            Self::TrebleBoost,
            Self::VocalBoost,
            Self::Loudness,
            Self::Custom,
        ]
    }

    pub const fn as_storage(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::BassBoost => "bass-boost",
            Self::TrebleBoost => "treble-boost",
            Self::VocalBoost => "vocal-boost",
            Self::Loudness => "loudness",
            Self::Custom => "custom",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::BassBoost => "Bass Boost",
            Self::TrebleBoost => "Treble Boost",
            Self::VocalBoost => "Vocal Boost",
            Self::Loudness => "Loudness",
            Self::Custom => "Custom",
        }
    }

    pub fn from_storage(value: &str) -> Self {
        match value {
            "bass-boost" => Self::BassBoost,
            "treble-boost" => Self::TrebleBoost,
            "vocal-boost" => Self::VocalBoost,
            "loudness" => Self::Loudness,
            "custom" => Self::Custom,
            _ => Self::Flat,
        }
    }

    pub const fn gains(self) -> [f32; 10] {
        match self {
            Self::Flat | Self::Custom => [0.0; 10],
            Self::BassBoost => [7.0, 6.5, 5.0, 3.5, 1.0, -0.5, -1.0, -1.5, -1.5, -1.5],
            Self::TrebleBoost => [-1.5, -1.5, -1.0, -0.5, 0.0, 0.5, 2.0, 4.0, 6.0, 6.5],
            Self::VocalBoost => [-2.5, -2.0, -1.5, 0.0, 2.5, 3.5, 3.0, 2.5, 0.5, -0.5],
            Self::Loudness => [5.0, 4.5, 3.0, 1.5, 0.5, 0.0, 1.0, 2.5, 4.0, 4.5],
        }
    }

    pub const fn default_preamp_db(self) -> Option<f32> {
        match self {
            Self::Flat => Some(0.0),
            Self::BassBoost => Some(-4.0),
            Self::TrebleBoost => Some(-2.0),
            Self::VocalBoost => Some(-1.5),
            Self::Loudness => Some(-5.0),
            Self::Custom => None,
        }
    }
}

fn default_eq_bands() -> [f32; 10] {
    [0.0; 10]
}

/// Slots in the current 10-band layout (32/64/125/250/500/1k/2k/4k/8k/16k Hz)
/// that the legacy 5-band layout (60/250/1k/4k/12k Hz) maps onto, picked as the
/// nearest frequency: 60→64, 250→250, 1k→1k, 4k→4k, 12k→16k.
const LEGACY_EQ_BAND_SLOTS: [usize; 5] = [1, 3, 5, 7, 9];

fn deserialize_eq_bands<'de, D>(deserializer: D) -> Result<[f32; 10], D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values: Vec<f32> = Vec::deserialize(deserializer)?;
    let mut out = [0.0_f32; 10];

    if values.len() == LEGACY_EQ_BAND_SLOTS.len() {
        // Migrate a saved 5-band custom preset onto the nearest 10-band slots so
        // existing boosts keep their original frequencies instead of shifting
        // down (e.g. a 1 kHz boost must not be reinterpreted as a 125 Hz boost).
        for (&slot, value) in LEGACY_EQ_BAND_SLOTS.iter().zip(values.iter().copied()) {
            out[slot] = value;
        }
    } else {
        for (slot, value) in out.iter_mut().zip(values.iter().copied()) {
            *slot = value;
        }
    }

    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EqualizerSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub preset: EqPreset,
    #[serde(
        default = "default_eq_bands",
        deserialize_with = "deserialize_eq_bands"
    )]
    pub bands: [f32; 10],
    #[serde(default)]
    pub preamp_db: f32,
}

impl EqualizerSettings {
    pub fn resolved_bands(&self) -> [f32; 10] {
        if self.preset == EqPreset::Custom {
            self.bands
        } else {
            self.preset.gains()
        }
    }
}

impl Default for EqualizerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            preset: EqPreset::Flat,
            bands: default_eq_bands(),
            preamp_db: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum OfflineQuality {
    Kbps128,
    Kbps160,
    Kbps192,
    Kbps256,
    #[default]
    Kbps320,
    Original,
}

impl OfflineQuality {
    pub const ALL: &'static [Self] = &[
        Self::Kbps128,
        Self::Kbps160,
        Self::Kbps192,
        Self::Kbps256,
        Self::Kbps320,
        Self::Original,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Kbps128 => "128 kbps",
            Self::Kbps160 => "160 kbps",
            Self::Kbps192 => "192 kbps",
            Self::Kbps256 => "256 kbps",
            Self::Kbps320 => "320 kbps",
            Self::Original => "Original",
        }
    }

    pub fn value_str(self) -> &'static str {
        match self {
            Self::Kbps128 => "128",
            Self::Kbps160 => "160",
            Self::Kbps192 => "192",
            Self::Kbps256 => "256",
            Self::Kbps320 => "320",
            Self::Original => "original",
        }
    }

    pub fn from_value_str(s: &str) -> Self {
        match s {
            "128" => Self::Kbps128,
            "160" => Self::Kbps160,
            "192" => Self::Kbps192,
            "256" => Self::Kbps256,
            "320" => Self::Kbps320,
            _ => Self::Original,
        }
    }

    pub fn jellyfin_bitrate_bps(self) -> Option<u32> {
        match self {
            Self::Kbps128 => Some(128_000),
            Self::Kbps160 => Some(160_000),
            Self::Kbps192 => Some(192_000),
            Self::Kbps256 => Some(256_000),
            Self::Kbps320 => Some(320_000),
            Self::Original => None,
        }
    }

    pub fn subsonic_max_bitrate_kbps(self) -> u32 {
        match self {
            Self::Kbps128 => 128,
            Self::Kbps160 => 160,
            Self::Kbps192 => 192,
            Self::Kbps256 => 256,
            Self::Kbps320 => 320,
            Self::Original => 0,
        }
    }

    pub fn file_extension(self) -> &'static str {
        match self {
            Self::Original => "bin",
            _ => "mp3",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum TitlebarMode {
    #[default]
    Custom,
    System,
    Off,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum PlayerBarPosition {
    #[default]
    Bottom,
    Top,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum UiStyle {
    #[default]
    Normal,
    #[serde(alias = "Modern")]
    Vaxry,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ListenNowStyle {
    #[default]
    List,
    Cards,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HomeSection {
    pub key: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

pub const HOME_SECTION_KEYS: &[&str] = &[
    "hero",
    "continue_listening",
    "listen_now",
    "top_artists",
    "new_releases",
    "made_for_you",
    "recently_added",
    "playlists",
];

pub fn default_home_sections() -> Vec<HomeSection> {
    HOME_SECTION_KEYS
        .iter()
        .map(|k| HomeSection {
            key: (*k).to_string(),
            enabled: true,
        })
        .collect()
}

fn default_hero_height() -> u32 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: Option<MusicServer>,
    #[serde(default)]
    pub servers: Vec<SavedServer>,
    /// Named, isolated filesystem libraries. The legacy `music_directory`
    /// remains the built-in Local source for backwards compatibility.
    #[serde(default)]
    pub local_sources: Vec<SavedLocalSource>,
    /// The active source: built-in Local, a named local library, or Server(id).
    /// `server` is hydrated only for the active remote source.
    #[serde(default)]
    pub active_source: Source,
    #[serde(default)]
    pub source_explicitly_set: bool,
    #[serde(default, deserialize_with = "deserialize_music_directories")]
    pub music_directory: Vec<PathBuf>,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_device_id")]
    pub device_id: String,
    #[serde(default = "default_discord_presence")]
    pub discord_presence: Option<bool>,
    #[serde(default = "default_discord_presence_paused")]
    pub discord_presence_paused: Option<bool>,
    #[serde(default = "default_discord_presence_source")]
    pub discord_presence_source: Option<bool>,
    #[serde(default = "default_sort_order")]
    pub sort_order: SortOrder,
    #[serde(default = "default_album_sort")]
    pub album_sort: Vec<SortCriterion<AlbumSortField>>,
    #[serde(default = "default_library_sort")]
    pub library_sort: Vec<SortCriterion<TrackSortField>>,
    #[serde(default = "default_album_sort")]
    pub artist_album_sort: Vec<SortCriterion<AlbumSortField>>,
    #[serde(default = "default_artist_sort")]
    pub artist_sort: Vec<SortCriterion<ArtistSortField>>,
    #[serde(default)]
    pub album_view_mode: AlbumViewMode,
    #[serde(default)]
    pub artist_album_view_mode: AlbumViewMode,
    #[serde(default)]
    pub artists_view_mode: AlbumViewMode,
    #[serde(default = "default_artist_view_order")]
    pub artist_view_order: ArtistViewOrder,
    #[serde(default)]
    pub listen_counts: HashMap<String, u64>,
    #[serde(default)]
    pub musicbrainz_token: String,
    #[serde(default)]
    pub lastfm_api_key: String,
    #[serde(default)]
    pub lastfm_api_secret: String,
    #[serde(default)]
    pub lastfm_session_key: String,
    #[serde(default)]
    pub librefm_api_key: String,
    #[serde(default)]
    pub librefm_api_secret: String,
    #[serde(default)]
    pub librefm_session_key: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub reduce_animations: bool,
    /// When enabled, fullscreen mode hides its own transport controls and lets
    /// the player bar act as the multimedia controller instead.
    #[serde(default)]
    pub fullscreen_use_player_bar: bool,
    /// Fullscreen: the Up Next / Lyrics side panel is collapsed. Toggled from
    /// the fullscreen UI itself, remembered across sessions.
    #[serde(default)]
    pub fullscreen_tabs_collapsed: bool,
    /// Use the current track's cover as the app background, overriding the
    /// active theme's background (including the album-art gradient).
    #[serde(default)]
    pub cover_art_background: bool,
    /// How strongly the cover art background is darkened, in percent (0-95).
    #[serde(default = "default_cover_art_darkening")]
    pub cover_art_darkening: u8,
    /// Blur radius of the cover art background, in pixels (0 = sharp).
    #[serde(default)]
    pub cover_art_blur: u8,
    /// Absolute path to a user-chosen image used as the app background,
    /// overriding both the theme background and the cover art background.
    /// Empty = unset. Shares the darkening/blur treatment with cover art.
    #[serde(default)]
    pub custom_background_path: String,
    /// Absolute path to a user-chosen font file (ttf/otf/woff/woff2) applied
    /// as the app's UI font, overriding the default JetBrains Mono stack.
    /// Empty = unset.
    #[serde(default)]
    pub custom_font_path: String,
    /// Opt-in chrome/Perfetto performance trace. Read at startup (the
    /// subscriber is built once), so a change needs a restart. Adds runtime
    /// overhead — surfaced with a warning in settings.
    #[serde(default)]
    pub tracing_enabled: bool,
    #[serde(default = "default_auto_check_updates")]
    pub auto_check_updates: bool,
    /// Desktop-only: when enabled, closing the window hides it to the system
    /// tray instead of quitting, so playback keeps running in the background.
    #[serde(default)]
    pub minimize_to_tray: bool,
    #[serde(default = "default_show_source_toggle")]
    pub show_source_toggle: bool,
    #[serde(default = "default_true")]
    pub show_row_images: bool,
    #[serde(default = "default_sidebar_order")]
    pub sidebar_order: Vec<String>,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default = "default_volume_scroll_step")]
    pub volume_scroll_step: f32,
    #[serde(default = "default_crossfade_seconds")]
    pub crossfade_seconds: u8,
    #[serde(default)]
    pub custom_themes: HashMap<String, CustomTheme>,
    #[serde(default)]
    pub back_behavior: BackBehavior,
    #[serde(default)]
    pub channel_mode: ChannelMode,
    #[serde(default)]
    pub equalizer: EqualizerSettings,
    #[serde(default)]
    pub device_change_behavior: DeviceChangeBehavior,
    #[serde(default)]
    pub sample_rate_mode: SampleRateMode,
    #[serde(default)]
    pub ytdlp_output_dir: String,
    #[serde(default)]
    pub ytdlp_options: YtdlpOptions,
    #[serde(default)]
    pub ytdlp_history: Vec<YtdlpHistoryEntry>,
    #[serde(default)]
    pub titlebar_mode: TitlebarMode,
    #[serde(default)]
    pub offline_quality: OfflineQuality,
    #[serde(default)]
    pub offline_tracks: HashMap<String, String>,
    #[serde(default)]
    pub player_bar_position: PlayerBarPosition,
    #[serde(default)]
    pub ui_style: UiStyle,
    #[serde(default = "default_hero_height")]
    pub hero_height: u32,
    #[serde(default = "default_home_sections")]
    pub home_sections: Vec<HomeSection>,
    #[serde(default)]
    pub listen_now_style: ListenNowStyle,
    #[serde(default)]
    pub auto_fetch_covers: bool,
    #[serde(default)]
    pub cover_fetch_strategy: FetchStrategy,
    #[serde(default = "default_radio_registries")]
    pub radio_registries: Vec<RegistryEntry>,
    /// Station manifests (JSON) pinned from the radio browser.
    #[serde(default)]
    pub pinned_stations: Vec<String>,
    #[serde(default)]
    pub prefer_local_lyrics: bool,
    #[serde(default)]
    pub enable_musixmatch_lyrics: bool,
}

fn default_theme() -> String {
    "default".to_string()
}

fn default_device_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn default_discord_presence() -> Option<bool> {
    Some(true)
}

fn default_discord_presence_paused() -> Option<bool> {
    Some(true)
}

fn default_discord_presence_source() -> Option<bool> {
    Some(true)
}

fn default_sort_order() -> SortOrder {
    SortOrder::Title
}

fn default_album_sort() -> Vec<SortCriterion<AlbumSortField>> {
    vec![SortCriterion::new(
        AlbumSortField::Title,
        SortDirection::Asc,
    )]
}

fn default_library_sort() -> Vec<SortCriterion<TrackSortField>> {
    vec![SortCriterion::new(
        TrackSortField::Title,
        SortDirection::Asc,
    )]
}

fn default_artist_sort() -> Vec<SortCriterion<ArtistSortField>> {
    vec![SortCriterion::new(
        ArtistSortField::Name,
        SortDirection::Asc,
    )]
}

fn default_artist_view_order() -> ArtistViewOrder {
    ArtistViewOrder::Tracks
}

fn default_show_source_toggle() -> bool {
    true
}

fn default_auto_check_updates() -> bool {
    true
}

fn default_cover_art_darkening() -> u8 {
    60
}

pub fn default_sidebar_order() -> Vec<String> {
    vec![
        "home".to_string(),
        "search".to_string(),
        "library".to_string(),
        "albums".to_string(),
        "artists".to_string(),
        "playlists".to_string(),
        "favorites".to_string(),
        "radio".to_string(),
        "activity".to_string(),
        "ytdlp".to_string(),
    ]
}

fn default_volume() -> f32 {
    1.0
}

fn default_volume_scroll_step() -> f32 {
    0.05
}

fn default_crossfade_seconds() -> u8 {
    0
}

fn default_language() -> String {
    "en".to_string()
}

fn deserialize_music_directories<'de, D>(deserializer: D) -> Result<Vec<PathBuf>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(PathBuf),
        Many(Vec<PathBuf>),
    }
    match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(p) => Ok(vec![p]),
        OneOrMany::Many(v) => Ok(v),
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        let music_directory = directories::UserDirs::new()
            .and_then(|u| u.audio_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("./assets"));
        Self {
            server: None,
            servers: Vec::new(),
            local_sources: Vec::new(),
            active_source: Source::Local,
            source_explicitly_set: false,
            music_directory: vec![music_directory],
            theme: default_theme(),
            device_id: default_device_id(),
            discord_presence: Some(true),
            discord_presence_paused: Some(true),
            discord_presence_source: Some(true),
            sort_order: default_sort_order(),
            album_sort: default_album_sort(),
            library_sort: default_library_sort(),
            artist_album_sort: default_album_sort(),
            artist_sort: default_artist_sort(),
            album_view_mode: AlbumViewMode::Grid,
            artist_album_view_mode: AlbumViewMode::Grid,
            artists_view_mode: AlbumViewMode::Grid,
            artist_view_order: default_artist_view_order(),
            listen_counts: HashMap::new(),
            musicbrainz_token: String::new(),
            lastfm_api_key: String::new(),
            lastfm_api_secret: String::new(),
            lastfm_session_key: String::new(),
            librefm_api_key: String::new(),
            librefm_api_secret: String::new(),
            librefm_session_key: String::new(),
            language: default_language(),
            reduce_animations: false,
            fullscreen_use_player_bar: false,
            fullscreen_tabs_collapsed: false,
            cover_art_background: false,
            cover_art_darkening: default_cover_art_darkening(),
            cover_art_blur: 0,
            custom_background_path: String::new(),
            custom_font_path: String::new(),
            tracing_enabled: false,
            auto_check_updates: default_auto_check_updates(),
            minimize_to_tray: false,
            show_source_toggle: default_show_source_toggle(),
            show_row_images: true,
            sidebar_order: default_sidebar_order(),
            volume: default_volume(),
            volume_scroll_step: default_volume_scroll_step(),
            crossfade_seconds: default_crossfade_seconds(),
            custom_themes: HashMap::new(),
            back_behavior: BackBehavior::RewindThenPrev,
            channel_mode: ChannelMode::Stereo,
            equalizer: EqualizerSettings::default(),
            device_change_behavior: DeviceChangeBehavior::Pause,
            sample_rate_mode: SampleRateMode::System,
            ytdlp_output_dir: String::new(),
            ytdlp_options: YtdlpOptions::default(),
            ytdlp_history: Vec::new(),
            titlebar_mode: TitlebarMode::Custom,
            offline_quality: OfflineQuality::default(),
            offline_tracks: HashMap::new(),
            player_bar_position: PlayerBarPosition::Bottom,
            ui_style: UiStyle::Normal,
            hero_height: default_hero_height(),
            home_sections: default_home_sections(),
            listen_now_style: ListenNowStyle::default(),
            auto_fetch_covers: false,
            cover_fetch_strategy: FetchStrategy::default(),
            radio_registries: default_radio_registries(),
            pinned_stations: Vec::new(),
            prefer_local_lyrics: false,
            enable_musixmatch_lyrics: false,
        }
    }
}

impl AppConfig {
    pub fn migrate_home_sections(&mut self) {
        let allowed: std::collections::HashSet<&&str> = HOME_SECTION_KEYS.iter().collect();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let existing = std::mem::take(&mut self.home_sections);
        for s in existing {
            if allowed.contains(&s.key.as_str()) && seen.insert(s.key.clone()) {
                self.home_sections.push(s);
            }
        }
        for key in HOME_SECTION_KEYS {
            if !seen.contains(*key) {
                self.home_sections.push(HomeSection {
                    key: (*key).to_string(),
                    enabled: true,
                });
            }
        }
    }

    pub fn migrate_servers(&mut self) {
        if let Some(server) = self.server.as_mut()
            && server.id.is_none()
        {
            server.id = Some(uuid::Uuid::new_v4().to_string());
        }
        if let Some(server) = self.server.clone() {
            let already = self.servers.iter().any(|s| s.matches(&server));
            if !already {
                let id = server
                    .id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                self.servers.push(SavedServer {
                    id,
                    name: server.name.clone(),
                    url: server.url.clone(),
                    service: server.service,
                    yt_browser: server.yt_browser,
                    yt_anonymous: server.yt_anonymous,
                });
            }
        }
    }

    pub fn add_saved_server(&mut self, entry: SavedServer) {
        if !self.servers.iter().any(|s| s.id == entry.id) {
            self.servers.push(entry);
        }
    }

    pub fn remove_saved_server(&mut self, id: &str) {
        self.servers.retain(|s| s.id != id);
        if let Some(active) = &self.server
            && active.id.as_deref() == Some(id)
        {
            self.server = None;
        }
    }

    pub fn add_local_source(&mut self, source: SavedLocalSource) {
        if !self.local_sources.iter().any(|saved| saved.id == source.id) {
            self.local_sources.push(source);
        }
    }

    pub fn remove_local_source(&mut self, id: &str) {
        self.local_sources.retain(|source| source.id != id);
        if self.active_source.local_library_id() == Some(id) {
            self.clear_active_server();
        }
    }

    pub fn find_saved_server(&self, id: &str) -> Option<&SavedServer> {
        self.servers.iter().find(|s| s.id == id)
    }

    pub fn migrate_sidebar_order(&mut self) {
        let all_keys = default_sidebar_order();
        for key in &all_keys {
            if !self.sidebar_order.iter().any(|k| k == key) {
                self.sidebar_order.push(key.to_string());
            }
        }
        self.sidebar_order.retain(|k| all_keys.contains(k));
    }

    pub fn migrate_registry_paths(&mut self) {
        // Ensure the default registry entry is always present
        if !self.radio_registries.iter().any(|r| r.is_default) {
            self.radio_registries.insert(
                0,
                RegistryEntry {
                    url: DEFAULT_REGISTRY_URL.to_string(),
                    enabled: true,
                    is_default: true,
                },
            );
        }
    }
}

impl AppConfig {
    pub fn clear_active_server(&mut self) {
        self.active_source = Source::Local;
        self.server = None;
        self.source_explicitly_set = true;
    }

    pub fn set_active_local_source(&mut self, source: Source) {
        debug_assert!(source.is_local());
        self.active_source = source;
        self.server = None;
        self.source_explicitly_set = true;
    }

    pub fn set_active_server_snapshot(&mut self, server: MusicServer) {
        let source = server.id.clone().map_or(Source::Local, Source::Server);
        self.active_source = source;
        self.server = Some(server);
        self.source_explicitly_set = true;
    }

    pub fn active_service(&self) -> Option<MusicService> {
        self.active_source.server_id()?;
        self.server.as_ref().map(|server| server.service)
    }

    pub fn uses_jellyfin_server(&self) -> bool {
        self.active_service() == Some(MusicService::Jellyfin)
    }

    /// The server to activate when toggling into server mode: the current server
    /// if already on one, else the first saved server. `None` ⇒ no servers, so
    /// the toggle is a no-op.
    pub fn server_toggle_target(&self) -> Option<Source> {
        self.active_source
            .server_id()
            .map(String::from)
            .or_else(|| self.servers.first().map(|s| s.id.clone()))
            .map(Source::Server)
    }
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, BackBehavior, Browser, EqualizerSettings, MusicServer, ServerAuth};
    use std::path::PathBuf;

    #[test]
    fn legacy_five_band_custom_eq_migrates_to_nearest_slots() {
        // A custom preset saved by the old 5-band UI: boosts at 60/250/1k/4k/12k Hz.
        let json = r#"{
            "enabled": true,
            "preset": "Custom",
            "bands": [3.0, 0.0, 5.0, -2.0, 4.0],
            "preamp_db": 0.0
        }"#;

        let eq: EqualizerSettings = serde_json::from_str(json).unwrap();

        // Each legacy value lands on the nearest 10-band slot (64/250/1k/4k/16k Hz),
        // not the first five slots (which are now 32/64/125/250/500 Hz).
        assert_eq!(
            eq.bands,
            [0.0, 3.0, 0.0, 0.0, 0.0, 5.0, 0.0, -2.0, 0.0, 4.0]
        );
    }

    #[test]
    fn modern_ten_band_eq_round_trips_unchanged() {
        let bands = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let json = format!(
            r#"{{ "enabled": true, "preset": "Custom", "bands": {bands:?}, "preamp_db": 0.0 }}"#
        );

        let eq: EqualizerSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(eq.bands, bands);
    }

    #[test]
    fn config_deserializes_legacy_single_music_directory() {
        let json = r#"{
            "music_directory": "/music"
        }"#;

        let config: AppConfig = serde_json::from_str(json).unwrap();

        assert_eq!(config.music_directory, vec![PathBuf::from("/music")]);
    }

    #[test]
    fn config_deserializes_multiple_music_directories() {
        let json = r#"{
            "music_directory": ["/music", "/archive"]
        }"#;

        let config: AppConfig = serde_json::from_str(json).unwrap();

        assert_eq!(
            config.music_directory,
            vec![PathBuf::from("/music"), PathBuf::from("/archive")]
        );
    }

    #[test]
    fn playback_view_projects_playback_fields() {
        let mut config = AppConfig {
            volume: 0.4,
            crossfade_seconds: 5,
            back_behavior: BackBehavior::AlwaysPrev,
            ..AppConfig::default()
        };
        config.equalizer.enabled = true;

        let playback = config.playback();

        assert_eq!(playback.volume, 0.4);
        assert_eq!(playback.crossfade_seconds, 5);
        assert_eq!(playback.back_behavior, BackBehavior::AlwaysPrev);
        assert!(playback.equalizer.enabled);
    }

    #[test]
    fn browser_signin_server_auth_is_typed() {
        let mut server = MusicServer::new_with_service(
            "yt".to_string(),
            "https://music.youtube.com".to_string(),
            super::MusicService::YtMusic,
        );
        server.yt_browser = Some(Browser::Brave);
        server.yt_anonymous = true;

        assert_eq!(
            server.auth(),
            ServerAuth::Browser {
                browser: Some(Browser::Brave),
                token: None,
                user_id: None,
                anonymous: true,
            }
        );
    }
}
