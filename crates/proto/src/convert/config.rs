//! `config::AppConfig` <-> the wire `Config` message.
//!
//! The settings surface is a real message, not JSON in a string field: the
//! config is a closed Rust struct on both ends of one binary, so the schema
//! can say so. Credential keys and machine-local path state are absent from
//! the wire entirely -- `config_from_proto` leaves them at their defaults
//! and the daemon restores what it holds.

use crate::*;

pub fn album_view_mode_to_proto(value: config::AlbumViewMode) -> AlbumViewMode {
    match value {
        config::AlbumViewMode::Grid => AlbumViewMode::Grid,
        config::AlbumViewMode::List => AlbumViewMode::List,
    }
}

pub fn album_view_mode_from_proto(value: i32) -> config::AlbumViewMode {
    match AlbumViewMode::try_from(value).unwrap_or(AlbumViewMode::Unspecified) {
        AlbumViewMode::Grid => config::AlbumViewMode::Grid,
        AlbumViewMode::List => config::AlbumViewMode::List,
        AlbumViewMode::Unspecified => config::AlbumViewMode::default(),
    }
}

pub fn artist_view_order_to_proto(value: &config::ArtistViewOrder) -> ArtistViewOrder {
    match value {
        config::ArtistViewOrder::Tracks => ArtistViewOrder::Tracks,
        config::ArtistViewOrder::Albums => ArtistViewOrder::Albums,
    }
}

pub fn artist_view_order_from_proto(value: i32) -> config::ArtistViewOrder {
    match ArtistViewOrder::try_from(value).unwrap_or(ArtistViewOrder::Unspecified) {
        ArtistViewOrder::Tracks => config::ArtistViewOrder::Tracks,
        ArtistViewOrder::Albums => config::ArtistViewOrder::Albums,
        ArtistViewOrder::Unspecified => config::ArtistViewOrder::Tracks,
    }
}

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

pub fn listen_now_style_to_proto(value: config::ListenNowStyle) -> ListenNowStyle {
    match value {
        config::ListenNowStyle::List => ListenNowStyle::List,
        config::ListenNowStyle::Cards => ListenNowStyle::Cards,
    }
}

pub fn listen_now_style_from_proto(value: i32) -> config::ListenNowStyle {
    match ListenNowStyle::try_from(value).unwrap_or(ListenNowStyle::Unspecified) {
        ListenNowStyle::List => config::ListenNowStyle::List,
        ListenNowStyle::Cards => config::ListenNowStyle::Cards,
        ListenNowStyle::Unspecified => config::ListenNowStyle::default(),
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

pub fn sort_order_to_proto(value: &config::SortOrder) -> SortOrder {
    match value {
        config::SortOrder::Title => SortOrder::Title,
        config::SortOrder::Artist => SortOrder::Artist,
        config::SortOrder::Album => SortOrder::Album,
    }
}

pub fn sort_order_from_proto(value: i32) -> config::SortOrder {
    match SortOrder::try_from(value).unwrap_or(SortOrder::Unspecified) {
        SortOrder::Title => config::SortOrder::Title,
        SortOrder::Artist => config::SortOrder::Artist,
        SortOrder::Album => config::SortOrder::Album,
        SortOrder::Unspecified => config::SortOrder::Title,
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
    }
}

pub fn ui_style_from_proto(value: i32) -> config::UiStyle {
    match UiStyle::try_from(value).unwrap_or(UiStyle::Unspecified) {
        UiStyle::Normal => config::UiStyle::Normal,
        UiStyle::Vaxry => config::UiStyle::Vaxry,
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

pub fn sort_direction_to_proto(value: config::SortDirection) -> SortDirection {
    match value {
        config::SortDirection::Asc => SortDirection::Asc,
        config::SortDirection::Desc => SortDirection::Desc,
    }
}

pub fn sort_direction_from_proto(value: i32) -> config::SortDirection {
    match SortDirection::try_from(value).unwrap_or(SortDirection::Unspecified) {
        SortDirection::Asc => config::SortDirection::Asc,
        SortDirection::Desc => config::SortDirection::Desc,
        SortDirection::Unspecified => config::SortDirection::default(),
    }
}

pub fn album_sort_field_to_proto(value: config::AlbumSortField) -> AlbumSortField {
    match value {
        config::AlbumSortField::Title => AlbumSortField::Title,
        config::AlbumSortField::Artist => AlbumSortField::Artist,
        config::AlbumSortField::Year => AlbumSortField::Year,
        config::AlbumSortField::Genre => AlbumSortField::Genre,
    }
}

pub fn album_sort_field_from_proto(value: i32) -> config::AlbumSortField {
    match AlbumSortField::try_from(value).unwrap_or(AlbumSortField::Unspecified) {
        AlbumSortField::Title => config::AlbumSortField::Title,
        AlbumSortField::Artist => config::AlbumSortField::Artist,
        AlbumSortField::Year => config::AlbumSortField::Year,
        AlbumSortField::Genre => config::AlbumSortField::Genre,
        AlbumSortField::Unspecified => config::AlbumSortField::Title,
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

pub fn artist_sort_field_to_proto(value: config::ArtistSortField) -> ArtistSortField {
    match value {
        config::ArtistSortField::Name => ArtistSortField::Name,
        config::ArtistSortField::Tracks => ArtistSortField::Tracks,
        config::ArtistSortField::Albums => ArtistSortField::Albums,
    }
}

pub fn artist_sort_field_from_proto(value: i32) -> config::ArtistSortField {
    match ArtistSortField::try_from(value).unwrap_or(ArtistSortField::Unspecified) {
        ArtistSortField::Name => config::ArtistSortField::Name,
        ArtistSortField::Tracks => config::ArtistSortField::Tracks,
        ArtistSortField::Albums => config::ArtistSortField::Albums,
        ArtistSortField::Unspecified => config::ArtistSortField::Name,
    }
}

pub fn source_ref_to_proto(value: &config::Source) -> SourceRef {
    let kind = match value {
        config::Source::Local => source_ref::Kind::Local(Unit {}),
        config::Source::LocalLibrary(id) => source_ref::Kind::LocalLibrary(id.clone()),
        config::Source::Server(id) => source_ref::Kind::Server(id.clone()),
    };
    SourceRef { kind: Some(kind) }
}

pub fn source_ref_from_proto(value: Option<&SourceRef>) -> config::Source {
    match value.and_then(|value| value.kind.as_ref()) {
        Some(source_ref::Kind::LocalLibrary(id)) => config::Source::LocalLibrary(id.clone()),
        Some(source_ref::Kind::Server(id)) => config::Source::Server(id.clone()),
        Some(source_ref::Kind::Local(_)) | None => config::Source::Local,
    }
}

pub fn saved_local_source_to_proto(value: &config::SavedLocalSource) -> SavedLocalSource {
    SavedLocalSource {
        id: value.id.clone(),
        name: value.name.clone(),
        directories: value
            .directories
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    }
}

pub fn saved_local_source_from_proto(value: &SavedLocalSource) -> config::SavedLocalSource {
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

pub fn home_section_to_proto(value: &config::HomeSection) -> HomeSection {
    HomeSection {
        key: value.key.clone(),
        enabled: value.enabled,
    }
}

pub fn home_section_from_proto(value: &HomeSection) -> config::HomeSection {
    config::HomeSection {
        key: value.key.clone(),
        enabled: value.enabled,
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

pub fn ytdlp_history_entry_to_proto(value: &config::YtdlpHistoryEntry) -> YtdlpHistoryEntry {
    YtdlpHistoryEntry {
        url: value.url.clone(),
        title: value.title.clone(),
        format: value.format.clone(),
        status: value.status.clone(),
        error: value.error.clone(),
    }
}

pub fn ytdlp_history_entry_from_proto(value: &YtdlpHistoryEntry) -> config::YtdlpHistoryEntry {
    config::YtdlpHistoryEntry {
        url: value.url.clone(),
        title: value.title.clone(),
        format: value.format.clone(),
        status: value.status.clone(),
        error: value.error.clone(),
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

pub fn album_sort_criterion_to_proto(
    value: &config::SortCriterion<config::AlbumSortField>,
) -> AlbumSortCriterion {
    AlbumSortCriterion {
        field: album_sort_field_to_proto(value.field) as i32,
        direction: sort_direction_to_proto(value.direction) as i32,
    }
}

pub fn album_sort_criterion_from_proto(
    value: &AlbumSortCriterion,
) -> config::SortCriterion<config::AlbumSortField> {
    config::SortCriterion {
        field: album_sort_field_from_proto(value.field),
        direction: sort_direction_from_proto(value.direction),
    }
}

pub fn track_sort_criterion_to_proto(
    value: &config::SortCriterion<config::TrackSortField>,
) -> TrackSortCriterion {
    TrackSortCriterion {
        field: track_sort_field_to_proto(value.field) as i32,
        direction: sort_direction_to_proto(value.direction) as i32,
    }
}

pub fn track_sort_criterion_from_proto(
    value: &TrackSortCriterion,
) -> config::SortCriterion<config::TrackSortField> {
    config::SortCriterion {
        field: track_sort_field_from_proto(value.field),
        direction: sort_direction_from_proto(value.direction),
    }
}

pub fn artist_sort_criterion_to_proto(
    value: &config::SortCriterion<config::ArtistSortField>,
) -> ArtistSortCriterion {
    ArtistSortCriterion {
        field: artist_sort_field_to_proto(value.field) as i32,
        direction: sort_direction_to_proto(value.direction) as i32,
    }
}

pub fn artist_sort_criterion_from_proto(
    value: &ArtistSortCriterion,
) -> config::SortCriterion<config::ArtistSortField> {
    config::SortCriterion {
        field: artist_sort_field_from_proto(value.field),
        direction: sort_direction_from_proto(value.direction),
    }
}

pub fn ytdlp_options_to_proto(value: &config::YtdlpOptions) -> YtdlpOptions {
    YtdlpOptions {
        embed_metadata: value.embed_metadata,
        embed_thumbnail: value.embed_thumbnail,
        postprocess_thumbnail_square: value.postprocess_thumbnail_square,
        embed_chapters: value.embed_chapters,
        embed_subs: value.embed_subs,
        embed_info_json: value.embed_info_json,
        write_thumbnail: value.write_thumbnail,
        write_description: value.write_description,
        write_info_json: value.write_info_json,
        write_subs: value.write_subs,
        write_auto_subs: value.write_auto_subs,
        write_comments: value.write_comments,
        sponsorblock: value.sponsorblock,
        sponsorblock_mark: value.sponsorblock_mark,
        split_chapters: value.split_chapters,
        convert_thumbnail: value.convert_thumbnail.clone(),
        no_playlist: value.no_playlist,
        xattrs: value.xattrs,
        no_mtime: value.no_mtime,
        rate_limit: value.rate_limit.clone(),
        cookies_from_browser: value.cookies_from_browser.clone(),
        js_runtimes: value.js_runtimes.clone(),
        audio_quality: u32::from(value.audio_quality),
    }
}

pub fn ytdlp_options_from_proto(value: Option<&YtdlpOptions>) -> config::YtdlpOptions {
    let Some(value) = value else {
        return config::YtdlpOptions::default();
    };
    config::YtdlpOptions {
        embed_metadata: value.embed_metadata,
        embed_thumbnail: value.embed_thumbnail,
        postprocess_thumbnail_square: value.postprocess_thumbnail_square,
        embed_chapters: value.embed_chapters,
        embed_subs: value.embed_subs,
        embed_info_json: value.embed_info_json,
        write_thumbnail: value.write_thumbnail,
        write_description: value.write_description,
        write_info_json: value.write_info_json,
        write_subs: value.write_subs,
        write_auto_subs: value.write_auto_subs,
        write_comments: value.write_comments,
        sponsorblock: value.sponsorblock,
        sponsorblock_mark: value.sponsorblock_mark,
        split_chapters: value.split_chapters,
        convert_thumbnail: value.convert_thumbnail.clone(),
        no_playlist: value.no_playlist,
        xattrs: value.xattrs,
        no_mtime: value.no_mtime,
        rate_limit: value.rate_limit.clone(),
        cookies_from_browser: value.cookies_from_browser.clone(),
        js_runtimes: value.js_runtimes.clone(),
        audio_quality: value.audio_quality.min(u32::from(u8::MAX)) as u8,
    }
}

pub fn config_to_proto(value: &config::AppConfig) -> Config {
    Config {
        local_sources: value
            .local_sources
            .iter()
            .map(saved_local_source_to_proto)
            .collect(),
        active_source: Some(source_ref_to_proto(&value.active_source)),
        source_explicitly_set: value.source_explicitly_set,
        server_folders: value
            .server_folders
            .iter()
            .map(|(k, v)| (k.clone(), StringList { values: v.clone() }))
            .collect(),
        spotify_browser: value.spotify_browser.clone(),
        spotify_prefer_active_device: value.spotify_prefer_active_device,
        music_directory: value
            .music_directory
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        theme: value.theme.clone(),
        live_theme_path: value.live_theme_path.clone(),
        device_id: value.device_id.clone(),
        discord_presence: value.discord_presence,
        discord_presence_paused: value.discord_presence_paused,
        discord_presence_source: value.discord_presence_source,
        sort_order: sort_order_to_proto(&value.sort_order) as i32,
        album_sort: value
            .album_sort
            .iter()
            .map(album_sort_criterion_to_proto)
            .collect(),
        library_sort: value
            .library_sort
            .iter()
            .map(track_sort_criterion_to_proto)
            .collect(),
        artist_album_sort: value
            .artist_album_sort
            .iter()
            .map(album_sort_criterion_to_proto)
            .collect(),
        artist_sort: value
            .artist_sort
            .iter()
            .map(artist_sort_criterion_to_proto)
            .collect(),
        album_view_mode: album_view_mode_to_proto(value.album_view_mode) as i32,
        artist_album_view_mode: album_view_mode_to_proto(value.artist_album_view_mode) as i32,
        artists_view_mode: album_view_mode_to_proto(value.artists_view_mode) as i32,
        artist_view_order: artist_view_order_to_proto(&value.artist_view_order) as i32,
        listen_counts: value.listen_counts.clone().into_iter().collect(),
        language: value.language.clone(),
        reduce_animations: value.reduce_animations,
        fullscreen_use_player_bar: value.fullscreen_use_player_bar,
        fullscreen_tabs_collapsed: value.fullscreen_tabs_collapsed,
        cover_art_background: value.cover_art_background,
        cover_art_darkening: u32::from(value.cover_art_darkening),
        cover_art_blur: u32::from(value.cover_art_blur),
        custom_background_path: value.custom_background_path.clone(),
        custom_font_path: value.custom_font_path.clone(),
        tracing_enabled: value.tracing_enabled,
        auto_check_updates: value.auto_check_updates,
        minimize_to_tray: value.minimize_to_tray,
        show_source_toggle: value.show_source_toggle,
        show_row_images: value.show_row_images,
        sidebar_order: value.sidebar_order.clone(),
        volume: value.volume,
        volume_scroll_step: value.volume_scroll_step,
        crossfade_seconds: u32::from(value.crossfade_seconds),
        custom_themes: value
            .custom_themes
            .iter()
            .map(|(k, v)| (k.clone(), custom_theme_to_proto(v)))
            .collect(),
        back_behavior: back_behavior_to_proto(value.back_behavior) as i32,
        channel_mode: channel_mode_to_proto(value.channel_mode) as i32,
        equalizer: Some(equalizer_to_proto(&value.equalizer)),
        device_change_behavior: device_change_behavior_to_proto(value.device_change_behavior)
            as i32,
        sample_rate_mode: sample_rate_mode_to_proto(value.sample_rate_mode) as i32,
        ytdlp_output_dir: value.ytdlp_output_dir.clone(),
        ytdlp_options: Some(ytdlp_options_to_proto(&value.ytdlp_options)),
        ytdlp_history: value
            .ytdlp_history
            .iter()
            .map(ytdlp_history_entry_to_proto)
            .collect(),
        titlebar_mode: titlebar_mode_to_proto(value.titlebar_mode) as i32,
        offline_quality: offline_quality_to_proto(value.offline_quality) as i32,
        player_bar_position: player_bar_position_to_proto(value.player_bar_position) as i32,
        ui_style: ui_style_to_proto(value.ui_style) as i32,
        settings_layout: settings_layout_to_proto(value.settings_layout) as i32,
        hero_height: value.hero_height,
        home_sections: value
            .home_sections
            .iter()
            .map(home_section_to_proto)
            .collect(),
        listen_now_style: listen_now_style_to_proto(value.listen_now_style) as i32,
        auto_fetch_covers: value.auto_fetch_covers,
        cover_fetch_strategy: fetch_strategy_to_proto(value.cover_fetch_strategy) as i32,
        radio_registries: value
            .radio_registries
            .iter()
            .map(registry_entry_to_proto)
            .collect(),
        pinned_stations: value.pinned_stations.clone(),
        prefer_local_lyrics: value.prefer_local_lyrics,
        enable_musixmatch_lyrics: value.enable_musixmatch_lyrics,
        lyrics_offset_ms: value.lyrics_offset_ms,
        lyrics_offset_auto: value.lyrics_offset_auto,
        lyrics_depth_blur: value.lyrics_depth_blur,
        lyrics_depth_blur_strength: u32::from(value.lyrics_depth_blur_strength),
    }
}

/// Credential fields and machine-local path state are not on the wire, so
/// they come back as their defaults here; the daemon overwrites them with
/// what it already holds before anything is persisted.
pub fn config_from_proto(value: &Config) -> config::AppConfig {
    config::AppConfig {
        local_sources: value
            .local_sources
            .iter()
            .map(saved_local_source_from_proto)
            .collect(),
        active_source: source_ref_from_proto(value.active_source.as_ref()),
        source_explicitly_set: value.source_explicitly_set,
        server_folders: value
            .server_folders
            .iter()
            .map(|(k, v)| (k.clone(), v.values.clone()))
            .collect(),
        spotify_browser: value.spotify_browser.clone(),
        spotify_prefer_active_device: value.spotify_prefer_active_device,
        music_directory: value
            .music_directory
            .iter()
            .map(std::path::PathBuf::from)
            .collect(),
        theme: value.theme.clone(),
        live_theme_path: value.live_theme_path.clone(),
        device_id: value.device_id.clone(),
        discord_presence: value.discord_presence,
        discord_presence_paused: value.discord_presence_paused,
        discord_presence_source: value.discord_presence_source,
        sort_order: sort_order_from_proto(value.sort_order),
        album_sort: value
            .album_sort
            .iter()
            .map(album_sort_criterion_from_proto)
            .collect(),
        library_sort: value
            .library_sort
            .iter()
            .map(track_sort_criterion_from_proto)
            .collect(),
        artist_album_sort: value
            .artist_album_sort
            .iter()
            .map(album_sort_criterion_from_proto)
            .collect(),
        artist_sort: value
            .artist_sort
            .iter()
            .map(artist_sort_criterion_from_proto)
            .collect(),
        album_view_mode: album_view_mode_from_proto(value.album_view_mode),
        artist_album_view_mode: album_view_mode_from_proto(value.artist_album_view_mode),
        artists_view_mode: album_view_mode_from_proto(value.artists_view_mode),
        artist_view_order: artist_view_order_from_proto(value.artist_view_order),
        listen_counts: value.listen_counts.clone().into_iter().collect(),
        language: value.language.clone(),
        reduce_animations: value.reduce_animations,
        fullscreen_use_player_bar: value.fullscreen_use_player_bar,
        fullscreen_tabs_collapsed: value.fullscreen_tabs_collapsed,
        cover_art_background: value.cover_art_background,
        cover_art_darkening: value.cover_art_darkening.min(u32::from(u8::MAX)) as u8,
        cover_art_blur: value.cover_art_blur.min(u32::from(u8::MAX)) as u8,
        custom_background_path: value.custom_background_path.clone(),
        custom_font_path: value.custom_font_path.clone(),
        tracing_enabled: value.tracing_enabled,
        auto_check_updates: value.auto_check_updates,
        minimize_to_tray: value.minimize_to_tray,
        show_source_toggle: value.show_source_toggle,
        show_row_images: value.show_row_images,
        sidebar_order: value.sidebar_order.clone(),
        volume: value.volume,
        volume_scroll_step: value.volume_scroll_step,
        crossfade_seconds: value.crossfade_seconds.min(u32::from(u8::MAX)) as u8,
        custom_themes: value
            .custom_themes
            .iter()
            .map(|(k, v)| (k.clone(), custom_theme_from_proto(v)))
            .collect(),
        back_behavior: back_behavior_from_proto(value.back_behavior),
        channel_mode: channel_mode_from_proto(value.channel_mode),
        equalizer: equalizer_from_proto(value.equalizer.as_ref()),
        device_change_behavior: device_change_behavior_from_proto(value.device_change_behavior),
        sample_rate_mode: sample_rate_mode_from_proto(value.sample_rate_mode),
        ytdlp_output_dir: value.ytdlp_output_dir.clone(),
        ytdlp_options: ytdlp_options_from_proto(value.ytdlp_options.as_ref()),
        ytdlp_history: value
            .ytdlp_history
            .iter()
            .map(ytdlp_history_entry_from_proto)
            .collect(),
        titlebar_mode: titlebar_mode_from_proto(value.titlebar_mode),
        offline_quality: offline_quality_from_proto(value.offline_quality),
        player_bar_position: player_bar_position_from_proto(value.player_bar_position),
        ui_style: ui_style_from_proto(value.ui_style),
        settings_layout: settings_layout_from_proto(value.settings_layout),
        hero_height: value.hero_height,
        home_sections: value
            .home_sections
            .iter()
            .map(home_section_from_proto)
            .collect(),
        listen_now_style: listen_now_style_from_proto(value.listen_now_style),
        auto_fetch_covers: value.auto_fetch_covers,
        cover_fetch_strategy: fetch_strategy_from_proto(value.cover_fetch_strategy),
        radio_registries: value
            .radio_registries
            .iter()
            .map(registry_entry_from_proto)
            .collect(),
        pinned_stations: value.pinned_stations.clone(),
        prefer_local_lyrics: value.prefer_local_lyrics,
        enable_musixmatch_lyrics: value.enable_musixmatch_lyrics,
        lyrics_offset_ms: value.lyrics_offset_ms,
        lyrics_offset_auto: value.lyrics_offset_auto,
        lyrics_depth_blur: value.lyrics_depth_blur,
        lyrics_depth_blur_strength: value.lyrics_depth_blur_strength.min(u32::from(u8::MAX)) as u8,
        ..Default::default()
    }
}
