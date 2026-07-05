use std::path::PathBuf;

use crate::{
    AppConfig, ArtistPhotoSource, ArtistViewOrder, BackBehavior, Browser, ChannelMode,
    EqualizerSettings, FetchStrategy, HomeSection, ListenNowStyle, MusicServer, MusicService,
    PlayerBarPosition, RegistryEntry, SortOrder, TitlebarMode, UiStyle,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerAuth {
    Token {
        token: Option<String>,
        user_id: Option<String>,
    },
    Browser {
        browser: Option<Browser>,
        token: Option<String>,
        user_id: Option<String>,
        anonymous: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackConfig {
    pub volume: f32,
    pub volume_scroll_step: f32,
    pub crossfade_seconds: u8,
    pub back_behavior: BackBehavior,
    pub channel_mode: ChannelMode,
    pub equalizer: EqualizerSettings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiConfig {
    pub theme: String,
    pub language: String,
    pub reduce_animations: bool,
    pub titlebar_mode: TitlebarMode,
    pub player_bar_position: PlayerBarPosition,
    pub ui_style: UiStyle,
    pub hero_height: u32,
    pub home_sections: Vec<HomeSection>,
    pub listen_now_style: ListenNowStyle,
    pub show_source_toggle: bool,
    pub sidebar_order: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LibraryConfig {
    pub music_directory: Vec<PathBuf>,
    pub sort_order: SortOrder,
    pub artist_view_order: ArtistViewOrder,
    pub artist_photo_source: ArtistPhotoSource,
    pub auto_fetch_covers: bool,
    pub cover_fetch_strategy: FetchStrategy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntegrationConfig {
    pub discord_presence: Option<bool>,
    pub discord_presence_paused: Option<bool>,
    pub discord_presence_source: Option<bool>,
    pub musicbrainz_token: String,
    pub lastfm_api_key: String,
    pub lastfm_api_secret: String,
    pub lastfm_session_key: String,
    pub librefm_api_key: String,
    pub librefm_api_secret: String,
    pub librefm_session_key: String,
    pub prefer_local_lyrics: bool,
    pub enable_musixmatch_lyrics: bool,
    pub radio_registries: Vec<RegistryEntry>,
}

impl MusicServer {
    pub fn auth(&self) -> ServerAuth {
        if self.service == MusicService::YtMusic || self.service == MusicService::SoundCloud {
            ServerAuth::Browser {
                browser: self.yt_browser,
                token: self.access_token.clone(),
                user_id: self.user_id.clone(),
                anonymous: self.yt_anonymous,
            }
        } else {
            ServerAuth::Token {
                token: self.access_token.clone(),
                user_id: self.user_id.clone(),
            }
        }
    }
}

impl AppConfig {
    pub fn playback(&self) -> PlaybackConfig {
        PlaybackConfig {
            volume: self.volume,
            volume_scroll_step: self.volume_scroll_step,
            crossfade_seconds: self.crossfade_seconds,
            back_behavior: self.back_behavior,
            channel_mode: self.channel_mode,
            equalizer: self.equalizer.clone(),
        }
    }

    pub fn ui(&self) -> UiConfig {
        UiConfig {
            theme: self.theme.clone(),
            language: self.language.clone(),
            reduce_animations: self.reduce_animations,
            titlebar_mode: self.titlebar_mode,
            player_bar_position: self.player_bar_position,
            ui_style: self.ui_style,
            hero_height: self.hero_height,
            home_sections: self.home_sections.clone(),
            listen_now_style: self.listen_now_style,
            show_source_toggle: self.show_source_toggle,
            sidebar_order: self.sidebar_order.clone(),
        }
    }

    pub fn library(&self) -> LibraryConfig {
        LibraryConfig {
            music_directory: self.music_directory.clone(),
            sort_order: self.sort_order.clone(),
            artist_view_order: self.artist_view_order.clone(),
            artist_photo_source: self.artist_photo_source,
            auto_fetch_covers: self.auto_fetch_covers,
            cover_fetch_strategy: self.cover_fetch_strategy,
        }
    }

    pub fn integrations(&self) -> IntegrationConfig {
        IntegrationConfig {
            discord_presence: self.discord_presence,
            discord_presence_paused: self.discord_presence_paused,
            discord_presence_source: self.discord_presence_source,
            musicbrainz_token: self.musicbrainz_token.clone(),
            lastfm_api_key: self.lastfm_api_key.clone(),
            lastfm_api_secret: self.lastfm_api_secret.clone(),
            lastfm_session_key: self.lastfm_session_key.clone(),
            librefm_api_key: self.librefm_api_key.clone(),
            librefm_api_secret: self.librefm_api_secret.clone(),
            librefm_session_key: self.librefm_session_key.clone(),
            prefer_local_lyrics: self.prefer_local_lyrics,
            enable_musixmatch_lyrics: self.enable_musixmatch_lyrics,
            radio_registries: self.radio_registries.clone(),
        }
    }
}
