//! `config::Settings` <-> the flat wire `Config` message, every setting included.

use crate::*;

pub fn back_behavior_to_proto(value: config::BackBehavior) -> BackBehavior {
    match value {
        config::BackBehavior::RewindThenPrev => BackBehavior::RewindThenPrev,
        config::BackBehavior::AlwaysPrev => BackBehavior::AlwaysPrev,
    }
}

pub fn back_behavior_from_proto(value: i32) -> config::BackBehavior {
    match BackBehavior::try_from(value).unwrap_or(BackBehavior::Unspecified) {
        BackBehavior::RewindThenPrev => config::BackBehavior::RewindThenPrev,
        BackBehavior::AlwaysPrev => config::BackBehavior::AlwaysPrev,
        BackBehavior::Unspecified => config::BackBehavior::default(),
    }
}

pub fn channel_mode_to_proto(value: config::ChannelMode) -> ChannelMode {
    match value {
        config::ChannelMode::Stereo => ChannelMode::Stereo,
        config::ChannelMode::Mono => ChannelMode::Mono,
        config::ChannelMode::LeftOnly => ChannelMode::LeftOnly,
        config::ChannelMode::RightOnly => ChannelMode::RightOnly,
        config::ChannelMode::SwapLeftRight => ChannelMode::SwapLeftRight,
    }
}

pub fn channel_mode_from_proto(value: i32) -> config::ChannelMode {
    match ChannelMode::try_from(value).unwrap_or(ChannelMode::Unspecified) {
        ChannelMode::Stereo => config::ChannelMode::Stereo,
        ChannelMode::Mono => config::ChannelMode::Mono,
        ChannelMode::LeftOnly => config::ChannelMode::LeftOnly,
        ChannelMode::RightOnly => config::ChannelMode::RightOnly,
        ChannelMode::SwapLeftRight => config::ChannelMode::SwapLeftRight,
        ChannelMode::Unspecified => config::ChannelMode::default(),
    }
}

pub fn stream_quality_to_proto(value: config::StreamQuality) -> StreamQuality {
    match value {
        config::StreamQuality::Low => StreamQuality::Low,
        config::StreamQuality::Normal => StreamQuality::Normal,
        config::StreamQuality::High => StreamQuality::High,
    }
}

pub fn stream_quality_from_proto(value: i32) -> config::StreamQuality {
    match StreamQuality::try_from(value).unwrap_or(StreamQuality::Unspecified) {
        StreamQuality::Low => config::StreamQuality::Low,
        StreamQuality::Normal => config::StreamQuality::Normal,
        StreamQuality::High => config::StreamQuality::High,
        StreamQuality::Unspecified => config::StreamQuality::default(),
    }
}

pub fn device_change_behavior_to_proto(
    value: config::DeviceChangeBehavior,
) -> DeviceChangeBehavior {
    match value {
        config::DeviceChangeBehavior::Resume => DeviceChangeBehavior::Resume,
        config::DeviceChangeBehavior::Pause => DeviceChangeBehavior::Pause,
    }
}

pub fn device_change_behavior_from_proto(value: i32) -> config::DeviceChangeBehavior {
    match DeviceChangeBehavior::try_from(value).unwrap_or(DeviceChangeBehavior::Unspecified) {
        DeviceChangeBehavior::Resume => config::DeviceChangeBehavior::Resume,
        DeviceChangeBehavior::Pause => config::DeviceChangeBehavior::Pause,
        DeviceChangeBehavior::Unspecified => config::DeviceChangeBehavior::default(),
    }
}

pub fn fetch_strategy_to_proto(value: config::FetchStrategy) -> FetchStrategy {
    match value {
        config::FetchStrategy::MusicBrainzFirst => FetchStrategy::MusicBrainzFirst,
        config::FetchStrategy::LastFmFirst => FetchStrategy::LastFmFirst,
        config::FetchStrategy::MusicBrainzOnly => FetchStrategy::MusicBrainzOnly,
        config::FetchStrategy::LastFmOnly => FetchStrategy::LastFmOnly,
    }
}

pub fn fetch_strategy_from_proto(value: i32) -> config::FetchStrategy {
    match FetchStrategy::try_from(value).unwrap_or(FetchStrategy::Unspecified) {
        FetchStrategy::MusicBrainzFirst => config::FetchStrategy::MusicBrainzFirst,
        FetchStrategy::LastFmFirst => config::FetchStrategy::LastFmFirst,
        FetchStrategy::MusicBrainzOnly => config::FetchStrategy::MusicBrainzOnly,
        FetchStrategy::LastFmOnly => config::FetchStrategy::LastFmOnly,
        FetchStrategy::Unspecified => config::FetchStrategy::default(),
    }
}

pub fn offline_quality_to_proto(value: config::OfflineQuality) -> OfflineQuality {
    match value {
        config::OfflineQuality::Kbps128 => OfflineQuality::Kbps128,
        config::OfflineQuality::Kbps160 => OfflineQuality::Kbps160,
        config::OfflineQuality::Kbps192 => OfflineQuality::Kbps192,
        config::OfflineQuality::Kbps256 => OfflineQuality::Kbps256,
        config::OfflineQuality::Kbps320 => OfflineQuality::Kbps320,
        config::OfflineQuality::Original => OfflineQuality::Original,
    }
}

pub fn offline_quality_from_proto(value: i32) -> config::OfflineQuality {
    match OfflineQuality::try_from(value).unwrap_or(OfflineQuality::Unspecified) {
        OfflineQuality::Kbps128 => config::OfflineQuality::Kbps128,
        OfflineQuality::Kbps160 => config::OfflineQuality::Kbps160,
        OfflineQuality::Kbps192 => config::OfflineQuality::Kbps192,
        OfflineQuality::Kbps256 => config::OfflineQuality::Kbps256,
        OfflineQuality::Kbps320 => config::OfflineQuality::Kbps320,
        OfflineQuality::Original => config::OfflineQuality::Original,
        OfflineQuality::Unspecified => config::OfflineQuality::default(),
    }
}

pub fn player_bar_position_to_proto(value: config::PlayerBarPosition) -> PlayerBarPosition {
    match value {
        config::PlayerBarPosition::Bottom => PlayerBarPosition::Bottom,
        config::PlayerBarPosition::Top => PlayerBarPosition::Top,
    }
}

pub fn player_bar_position_from_proto(value: i32) -> config::PlayerBarPosition {
    match PlayerBarPosition::try_from(value).unwrap_or(PlayerBarPosition::Unspecified) {
        PlayerBarPosition::Bottom => config::PlayerBarPosition::Bottom,
        PlayerBarPosition::Top => config::PlayerBarPosition::Top,
        PlayerBarPosition::Unspecified => config::PlayerBarPosition::default(),
    }
}

pub fn sample_rate_mode_to_proto(value: config::SampleRateMode) -> SampleRateMode {
    match value {
        config::SampleRateMode::System => SampleRateMode::System,
        config::SampleRateMode::Source => SampleRateMode::Source,
    }
}

pub fn sample_rate_mode_from_proto(value: i32) -> config::SampleRateMode {
    match SampleRateMode::try_from(value).unwrap_or(SampleRateMode::Unspecified) {
        SampleRateMode::System => config::SampleRateMode::System,
        SampleRateMode::Source => config::SampleRateMode::Source,
        SampleRateMode::Unspecified => config::SampleRateMode::default(),
    }
}

pub fn settings_layout_to_proto(value: config::SettingsLayout) -> SettingsLayout {
    match value {
        config::SettingsLayout::Cd => SettingsLayout::Cd,
        config::SettingsLayout::TopBar => SettingsLayout::TopBar,
    }
}

pub fn settings_layout_from_proto(value: i32) -> config::SettingsLayout {
    match SettingsLayout::try_from(value).unwrap_or(SettingsLayout::Unspecified) {
        SettingsLayout::Cd => config::SettingsLayout::Cd,
        SettingsLayout::TopBar => config::SettingsLayout::TopBar,
        SettingsLayout::Unspecified => config::SettingsLayout::default(),
    }
}

pub fn titlebar_mode_to_proto(value: config::TitlebarMode) -> TitlebarMode {
    match value {
        config::TitlebarMode::Custom => TitlebarMode::Custom,
        config::TitlebarMode::System => TitlebarMode::System,
        config::TitlebarMode::Off => TitlebarMode::Off,
    }
}

pub fn titlebar_mode_from_proto(value: i32) -> config::TitlebarMode {
    match TitlebarMode::try_from(value).unwrap_or(TitlebarMode::Unspecified) {
        TitlebarMode::Custom => config::TitlebarMode::Custom,
        TitlebarMode::System => config::TitlebarMode::System,
        TitlebarMode::Off => config::TitlebarMode::Off,
        TitlebarMode::Unspecified => config::TitlebarMode::default(),
    }
}

pub fn ui_style_to_proto(value: config::UiStyle) -> UiStyle {
    match value {
        config::UiStyle::Normal => UiStyle::Normal,
        config::UiStyle::Vaxry => UiStyle::Vaxry,
        config::UiStyle::Material3 => UiStyle::Material3,
    }
}

pub fn ui_style_from_proto(value: i32) -> config::UiStyle {
    match UiStyle::try_from(value).unwrap_or(UiStyle::Unspecified) {
        UiStyle::Normal => config::UiStyle::Normal,
        UiStyle::Vaxry => config::UiStyle::Vaxry,
        UiStyle::Material3 => config::UiStyle::Material3,
        UiStyle::Unspecified => config::UiStyle::default(),
    }
}

pub fn eq_preset_to_proto(value: config::EqPreset) -> EqPreset {
    match value {
        config::EqPreset::Flat => EqPreset::Flat,
        config::EqPreset::BassBoost => EqPreset::BassBoost,
        config::EqPreset::TrebleBoost => EqPreset::TrebleBoost,
        config::EqPreset::VocalBoost => EqPreset::VocalBoost,
        config::EqPreset::Loudness => EqPreset::Loudness,
        config::EqPreset::Custom => EqPreset::Custom,
    }
}

pub fn eq_preset_from_proto(value: i32) -> config::EqPreset {
    match EqPreset::try_from(value).unwrap_or(EqPreset::Unspecified) {
        EqPreset::Flat => config::EqPreset::Flat,
        EqPreset::BassBoost => config::EqPreset::BassBoost,
        EqPreset::TrebleBoost => config::EqPreset::TrebleBoost,
        EqPreset::VocalBoost => config::EqPreset::VocalBoost,
        EqPreset::Loudness => config::EqPreset::Loudness,
        EqPreset::Custom => config::EqPreset::Custom,
        EqPreset::Unspecified => config::EqPreset::default(),
    }
}

pub fn track_sort_field_to_proto(value: config::TrackSortField) -> TrackSortField {
    match value {
        config::TrackSortField::Title => TrackSortField::Title,
        config::TrackSortField::Artist => TrackSortField::Artist,
        config::TrackSortField::Album => TrackSortField::Album,
        config::TrackSortField::Duration => TrackSortField::Duration,
        config::TrackSortField::DateAdded => TrackSortField::DateAdded,
    }
}

pub fn track_sort_field_from_proto(value: i32) -> config::TrackSortField {
    match TrackSortField::try_from(value).unwrap_or(TrackSortField::Unspecified) {
        TrackSortField::Title => config::TrackSortField::Title,
        TrackSortField::Artist => config::TrackSortField::Artist,
        TrackSortField::Album => config::TrackSortField::Album,
        TrackSortField::Duration => config::TrackSortField::Duration,
        TrackSortField::DateAdded => config::TrackSortField::DateAdded,
        TrackSortField::Unspecified => config::TrackSortField::Title,
    }
}

pub fn source_ref_to_proto(value: &config::Source) -> SourceRef {
    let kind = match value {
        config::Source::LocalLibrary(id) => source_ref::Kind::Folders(id.clone()),
        config::Source::Server(id) => source_ref::Kind::Server(id.clone()),
    };
    SourceRef { kind: Some(kind) }
}

pub fn source_ref_from_proto(value: Option<&SourceRef>) -> config::Source {
    match value.and_then(|value| value.kind.as_ref()) {
        Some(source_ref::Kind::Folders(id)) => config::Source::LocalLibrary(id.clone()),
        Some(source_ref::Kind::Server(id)) => config::Source::Server(id.clone()),
        None => config::Source::default(),
    }
}

pub fn saved_folder_source_to_proto(value: &config::SavedLocalSource) -> SavedFolderSource {
    SavedFolderSource {
        id: value.id.clone(),
        name: value.name.clone(),
        directories: value
            .directories
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    }
}

pub fn saved_folder_source_from_proto(value: &SavedFolderSource) -> config::SavedLocalSource {
    config::SavedLocalSource {
        id: value.id.clone(),
        name: value.name.clone(),
        directories: value
            .directories
            .iter()
            .map(std::path::PathBuf::from)
            .collect(),
    }
}

pub fn registry_entry_to_proto(value: &config::RegistryEntry) -> RegistryEntry {
    RegistryEntry {
        url: value.url.clone(),
        enabled: value.enabled,
        is_default: value.is_default,
    }
}

pub fn registry_entry_from_proto(value: &RegistryEntry) -> config::RegistryEntry {
    config::RegistryEntry {
        url: value.url.clone(),
        enabled: value.enabled,
        is_default: value.is_default,
    }
}

pub fn custom_theme_to_proto(value: &config::CustomTheme) -> CustomTheme {
    CustomTheme {
        name: value.name.clone(),
        vars: value.vars.clone().into_iter().collect(),
    }
}

pub fn custom_theme_from_proto(value: &CustomTheme) -> config::CustomTheme {
    config::CustomTheme {
        name: value.name.clone(),
        vars: value.vars.clone().into_iter().collect(),
    }
}
pub fn equalizer_to_proto(value: &config::EqualizerSettings) -> EqualizerSettings {
    EqualizerSettings {
        enabled: value.enabled,
        preset: eq_preset_to_proto(value.preset) as i32,
        bands: value.bands.to_vec(),
        preamp_db: value.preamp_db,
    }
}

/// The band count is fixed at ten. A message carrying anything else is
/// malformed, so the defaults stand rather than a partially applied curve.
pub fn equalizer_from_proto(value: Option<&EqualizerSettings>) -> config::EqualizerSettings {
    let Some(value) = value else {
        return config::EqualizerSettings::default();
    };
    let bands = <[f32; 10]>::try_from(value.bands.as_slice())
        .unwrap_or_else(|_| config::EqualizerSettings::default().bands);
    config::EqualizerSettings {
        enabled: value.enabled,
        preset: eq_preset_from_proto(value.preset),
        bands,
        preamp_db: value.preamp_db,
    }
}

pub fn replay_gain_mode_to_proto(value: config::ReplayGainMode) -> ReplayGainMode {
    match value {
        config::ReplayGainMode::Off => ReplayGainMode::Off,
        config::ReplayGainMode::Track => ReplayGainMode::Track,
        config::ReplayGainMode::Album => ReplayGainMode::Album,
        config::ReplayGainMode::Auto => ReplayGainMode::Auto,
    }
}

pub fn replay_gain_mode_from_proto(value: i32) -> config::ReplayGainMode {
    match ReplayGainMode::try_from(value).unwrap_or(ReplayGainMode::Unspecified) {
        ReplayGainMode::Off => config::ReplayGainMode::Off,
        ReplayGainMode::Track => config::ReplayGainMode::Track,
        ReplayGainMode::Album => config::ReplayGainMode::Album,
        ReplayGainMode::Auto => config::ReplayGainMode::Auto,
        ReplayGainMode::Unspecified => config::ReplayGainMode::default(),
    }
}

pub fn replay_gain_to_proto(value: config::ReplayGainSettings) -> ReplayGainSettings {
    ReplayGainSettings {
        mode: replay_gain_mode_to_proto(value.mode) as i32,
        prevent_clipping: value.prevent_clipping,
        preamp_db: value.preamp_db,
        fallback_gain_db: value.fallback_gain_db,
        normalize_loudness: Some(value.normalize_loudness),
    }
}

pub fn replay_gain_from_proto(value: Option<&ReplayGainSettings>) -> config::ReplayGainSettings {
    let Some(value) = value else {
        return config::ReplayGainSettings::default();
    };
    config::ReplayGainSettings {
        mode: replay_gain_mode_from_proto(value.mode),
        prevent_clipping: value.prevent_clipping,
        preamp_db: value.preamp_db,
        fallback_gain_db: value.fallback_gain_db,
        normalize_loudness: value.normalize_loudness.unwrap_or(true),
    }
}

pub fn downloader_options_to_proto(value: &config::DownloaderOptions) -> DownloaderOptions {
    DownloaderOptions {
        embed_metadata: value.embed_metadata,
        embed_thumbnail: value.embed_thumbnail,
        write_thumbnail: value.write_thumbnail,
        organize_by_album: value.organize_by_album,
        overwrite_existing: value.overwrite_existing,
    }
}

pub fn downloader_options_from_proto(
    value: Option<&DownloaderOptions>,
) -> config::DownloaderOptions {
    let Some(value) = value else {
        return config::DownloaderOptions::default();
    };
    config::DownloaderOptions {
        embed_metadata: value.embed_metadata,
        embed_thumbnail: value.embed_thumbnail,
        write_thumbnail: value.write_thumbnail,
        organize_by_album: value.organize_by_album,
        overwrite_existing: value.overwrite_existing,
    }
}

pub fn config_to_proto(value: &config::Settings) -> Config {
    let config::Settings {
        playback,
        ui,
        background,
        lyrics,
        library,
        integrations,
        downloads,
        spotify,
        system,
    } = value;
    Config {
        folder_sources: library
            .local_sources
            .iter()
            .map(saved_folder_source_to_proto)
            .collect(),
        theme: ui.theme.clone(),
        live_theme_path: ui.live_theme_path.clone(),
        language: ui.language.clone(),
        reduce_animations: ui.reduce_animations,
        fullscreen_use_player_bar: ui.fullscreen_use_player_bar,
        cover_art_background: background.cover_art_background,
        cover_art_darkening: u32::from(background.cover_art_darkening),
        cover_art_blur: u32::from(background.cover_art_blur),
        custom_background_path: background.custom_background_path.clone(),
        custom_font_path: ui.custom_font_path.clone(),
        tracing_enabled: system.tracing_enabled,
        auto_check_updates: system.auto_check_updates,
        minimize_to_tray: system.minimize_to_tray,
        connectivity_check_urls: Some(StringList {
            values: system.connectivity_check_urls.clone(),
        }),
        show_source_toggle: ui.show_source_toggle,
        show_row_images: ui.show_row_images,
        volume_scroll_step: playback.volume_scroll_step,
        crossfade_seconds: u32::from(playback.crossfade_seconds),
        custom_themes: ui
            .custom_themes
            .iter()
            .map(|(k, v)| (k.clone(), custom_theme_to_proto(v)))
            .collect(),
        back_behavior: back_behavior_to_proto(playback.back_behavior) as i32,
        channel_mode: channel_mode_to_proto(playback.channel_mode) as i32,
        equalizer: Some(equalizer_to_proto(&playback.equalizer)),
        replay_gain: Some(replay_gain_to_proto(playback.replay_gain)),
        device_change_behavior: device_change_behavior_to_proto(playback.device_change_behavior)
            as i32,
        sample_rate_mode: sample_rate_mode_to_proto(playback.sample_rate_mode) as i32,
        titlebar_mode: titlebar_mode_to_proto(ui.titlebar_mode) as i32,
        offline_quality: offline_quality_to_proto(library.offline_quality) as i32,
        stream_quality: stream_quality_to_proto(playback.stream_quality) as i32,
        autoplay_radio: playback.autoplay_radio,
        skip_explicit: playback.skip_explicit,
        pause_watch_history: playback.pause_watch_history,
        player_bar_position: player_bar_position_to_proto(ui.player_bar_position) as i32,
        ui_style: ui_style_to_proto(ui.ui_style) as i32,
        settings_layout: settings_layout_to_proto(ui.settings_layout) as i32,
        auto_fetch_covers: library.auto_fetch_covers,
        cover_fetch_strategy: fetch_strategy_to_proto(library.cover_fetch_strategy) as i32,
        radio_registries: integrations
            .radio_registries
            .iter()
            .map(registry_entry_to_proto)
            .collect(),
        prefer_local_lyrics: lyrics.prefer_local_lyrics,
        enable_musixmatch_lyrics: lyrics.enable_musixmatch_lyrics,
        lyrics_offset_ms: lyrics.lyrics_offset_ms,
        lyrics_offset_auto: lyrics.lyrics_offset_auto,
        lyrics_depth_blur: lyrics.lyrics_depth_blur,
        lyrics_depth_blur_strength: u32::from(lyrics.lyrics_depth_blur_strength),
        sync_interval_minutes: Some(library.sync_interval_minutes),
        offline_cache_enabled: Some(library.offline_cache_enabled),
        offline_cache_max_bytes: Some(library.offline_cache_max_bytes),
        discord_presence: integrations.discord_presence,
        discord_presence_source: integrations.discord_presence_source,
        spotify_browser: spotify.spotify_browser.clone(),
        spotify_prefer_active_device: spotify.spotify_prefer_active_device,
        downloader_output_dir: downloads.downloader_output_dir.clone(),
        downloader_options: Some(downloader_options_to_proto(&downloads.downloader_options)),
    }
}

pub fn config_from_proto(value: &Config) -> config::Settings {
    config::Settings {
        playback: config::Playback {
            volume_scroll_step: value.volume_scroll_step,
            crossfade_seconds: value.crossfade_seconds.min(u32::from(u8::MAX)) as u8,
            back_behavior: back_behavior_from_proto(value.back_behavior),
            channel_mode: channel_mode_from_proto(value.channel_mode),
            equalizer: equalizer_from_proto(value.equalizer.as_ref()),
            replay_gain: replay_gain_from_proto(value.replay_gain.as_ref()),
            device_change_behavior: device_change_behavior_from_proto(value.device_change_behavior),
            sample_rate_mode: sample_rate_mode_from_proto(value.sample_rate_mode),
            stream_quality: stream_quality_from_proto(value.stream_quality),
            autoplay_radio: value.autoplay_radio,
            skip_explicit: value.skip_explicit,
            pause_watch_history: value.pause_watch_history,
        },
        ui: config::Ui {
            theme: value.theme.clone(),
            live_theme_path: value.live_theme_path.clone(),
            custom_themes: value
                .custom_themes
                .iter()
                .map(|(k, v)| (k.clone(), custom_theme_from_proto(v)))
                .collect(),
            language: value.language.clone(),
            reduce_animations: value.reduce_animations,
            fullscreen_use_player_bar: value.fullscreen_use_player_bar,
            show_source_toggle: value.show_source_toggle,
            show_row_images: value.show_row_images,
            titlebar_mode: titlebar_mode_from_proto(value.titlebar_mode),
            player_bar_position: player_bar_position_from_proto(value.player_bar_position),
            ui_style: ui_style_from_proto(value.ui_style),
            settings_layout: settings_layout_from_proto(value.settings_layout),
            custom_font_path: value.custom_font_path.clone(),
        },
        background: config::Background {
            cover_art_background: value.cover_art_background,
            cover_art_darkening: value.cover_art_darkening.min(u32::from(u8::MAX)) as u8,
            cover_art_blur: value.cover_art_blur.min(u32::from(u8::MAX)) as u8,
            custom_background_path: value.custom_background_path.clone(),
        },
        lyrics: config::Lyrics {
            prefer_local_lyrics: value.prefer_local_lyrics,
            enable_musixmatch_lyrics: value.enable_musixmatch_lyrics,
            lyrics_offset_ms: value.lyrics_offset_ms,
            lyrics_offset_auto: value.lyrics_offset_auto,
            lyrics_depth_blur: value.lyrics_depth_blur,
            lyrics_depth_blur_strength: value.lyrics_depth_blur_strength.min(u32::from(u8::MAX))
                as u8,
        },
        library: config::Library {
            local_sources: value
                .folder_sources
                .iter()
                .map(saved_folder_source_from_proto)
                .collect(),
            auto_fetch_covers: value.auto_fetch_covers,
            cover_fetch_strategy: fetch_strategy_from_proto(value.cover_fetch_strategy),
            offline_quality: offline_quality_from_proto(value.offline_quality),
            offline_cache_enabled: value.offline_cache_enabled.unwrap_or(true),
            offline_cache_max_bytes: value
                .offline_cache_max_bytes
                .unwrap_or(config::DEFAULT_OFFLINE_CACHE_MAX_BYTES),
            sync_interval_minutes: value
                .sync_interval_minutes
                .unwrap_or(config::DEFAULT_SYNC_INTERVAL_MINUTES),
        },
        integrations: config::Integrations {
            discord_presence: value.discord_presence,
            discord_presence_source: value.discord_presence_source,
            radio_registries: value
                .radio_registries
                .iter()
                .map(registry_entry_from_proto)
                .collect(),
        },
        downloads: config::Downloads {
            downloader_output_dir: value.downloader_output_dir.clone(),
            downloader_options: downloader_options_from_proto(value.downloader_options.as_ref()),
        },
        spotify: config::Spotify {
            spotify_browser: value.spotify_browser.clone(),
            spotify_prefer_active_device: value.spotify_prefer_active_device,
        },
        system: config::System {
            tracing_enabled: value.tracing_enabled,
            auto_check_updates: value.auto_check_updates,
            minimize_to_tray: value.minimize_to_tray,
            connectivity_check_urls: match &value.connectivity_check_urls {
                Some(list) => list.values.clone(),
                None => config::System::default().connectivity_check_urls,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn appearance_survives_the_wire_round_trip() {
        for ui_style in [
            config::UiStyle::Normal,
            config::UiStyle::Vaxry,
            config::UiStyle::Material3,
        ] {
            let mut original = config::Settings::default();
            original.ui.ui_style = ui_style;
            original.ui.theme = "system".into();
            let bytes = config_to_proto(&original).encode_to_vec();
            let restored = config_from_proto(&Config::decode(bytes.as_slice()).unwrap());
            assert_eq!(restored.ui.ui_style, ui_style);
            assert_eq!(restored.ui.theme, original.ui.theme);
        }
        assert_eq!(ui_style_from_proto(1), config::UiStyle::Normal);
        assert_eq!(ui_style_from_proto(2), config::UiStyle::Vaxry);
    }

    #[test]
    fn playback_settings_survive_the_wire_round_trip() {
        for stream_quality in config::StreamQuality::ALL.iter().copied() {
            let mut original = config::Settings::default();
            original.playback.stream_quality = stream_quality;
            original.playback.autoplay_radio = true;
            original.playback.skip_explicit = true;
            original.playback.pause_watch_history = true;
            let bytes = config_to_proto(&original).encode_to_vec();
            let restored = config_from_proto(&Config::decode(bytes.as_slice()).unwrap()).playback;
            assert_eq!(restored.stream_quality, stream_quality);
            assert!(restored.autoplay_radio && restored.skip_explicit);
            assert!(restored.pause_watch_history);
        }
        // A peer that predates the field sends nothing, which reads as today's behaviour.
        let restored = config_from_proto(&Config::default()).playback;
        assert_eq!(restored.stream_quality, config::StreamQuality::High);
        assert!(!restored.autoplay_radio && !restored.skip_explicit);
    }

    #[test]
    fn connectivity_checks_survive_the_wire_and_an_older_peer_keeps_the_defaults() {
        for urls in [Vec::new(), vec!["https://check.example/up".to_string()]] {
            let mut original = config::Settings::default();
            original.system.connectivity_check_urls = urls.clone();
            let bytes = config_to_proto(&original).encode_to_vec();
            let restored = config_from_proto(&Config::decode(bytes.as_slice()).unwrap());
            assert_eq!(restored.system.connectivity_check_urls, urls);
        }
        let older = config_from_proto(&Config::default());
        assert_eq!(
            older.system.connectivity_check_urls,
            config::DEFAULT_CONNECTIVITY_CHECK_URLS
        );
    }

    #[test]
    fn the_offline_cache_settings_survive_the_wire_and_an_older_peer_keeps_the_defaults() {
        let mut original = config::Settings::default();
        original.library.offline_cache_enabled = false;
        original.library.offline_cache_max_bytes = 2_000_000_000;
        let bytes = config_to_proto(&original).encode_to_vec();
        let restored = config_from_proto(&Config::decode(bytes.as_slice()).unwrap());
        assert!(!restored.library.offline_cache_enabled);
        assert_eq!(restored.library.offline_cache_max_bytes, 2_000_000_000);

        let older = config_from_proto(&Config::default());
        assert!(older.library.offline_cache_enabled, "on unless turned off");
        assert_eq!(
            older.library.offline_cache_max_bytes,
            config::DEFAULT_OFFLINE_CACHE_MAX_BYTES
        );
    }

    /// Every setting is on the wire now, the daemon-owned ones included, so every sample edit survives the round trip.
    #[test]
    fn every_wire_setting_survives_the_round_trip() {
        let mut original = config::Settings::default();
        original.integrations.radio_registries.clear();
        for edit in config::edit_samples() {
            original.apply(edit);
        }
        let restored = config_from_proto(&config_to_proto(&original));
        assert_eq!(restored, original);
    }
}
