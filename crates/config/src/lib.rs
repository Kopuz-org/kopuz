//! Kopuz's user settings, grouped into topic sections that the TOML layers in [`store`] read and write.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

mod edit;
mod source;
pub mod store;
mod ui_prefs;
pub use edit::{
    BackgroundEdit, BackgroundKey, CustomThemeOp, DownloadsEdit, DownloadsKey, IntegrationsEdit,
    IntegrationsKey, LibraryEdit, LibraryKey, LyricsEdit, LyricsKey, PlaybackEdit, PlaybackKey,
    RadioRegistryOp, SettingKey, SettingsEdit, SpotifyEdit, SpotifyKey, SystemEdit, SystemKey,
    UiEdit, UiKey, edit_samples,
};
pub use source::{
    Browser, BrowserEngine, DEFAULT_LOCAL_ID, DEFAULT_LOCAL_NAME, JellyfinServer, MusicServer,
    MusicService, SavedLocalSource, SavedServer, Source,
};
pub use ui_prefs::UiPrefs;

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
/// How the URL downloader writes what it fetches. Options the earlier yt-dlp
/// downloader stored are ignored when read back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DownloaderOptions {
    #[serde(default = "default_true")]
    pub embed_metadata: bool,
    #[serde(default = "default_true")]
    pub embed_thumbnail: bool,
    #[serde(default)]
    pub write_thumbnail: bool,
    #[serde(default = "default_true")]
    pub organize_by_album: bool,
    #[serde(default)]
    pub overwrite_existing: bool,
}

impl Default for DownloaderOptions {
    fn default() -> Self {
        Self {
            embed_metadata: true,
            embed_thumbnail: true,
            write_thumbnail: false,
            organize_by_album: true,
            overwrite_existing: false,
        }
    }
}

fn default_depth_blur_strength() -> u8 {
    100
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DownloaderHistoryEntry {
    pub url: String,
    pub title: String,
    pub format: String,
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
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
    /// When the album's newest track was added.
    RecentlyAdded,
    TrackCount,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, garde::Validate)]
pub struct EqualizerSettings {
    #[serde(default)]
    #[garde(skip)]
    pub enabled: bool,
    #[serde(default)]
    #[garde(skip)]
    pub preset: EqPreset,
    #[serde(
        default = "default_eq_bands",
        deserialize_with = "deserialize_eq_bands"
    )]
    #[garde(inner(custom(finite), range(min = -12.0, max = 12.0)))]
    pub bands: [f32; 10],
    #[serde(default)]
    #[garde(custom(finite), range(min = -12.0, max = 6.0))]
    pub preamp_db: f32,
}

/// `range` lets NaN through, since every comparison with it is false.
fn finite(value: &f32, _: &()) -> garde::Result {
    match value.is_finite() {
        true => Ok(()),
        false => Err(garde::Error::new("not a finite number")),
    }
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

/// Which of a track's ReplayGain values to apply.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ReplayGainMode {
    #[default]
    Off,
    Track,
    Album,
    /// Album gain while the queue is walking through one album, track gain
    /// otherwise, so a shuffled mix levels song to song but an album keeps
    /// the loud/quiet relief its mastering intended.
    Auto,
}

impl ReplayGainMode {
    pub const ALL: &'static [Self] = &[Self::Off, Self::Track, Self::Album, Self::Auto];

    pub const fn value_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Track => "track",
            Self::Album => "album",
            Self::Auto => "auto",
        }
    }

    pub fn from_value_str(value: &str) -> Self {
        match value {
            "track" => Self::Track,
            "album" => Self::Album,
            "auto" => Self::Auto,
            _ => Self::Off,
        }
    }

    pub const fn i18n_key(self) -> &'static str {
        match self {
            Self::Off => "replay_gain_mode_off",
            Self::Track => "replay_gain_mode_track",
            Self::Album => "replay_gain_mode_album",
            Self::Auto => "replay_gain_mode_auto",
        }
    }
}

/// ReplayGain values carried by a track, in the units the tags use: gains in
/// dB relative to the reference loudness, peaks as a linear sample amplitude
/// where 1.0 is full scale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplayGainInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_gain_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_peak: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_gain_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_peak: Option<f32>,
    /// dB above the service's reference loudness (YouTube Music's `loudnessDb`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness_db: Option<f32>,
}

impl ReplayGainInfo {
    pub fn is_empty(&self) -> bool {
        self.track_gain_db.is_none() && self.album_gain_db.is_none()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, garde::Validate)]
pub struct ReplayGainSettings {
    #[serde(default)]
    #[garde(skip)]
    pub mode: ReplayGainMode,
    /// Hold the gain down far enough that the track's stored peak stays under
    /// full scale. Only does something for tracks that tagged a peak.
    #[serde(default = "default_prevent_clipping")]
    #[garde(skip)]
    pub prevent_clipping: bool,
    /// Applied on top of every resolved gain, tagged or not.
    #[serde(default)]
    #[garde(custom(finite), range(min = -GAIN_LIMIT_DB, max = GAIN_LIMIT_DB))]
    pub preamp_db: f32,
    /// Used in place of a gain for tracks that carry none.
    #[serde(default)]
    #[garde(custom(finite), range(min = -GAIN_LIMIT_DB, max = GAIN_LIMIT_DB))]
    pub fallback_gain_db: f32,
    /// Levels untagged tracks by their service loudness regardless of `mode`.
    #[serde(default = "default_normalize_loudness")]
    #[garde(skip)]
    pub normalize_loudness: bool,
}

fn default_prevent_clipping() -> bool {
    true
}

fn default_normalize_loudness() -> bool {
    true
}

impl Default for ReplayGainSettings {
    fn default() -> Self {
        Self {
            mode: ReplayGainMode::Off,
            prevent_clipping: true,
            preamp_db: 0.0,
            fallback_gain_db: 0.0,
            normalize_loudness: true,
        }
    }
}

/// -20 dB: a track measured louder than that is more likely a bad value.
const MIN_LOUDNESS_GAIN: f32 = 0.1;

/// Only attenuates, like YouTube's web player: boosting would need a limiter.
pub fn loudness_gain(loudness_db: f32) -> f32 {
    if !loudness_db.is_finite() {
        return 1.0;
    }
    10.0_f32
        .powf(-loudness_db / 20.0)
        .clamp(MIN_LOUDNESS_GAIN, 1.0)
}

/// Gains outside this range are a broken tag, not a mastering choice; ±15 dB
/// already covers everything ReplayGain scanners emit in practice.
const GAIN_LIMIT_DB: f32 = 15.0;

impl ReplayGainSettings {
    /// Linear factor to scale a track's samples by. `album_context` says the
    /// queue is currently walking an album, which is what [`ReplayGainMode::Auto`]
    /// switches on.
    pub fn linear_gain(&self, info: ReplayGainInfo, album_context: bool) -> f32 {
        self.linear_gain_with_fallback(info, ReplayGainInfo::default(), album_context)
    }

    /// Prefer stream tags to service metadata for each gain, keeping the peak
    /// from the same analysis even when falling back between track and album.
    pub fn linear_gain_with_fallback(
        &self,
        stream: ReplayGainInfo,
        service: ReplayGainInfo,
        album_context: bool,
    ) -> f32 {
        if self.normalize_loudness
            && stream.is_empty()
            && service.is_empty()
            && let Some(db) = stream.loudness_db.or(service.loudness_db)
        {
            return loudness_gain(db);
        }

        let prefer_album = match self.mode {
            ReplayGainMode::Off => return 1.0,
            ReplayGainMode::Track => false,
            ReplayGainMode::Album => true,
            ReplayGainMode::Auto => album_context,
        };

        // An album peak bounds every track on the album, so it can stand in for a
        // missing track peak; a track peak is too low to bound the album gain.
        let album = |info: ReplayGainInfo| info.album_gain_db.map(|db| (Some(db), info.album_peak));
        let track = |info: ReplayGainInfo| {
            info.track_gain_db
                .map(|db| (Some(db), info.track_peak.or(info.album_peak)))
        };
        let album = album(stream).or_else(|| album(service));
        let track = track(stream).or_else(|| track(service));
        let (gain_db, peak) = if prefer_album {
            album.or(track)
        } else {
            track.or(album)
        }
        .unwrap_or((
            None,
            stream
                .album_peak
                .or(stream.track_peak)
                .or(service.album_peak)
                .or(service.track_peak),
        ));

        let db = gain_db
            .filter(|db| db.is_finite())
            .unwrap_or(self.fallback_gain_db)
            .clamp(-GAIN_LIMIT_DB, GAIN_LIMIT_DB)
            + self.preamp_db.clamp(-GAIN_LIMIT_DB, GAIN_LIMIT_DB);

        let mut linear = 10.0_f32.powf(db / 20.0);
        if self.prevent_clipping
            && let Some(peak) = peak.filter(|peak| peak.is_finite() && *peak > 0.0)
        {
            linear = linear.min(1.0 / peak);
        }

        if linear.is_finite() {
            linear.clamp(0.0, 8.0)
        } else {
            1.0
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
}

/// A source with only one format per track ignores this.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum StreamQuality {
    Low,
    /// Never a format a subscription unlocks.
    Normal,
    #[default]
    High,
}

impl StreamQuality {
    pub const ALL: &'static [Self] = &[Self::Low, Self::Normal, Self::High];

    pub const fn value_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }

    pub fn from_value_str(value: &str) -> Self {
        match value {
            "low" => Self::Low,
            "normal" => Self::Normal,
            _ => Self::High,
        }
    }

    pub const fn i18n_key(self) -> &'static str {
        match self {
            Self::Low => "stream_quality_low",
            Self::Normal => "stream_quality_normal",
            Self::High => "stream_quality_high",
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
    Material3,
}

impl UiStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Vaxry => "vaxry",
            Self::Material3 => "material3",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SettingsLayout {
    #[default]
    Cd,
    TopBar,
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

/// Every setting, grouped by topic; each section is flattened, so the stored keys stay top-level.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(flatten)]
    pub playback: Playback,
    #[serde(flatten)]
    pub ui: Ui,
    #[serde(flatten)]
    pub background: Background,
    #[serde(flatten)]
    pub lyrics: Lyrics,
    #[serde(flatten)]
    pub library: Library,
    #[serde(flatten)]
    pub integrations: Integrations,
    #[serde(flatten)]
    pub downloads: Downloads,
    #[serde(flatten)]
    pub spotify: Spotify,
    #[serde(flatten)]
    pub system: System,
}

/// Each section's `garde` rules are what its settings UI clamps to; they run on an edit, never on load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(default)]
pub struct Playback {
    #[serde(default = "default_volume_scroll_step")]
    #[garde(custom(finite), range(min = 0.01, max = 0.5))]
    pub volume_scroll_step: f32,
    #[serde(default = "default_crossfade_seconds")]
    #[garde(range(max = 12))]
    pub crossfade_seconds: u8,
    #[serde(default)]
    #[garde(skip)]
    pub back_behavior: BackBehavior,
    #[serde(default)]
    #[garde(skip)]
    pub channel_mode: ChannelMode,
    #[serde(default)]
    #[garde(dive)]
    pub equalizer: EqualizerSettings,
    #[serde(default)]
    #[garde(dive)]
    pub replay_gain: ReplayGainSettings,
    #[serde(default)]
    #[garde(skip)]
    pub device_change_behavior: DeviceChangeBehavior,
    #[serde(default)]
    #[garde(skip)]
    pub sample_rate_mode: SampleRateMode,
    #[serde(default)]
    #[garde(skip)]
    pub stream_quality: StreamQuality,
    /// Only a source with track radio can do this.
    #[serde(default)]
    #[garde(skip)]
    pub autoplay_radio: bool,
    #[serde(default)]
    #[garde(skip)]
    pub skip_explicit: bool,
    #[serde(default)]
    #[garde(skip)]
    pub pause_watch_history: bool,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            volume_scroll_step: default_volume_scroll_step(),
            crossfade_seconds: default_crossfade_seconds(),
            back_behavior: BackBehavior::RewindThenPrev,
            channel_mode: ChannelMode::Stereo,
            equalizer: EqualizerSettings::default(),
            replay_gain: ReplayGainSettings::default(),
            device_change_behavior: DeviceChangeBehavior::Pause,
            sample_rate_mode: SampleRateMode::System,
            stream_quality: StreamQuality::High,
            autoplay_radio: false,
            skip_explicit: false,
            pause_watch_history: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(default)]
pub struct Ui {
    #[serde(default = "default_theme")]
    #[garde(skip)]
    pub theme: String,
    /// Palette file matugen or pywal writes, polled while the live theme is active; empty means whichever default location exists.
    #[serde(default)]
    #[garde(skip)]
    pub live_theme_path: String,
    #[serde(default)]
    #[garde(skip)]
    pub custom_themes: HashMap<String, CustomTheme>,
    #[serde(default = "default_language")]
    #[garde(skip)]
    pub language: String,
    #[serde(default)]
    #[garde(skip)]
    pub reduce_animations: bool,
    /// Fullscreen hides its own transport controls and lets the player bar act as the controller.
    #[serde(default)]
    #[garde(skip)]
    pub fullscreen_use_player_bar: bool,
    #[serde(default = "default_show_source_toggle")]
    #[garde(skip)]
    pub show_source_toggle: bool,
    #[serde(default = "default_true")]
    #[garde(skip)]
    pub show_row_images: bool,
    #[serde(default)]
    #[garde(skip)]
    pub titlebar_mode: TitlebarMode,
    #[serde(default)]
    #[garde(skip)]
    pub player_bar_position: PlayerBarPosition,
    #[serde(default)]
    #[garde(skip)]
    pub ui_style: UiStyle,
    #[serde(default)]
    #[garde(skip)]
    pub settings_layout: SettingsLayout,
    /// User-chosen font file (ttf/otf/woff/woff2) replacing the default JetBrains Mono stack; empty means unset.
    #[serde(default)]
    #[garde(skip)]
    pub custom_font_path: String,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            live_theme_path: String::new(),
            custom_themes: HashMap::new(),
            language: default_language(),
            reduce_animations: false,
            fullscreen_use_player_bar: false,
            show_source_toggle: default_show_source_toggle(),
            show_row_images: true,
            titlebar_mode: TitlebarMode::Custom,
            player_bar_position: PlayerBarPosition::Bottom,
            ui_style: UiStyle::Normal,
            settings_layout: SettingsLayout::Cd,
            custom_font_path: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(default)]
pub struct Background {
    /// Use the current track's cover as the app background, overriding the theme's (album-art gradient included).
    #[serde(default)]
    #[garde(skip)]
    pub cover_art_background: bool,
    /// How strongly the cover art background is darkened, in percent (0-95).
    #[serde(default = "default_cover_art_darkening")]
    #[garde(range(max = 95))]
    pub cover_art_darkening: u8,
    /// Blur radius of the cover art background, in pixels (0 = sharp).
    #[serde(default)]
    #[garde(range(max = 100))]
    pub cover_art_blur: u8,
    /// User-chosen background image over both the theme and the cover art, sharing their darkening and blur; empty means unset.
    #[serde(default)]
    #[garde(skip)]
    pub custom_background_path: String,
}

impl Default for Background {
    fn default() -> Self {
        Self {
            cover_art_background: false,
            cover_art_darkening: default_cover_art_darkening(),
            cover_art_blur: 0,
            custom_background_path: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(default)]
pub struct Lyrics {
    #[serde(default)]
    #[garde(skip)]
    pub prefer_local_lyrics: bool,
    #[serde(default)]
    #[garde(skip)]
    pub enable_musixmatch_lyrics: bool,
    /// Milliseconds to hold the lyrics behind the playback clock.
    #[serde(default, deserialize_with = "deserialize_lyrics_offset_ms")]
    #[garde(range(min = -LYRICS_OFFSET_LIMIT_MS, max = LYRICS_OFFSET_LIMIT_MS))]
    pub lyrics_offset_ms: i32,
    /// Take the offset from the backend's playback timestamp instead.
    #[serde(default = "default_true")]
    #[garde(skip)]
    pub lyrics_offset_auto: bool,
    /// Apple Music style depth-of-field: blur lines by distance from the active one instead of just dimming them.
    #[serde(default = "default_true")]
    #[garde(skip)]
    pub lyrics_depth_blur: bool,
    /// Scales the depth-of-field ramp, in percent (100 = the built-in ramp).
    #[serde(default = "default_depth_blur_strength")]
    #[garde(range(min = 10, max = 200))]
    pub lyrics_depth_blur_strength: u8,
}

impl Default for Lyrics {
    fn default() -> Self {
        Self {
            prefer_local_lyrics: false,
            enable_musixmatch_lyrics: false,
            lyrics_offset_ms: 0,
            lyrics_offset_auto: true,
            lyrics_depth_blur: true,
            lyrics_depth_blur_strength: default_depth_blur_strength(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(default)]
pub struct Library {
    /// Isolated filesystem libraries; the one under `DEFAULT_LOCAL_ID` is the install's own.
    #[garde(skip)]
    pub local_sources: Vec<SavedLocalSource>,
    #[serde(default)]
    #[garde(skip)]
    pub auto_fetch_covers: bool,
    #[serde(default)]
    #[garde(skip)]
    pub cover_fetch_strategy: FetchStrategy,
    #[serde(default)]
    #[garde(skip)]
    pub offline_quality: OfflineQuality,
    /// Minutes between the daemon's own library syncs and folder rescans; 0 turns them off.
    #[serde(default = "default_sync_interval_minutes")]
    #[garde(range(max = MAX_SYNC_INTERVAL_MINUTES))]
    pub sync_interval_minutes: u32,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            local_sources: vec![SavedLocalSource::default_library(vec![
                default_music_directory(),
            ])],
            auto_fetch_covers: false,
            cover_fetch_strategy: FetchStrategy::default(),
            offline_quality: OfflineQuality::default(),
            sync_interval_minutes: default_sync_interval_minutes(),
        }
    }
}

impl Library {
    pub fn remove_local_source(&mut self, id: &str) {
        self.local_sources.retain(|source| source.id != id);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(default)]
pub struct Integrations {
    #[serde(default = "default_discord_presence")]
    #[garde(skip)]
    pub discord_presence: Option<bool>,
    #[serde(default = "default_discord_presence_source")]
    #[garde(skip)]
    pub discord_presence_source: Option<bool>,
    #[serde(default = "default_radio_registries")]
    #[garde(skip)]
    pub radio_registries: Vec<RegistryEntry>,
}

impl Default for Integrations {
    fn default() -> Self {
        Self {
            discord_presence: Some(true),
            discord_presence_source: Some(true),
            radio_registries: default_radio_registries(),
        }
    }
}

impl Integrations {
    /// Put the default registry entry back when it is missing.
    pub fn migrate_registry_paths(&mut self) {
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

/// Stored under the names the yt-dlp downloader gave these, so existing settings carry over.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(default)]
pub struct Downloads {
    #[serde(default, rename = "ytdlp_output_dir")]
    #[garde(skip)]
    pub downloader_output_dir: String,
    #[serde(default, rename = "ytdlp_options")]
    #[garde(skip)]
    pub downloader_options: DownloaderOptions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(default)]
pub struct Spotify {
    /// Browser id hosting Spotify playback (`chrome`/`edge`/`brave`/`chromium`/`vivaldi`/`safari`); `None` picks the first available.
    #[serde(default)]
    #[garde(skip)]
    pub spotify_browser: Option<String>,
    /// Adopt a Connect device that is already playing (default) instead of starting on this app's own device.
    #[serde(default = "default_true")]
    #[garde(skip)]
    pub spotify_prefer_active_device: bool,
}

impl Default for Spotify {
    fn default() -> Self {
        Self {
            spotify_browser: None,
            spotify_prefer_active_device: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(default)]
pub struct System {
    /// Opt-in chrome/Perfetto trace, read once at startup so a change needs a restart.
    #[serde(default)]
    #[garde(skip)]
    pub tracing_enabled: bool,
    #[serde(default = "default_auto_check_updates")]
    #[garde(skip)]
    pub auto_check_updates: bool,
    /// Desktop-only: closing the window hides it to the tray instead of quitting, so playback keeps running.
    #[serde(default)]
    #[garde(skip)]
    pub minimize_to_tray: bool,
}

impl Default for System {
    fn default() -> Self {
        Self {
            tracing_enabled: false,
            auto_check_updates: default_auto_check_updates(),
            minimize_to_tray: false,
        }
    }
}

pub const DEFAULT_SYNC_INTERVAL_MINUTES: u32 = 24 * 60;

/// The longest interval the settings UI offers.
pub const MAX_SYNC_INTERVAL_MINUTES: u32 = 7 * 24 * 60;

fn default_sync_interval_minutes() -> u32 {
    DEFAULT_SYNC_INTERVAL_MINUTES
}

fn default_theme() -> String {
    "default".to_string()
}

fn default_discord_presence() -> Option<bool> {
    Some(true)
}

fn default_discord_presence_source() -> Option<bool> {
    Some(true)
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
        "downloader".to_string(),
    ]
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

/// Slider bound for `lyrics_offset_ms`; also enforced here since config files
/// and env vars can set it without going through the UI.
pub const LYRICS_OFFSET_LIMIT_MS: i32 = 1000;

fn deserialize_lyrics_offset_ms<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(i32::deserialize(deserializer)?.clamp(-LYRICS_OFFSET_LIMIT_MS, LYRICS_OFFSET_LIMIT_MS))
}

/// The folder a fresh install scans. `directories` resolves the XDG layout
/// against `$HOME`, which on Android is a private app path with no music in it,
/// so point at the shared media directory the platform actually uses.
fn default_music_directory() -> PathBuf {
    if cfg!(target_os = "android") {
        return PathBuf::from("/storage/emulated/0/Music");
    }
    let dirs = directories::UserDirs::new();
    music_directory_of(
        dirs.as_ref().and_then(directories::UserDirs::audio_dir),
        dirs.as_ref().map(directories::UserDirs::home_dir),
    )
}

/// The XDG music folder, else `Music` under the home directory, which is where it would be had user-dirs named it.
fn music_directory_of(audio_dir: Option<&Path>, home_dir: Option<&Path>) -> PathBuf {
    audio_dir
        .map(Path::to_path_buf)
        .or_else(|| home_dir.map(|home| home.join("Music")))
        .unwrap_or_else(|| PathBuf::from("Music"))
}

#[cfg(test)]
mod tests {
    use super::{
        EqualizerSettings, ReplayGainInfo, ReplayGainMode, ReplayGainSettings, Settings,
        SettingsLayout, loudness_gain,
    };

    fn tagged() -> ReplayGainInfo {
        ReplayGainInfo {
            track_gain_db: Some(-6.0),
            track_peak: Some(0.5),
            album_gain_db: Some(-3.0),
            album_peak: Some(0.9),
            loudness_db: None,
        }
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-4,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn off_leaves_the_signal_alone() {
        let settings = ReplayGainSettings::default();
        assert_eq!(settings.mode, ReplayGainMode::Off);
        assert_close(settings.linear_gain(tagged(), true), 1.0);
    }

    #[test]
    fn track_and_album_modes_pick_their_own_gain() {
        let track = ReplayGainSettings {
            mode: ReplayGainMode::Track,
            prevent_clipping: false,
            ..Default::default()
        };
        let album = ReplayGainSettings {
            mode: ReplayGainMode::Album,
            ..track
        };
        assert_close(
            track.linear_gain(tagged(), false),
            10.0_f32.powf(-6.0 / 20.0),
        );
        assert_close(
            album.linear_gain(tagged(), false),
            10.0_f32.powf(-3.0 / 20.0),
        );
    }

    #[test]
    fn auto_follows_the_album_context() {
        let settings = ReplayGainSettings {
            mode: ReplayGainMode::Auto,
            prevent_clipping: false,
            ..Default::default()
        };
        assert_close(
            settings.linear_gain(tagged(), true),
            10.0_f32.powf(-3.0 / 20.0),
        );
        assert_close(
            settings.linear_gain(tagged(), false),
            10.0_f32.powf(-6.0 / 20.0),
        );
    }

    #[test]
    fn preamp_and_fallback_apply_to_untagged_tracks() {
        let settings = ReplayGainSettings {
            mode: ReplayGainMode::Track,
            prevent_clipping: false,
            preamp_db: 2.0,
            fallback_gain_db: -4.0,
            normalize_loudness: true,
        };
        assert_close(
            settings.linear_gain(ReplayGainInfo::default(), false),
            10.0_f32.powf(-2.0 / 20.0),
        );
    }

    #[test]
    fn a_track_peak_never_bounds_the_album_gain() {
        let info = ReplayGainInfo {
            track_gain_db: Some(-6.0),
            track_peak: Some(0.5),
            album_gain_db: Some(6.0),
            album_peak: None,
            loudness_db: None,
        };
        let settings = ReplayGainSettings {
            mode: ReplayGainMode::Album,
            prevent_clipping: true,
            ..Default::default()
        };
        assert_close(settings.linear_gain(info, false), 10.0_f32.powf(6.0 / 20.0));
    }

    #[test]
    fn backing_values_keep_each_gain_with_its_own_peak() {
        let stream = ReplayGainInfo {
            track_gain_db: Some(6.0),
            ..Default::default()
        };
        let mut settings = ReplayGainSettings {
            mode: ReplayGainMode::Track,
            ..Default::default()
        };
        assert_close(
            settings.linear_gain_with_fallback(stream, tagged(), false),
            10.0_f32.powf(6.0 / 20.0),
        );
        settings.mode = ReplayGainMode::Album;
        assert_close(
            settings.linear_gain_with_fallback(stream, tagged(), true),
            10.0_f32.powf(-3.0 / 20.0),
        );
    }

    #[test]
    fn an_album_peak_from_the_same_analysis_bounds_track_gain() {
        let stream = ReplayGainInfo {
            track_gain_db: Some(6.0),
            album_peak: Some(0.8),
            ..Default::default()
        };
        let settings = ReplayGainSettings {
            mode: ReplayGainMode::Track,
            ..Default::default()
        };
        assert_close(settings.linear_gain(stream, false), 1.25);
        assert_close(
            settings.linear_gain_with_fallback(ReplayGainInfo::default(), stream, false),
            1.25,
        );
    }

    #[test]
    fn clip_prevention_caps_the_gain_at_the_stored_peak() {
        let info = ReplayGainInfo {
            track_gain_db: Some(6.0),
            track_peak: Some(0.8),
            ..Default::default()
        };
        let settings = ReplayGainSettings {
            mode: ReplayGainMode::Track,
            prevent_clipping: true,
            ..Default::default()
        };
        assert_close(settings.linear_gain(info, false), 1.25);

        let unclamped = ReplayGainSettings {
            prevent_clipping: false,
            ..settings
        };
        assert_close(
            unclamped.linear_gain(info, false),
            10.0_f32.powf(6.0 / 20.0),
        );
    }

    #[test]
    fn loudness_maps_to_attenuation_only() {
        assert_close(loudness_gain(6.0), 10.0_f32.powf(-6.0 / 20.0));
        assert_close(loudness_gain(0.0), 1.0);
        assert_close(loudness_gain(-4.0), 1.0);
        assert_close(loudness_gain(40.0), 0.1);
        assert_close(loudness_gain(f32::NAN), 1.0);
    }

    #[test]
    fn service_loudness_levels_a_track_without_replay_gain() {
        let loud = ReplayGainInfo {
            loudness_db: Some(9.0),
            ..Default::default()
        };
        let settings = ReplayGainSettings::default();
        assert!(settings.normalize_loudness);
        assert_close(
            settings.linear_gain_with_fallback(ReplayGainInfo::default(), loud, false),
            10.0_f32.powf(-9.0 / 20.0),
        );
        assert_close(
            settings.linear_gain_with_fallback(
                ReplayGainInfo::default(),
                ReplayGainInfo::default(),
                false,
            ),
            1.0,
        );

        let off = ReplayGainSettings {
            normalize_loudness: false,
            ..settings
        };
        assert_close(
            off.linear_gain_with_fallback(ReplayGainInfo::default(), loud, false),
            1.0,
        );

        // Tagged ReplayGain wins over the service measurement.
        let track = ReplayGainSettings {
            mode: ReplayGainMode::Track,
            prevent_clipping: false,
            ..settings
        };
        assert_close(
            track.linear_gain_with_fallback(tagged(), loud, false),
            10.0_f32.powf(-6.0 / 20.0),
        );
        assert_close(
            track.linear_gain_with_fallback(ReplayGainInfo::default(), loud, false),
            10.0_f32.powf(-9.0 / 20.0),
        );
    }

    #[test]
    fn settings_without_the_loudness_key_normalise() {
        let settings: ReplayGainSettings = serde_json::from_str(r#"{"mode":"Off"}"#).unwrap();
        assert!(settings.normalize_loudness);
    }

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
    fn settings_layout_is_backward_compatible_and_deserializes_top_bar() {
        let default_settings: Settings = serde_json::from_str("{}").unwrap();
        let top_bar_settings: Settings =
            serde_json::from_str(r#"{"settings_layout":"TopBar"}"#).unwrap();

        assert_eq!(default_settings.ui.settings_layout, SettingsLayout::Cd);
        assert_eq!(top_bar_settings.ui.settings_layout, SettingsLayout::TopBar);
    }

    /// Settings saved before the key existed keep syncing daily rather than never.
    #[test]
    fn a_settings_file_without_a_sync_interval_reads_as_a_day() {
        let settings: Settings = serde_json::from_str("{}").expect("empty settings parse");
        assert_eq!(settings.library.sync_interval_minutes, 24 * 60);
        let never: Settings =
            serde_json::from_str(r#"{"sync_interval_minutes":0}"#).expect("explicit zero parses");
        assert_eq!(never.library.sync_interval_minutes, 0);
    }

    /// Nothing stored reads as the defaults, the default folder source and its music folder included.
    #[test]
    fn an_empty_document_reads_as_the_defaults() {
        let settings: Settings = serde_json::from_str("{}").expect("empty settings parse");
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn with_no_user_dirs_file_the_music_folder_is_under_home() {
        use std::path::{Path, PathBuf};
        let home = Path::new("/home/someone");
        assert_eq!(
            super::music_directory_of(None, Some(home)),
            PathBuf::from("/home/someone/Music")
        );
        assert_eq!(
            super::music_directory_of(Some(Path::new("/srv/tunes")), Some(home)),
            PathBuf::from("/srv/tunes")
        );
    }
}
