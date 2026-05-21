use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum FetchStrategy {
    #[default]
    MusicBrainzFirst,
    LastFmFirst,
    MusicBrainzOnly,
    LastFmOnly,
}

// Maybe host on the website?
pub const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/Kopuz-org/kopuz/refs/heads/master/radio-registry/index.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegistryEntry {
    pub url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub is_default: bool,
}

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
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum MusicSource {
    #[default]
    Local,
    #[serde(alias = "Jellyfin")]
    Server,
}

impl MusicSource {
    pub fn is_server(&self) -> bool {
        matches!(self, Self::Server)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum MusicService {
    #[default]
    Jellyfin,
    #[serde(alias = "Navidrome")]
    Subsonic,
    Custom,
}

impl MusicService {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Jellyfin => "Jellyfin",
            Self::Subsonic => "Subsonic",
            Self::Custom => "Custom",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SortOrder {
    Title,
    Artist,
    Album,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ArtistViewOrder {
    Tracks,
    Albums,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ArtistPhotoSource {
    #[default]
    AlbumCover,
    ArtistPhoto,
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

    pub const fn gains(self) -> [f32; 5] {
        match self {
            Self::Flat | Self::Custom => [0.0, 0.0, 0.0, 0.0, 0.0],
            Self::BassBoost => [6.0, 4.5, 2.0, -0.5, -1.5],
            Self::TrebleBoost => [-1.5, -0.5, 0.5, 4.0, 6.0],
            Self::VocalBoost => [-2.0, 0.5, 3.5, 2.5, -0.5],
            Self::Loudness => [4.0, 2.0, 0.5, 2.5, 4.0],
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

fn default_eq_bands() -> [f32; 5] {
    [0.0, 0.0, 0.0, 0.0, 0.0]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EqualizerSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub preset: EqPreset,
    #[serde(default = "default_eq_bands")]
    pub bands: [f32; 5],
    #[serde(default)]
    pub preamp_db: f32,
}

impl EqualizerSettings {
    pub fn resolved_bands(&self) -> [f32; 5] {
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
    Modern,
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

fn default_recently_played_limit() -> usize {
    50
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
    #[serde(default)]
    pub active_source: MusicSource,
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
    #[serde(default = "default_sort_order")]
    pub sort_order: SortOrder,
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
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub reduce_animations: bool,
    #[serde(default = "default_auto_check_updates")]
    pub auto_check_updates: bool,
    #[serde(default = "default_show_source_toggle")]
    pub show_source_toggle: bool,
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
    pub recently_played: Vec<String>,
    #[serde(default)]
    pub recently_played_server: Vec<String>,
    #[serde(default)]
    pub listen_now_style: ListenNowStyle,
    #[serde(default)]
    pub artist_photo_source: ArtistPhotoSource,
    #[serde(default)]
    pub auto_fetch_covers: bool,
    #[serde(default)]
    pub cover_fetch_strategy: FetchStrategy,
    #[serde(default = "default_radio_registries")]
    pub radio_registries: Vec<RegistryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MusicServer {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub service: MusicService,
    pub access_token: Option<String>,
    pub user_id: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

pub type JellyfinServer = MusicServer;

impl MusicServer {
    pub fn new(name: String, url: String) -> Self {
        Self::new_with_service(name, url, MusicService::Jellyfin)
    }

    pub fn new_with_service(name: String, url: String, service: MusicService) -> Self {
        Self {
            name,
            // trim once here so every consumer gets a clean url to prevent broken links
            url: url.trim_end_matches('/').to_string(),
            service,
            access_token: None,
            user_id: None,
            id: Some(uuid::Uuid::new_v4().to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SavedServer {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub service: MusicService,
}

impl SavedServer {
    pub fn new(name: String, url: String, service: MusicService) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            url: url.trim_end_matches('/').to_string(),
            service,
        }
    }

    pub fn matches(&self, server: &MusicServer) -> bool {
        if let Some(sid) = server.id.as_ref() {
            if sid == &self.id {
                return true;
            }
        }
        self.url == server.url && self.service == server.service
    }
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

fn default_sort_order() -> SortOrder {
    SortOrder::Title
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
            active_source: MusicSource::Local,
            source_explicitly_set: false,
            music_directory: vec![music_directory],
            theme: default_theme(),
            device_id: default_device_id(),
            discord_presence: Some(true),
            sort_order: default_sort_order(),
            artist_view_order: default_artist_view_order(),
            listen_counts: HashMap::new(),
            musicbrainz_token: String::new(),
            lastfm_api_key: String::new(),
            lastfm_api_secret: String::new(),
            lastfm_session_key: String::new(),
            language: default_language(),
            reduce_animations: false,
            auto_check_updates: default_auto_check_updates(),
            show_source_toggle: default_show_source_toggle(),
            sidebar_order: default_sidebar_order(),
            volume: default_volume(),
            volume_scroll_step: default_volume_scroll_step(),
            crossfade_seconds: default_crossfade_seconds(),
            custom_themes: HashMap::new(),
            back_behavior: BackBehavior::RewindThenPrev,
            channel_mode: ChannelMode::Stereo,
            equalizer: EqualizerSettings::default(),
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
            recently_played: Vec::new(),
            recently_played_server: Vec::new(),
            listen_now_style: ListenNowStyle::default(),
            artist_photo_source: ArtistPhotoSource::AlbumCover,
            auto_fetch_covers: false,
            cover_fetch_strategy: FetchStrategy::default(),
            radio_registries: default_radio_registries(),
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
        if let Some(server) = self.server.as_mut() {
            if server.id.is_none() {
                server.id = Some(uuid::Uuid::new_v4().to_string());
            }
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
        if let Some(active) = &self.server {
            if active.id.as_deref() == Some(id) {
                self.server = None;
            }
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

    pub fn push_recent(&mut self, id: String, server: bool) {
        let list = if server {
            &mut self.recently_played_server
        } else {
            &mut self.recently_played
        };
        list.retain(|x| x != &id);
        list.insert(0, id);
        let limit = default_recently_played_limit();
        if list.len() > limit {
            list.truncate(limit);
        }
    }
}

impl Default for MusicServer {
    fn default() -> Self {
        Self {
            name: String::new(),
            url: String::new(),
            service: MusicService::Jellyfin,
            access_token: None,
            user_id: None,
            id: None,
        }
    }
}

impl AppConfig {
    pub fn active_service(&self) -> Option<MusicService> {
        if self.active_source.is_server() {
            self.server.as_ref().map(|server| server.service)
        } else {
            None
        }
    }

    pub fn uses_jellyfin_server(&self) -> bool {
        self.active_service() == Some(MusicService::Jellyfin)
    }

    pub fn load(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        match fs::read_to_string(path) {
            Ok(data) => match serde_json::from_str::<Self>(&data) {
                Ok(mut config) => {
                    config.migrate_home_sections();
                    config.migrate_sidebar_order();
                    config.migrate_registry_paths();
                    config.migrate_servers();
                    config
                }
                Err(e) => {
                    eprintln!("Failed to parse config at {:?}: {}", path, e);
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!("Failed to read config at {:?}: {}", path, e);
                Self::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!("Failed to create config directory {:?}: {}", parent, e);
                return Err(e);
            }
        }
        let data = match serde_json::to_string_pretty(self) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Failed to serialize config: {}", e);
                return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
            }
        };
        if let Err(e) = fs::write(path, data) {
            eprintln!("Failed to write config to {:?}: {}", path, e);
            return Err(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;
    use std::path::PathBuf;

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
}
