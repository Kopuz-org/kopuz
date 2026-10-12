use crate::{
    AppConfig, ArtistViewOrder, BackBehavior, Browser, ChannelMode, EqualizerSettings,
    FetchStrategy, HomeSection, ListenNowStyle, MusicServer, MusicService, PlayerBarPosition,
    ReplayGainSettings, SettingsLayout, SortOrder, TitlebarMode, UiStyle,
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
    pub replay_gain: ReplayGainSettings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiConfig {
    pub theme: String,
    pub language: String,
    pub reduce_animations: bool,
    pub titlebar_mode: TitlebarMode,
    pub player_bar_position: PlayerBarPosition,
    pub ui_style: UiStyle,
    pub settings_layout: SettingsLayout,
    pub hero_height: u32,
    pub home_sections: Vec<HomeSection>,
    pub listen_now_style: ListenNowStyle,
    pub show_source_toggle: bool,
    pub sidebar_order: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LibraryConfig {
    pub sort_order: SortOrder,
    pub artist_view_order: ArtistViewOrder,
    pub auto_fetch_covers: bool,
    pub cover_fetch_strategy: FetchStrategy,
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
            replay_gain: self.replay_gain,
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
            settings_layout: self.settings_layout,
            hero_height: self.hero_height,
            home_sections: self.home_sections.clone(),
            listen_now_style: self.listen_now_style,
            show_source_toggle: self.show_source_toggle,
            sidebar_order: self.sidebar_order.clone(),
        }
    }

    pub fn library(&self) -> LibraryConfig {
        LibraryConfig {
            sort_order: self.sort_order.clone(),
            artist_view_order: self.artist_view_order.clone(),
            auto_fetch_covers: self.auto_fetch_covers,
            cover_fetch_strategy: self.cover_fetch_strategy,
        }
    }
}
