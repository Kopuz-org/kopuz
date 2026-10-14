//! Typed settings edits, their keys and their outcomes <-> the wire `ConfigEdit`, `SettingKey` and `EditReport`.

use super::config::*;
use super::service::{config_view_from_proto, config_view_to_proto};
use crate::*;
use config::{
    BackgroundEdit as Bg, BackgroundKey, CustomThemeOp as ThemeOp, DownloadsEdit as Dl,
    DownloadsKey, IntegrationsEdit as In, IntegrationsKey, LibraryEdit as Li, LibraryKey,
    LyricsEdit as Ly, LyricsKey, PlaybackEdit as Pb, PlaybackKey, RadioRegistryOp as RegistryOp,
    SettingsEdit as Se, SpotifyEdit as Sp, SpotifyKey, SystemEdit as Sy, SystemKey, UiEdit, UiKey,
};
use config_edit::Edit;

pub fn setting_key_to_proto(value: config::SettingKey) -> SettingKey {
    use config::SettingKey as K;
    match value {
        K::Playback(key) => match key {
            PlaybackKey::VolumeScrollStep => SettingKey::VolumeScrollStep,
            PlaybackKey::CrossfadeSeconds => SettingKey::CrossfadeSeconds,
            PlaybackKey::BackBehavior => SettingKey::BackBehavior,
            PlaybackKey::ChannelMode => SettingKey::ChannelMode,
            PlaybackKey::Equalizer => SettingKey::Equalizer,
            PlaybackKey::ReplayGain => SettingKey::ReplayGain,
            PlaybackKey::DeviceChangeBehavior => SettingKey::DeviceChangeBehavior,
            PlaybackKey::SampleRateMode => SettingKey::SampleRateMode,
            PlaybackKey::StreamQuality => SettingKey::StreamQuality,
            PlaybackKey::AutoplayRadio => SettingKey::AutoplayRadio,
            PlaybackKey::SkipExplicit => SettingKey::SkipExplicit,
            PlaybackKey::PauseWatchHistory => SettingKey::PauseWatchHistory,
        },
        K::Ui(key) => match key {
            UiKey::Theme => SettingKey::Theme,
            UiKey::LiveThemePath => SettingKey::LiveThemePath,
            UiKey::CustomThemes => SettingKey::CustomThemes,
            UiKey::Language => SettingKey::Language,
            UiKey::ReduceAnimations => SettingKey::ReduceAnimations,
            UiKey::FullscreenUsePlayerBar => SettingKey::FullscreenUsePlayerBar,
            UiKey::ShowSourceToggle => SettingKey::ShowSourceToggle,
            UiKey::ShowRowImages => SettingKey::ShowRowImages,
            UiKey::TitlebarMode => SettingKey::TitlebarMode,
            UiKey::PlayerBarPosition => SettingKey::PlayerBarPosition,
            UiKey::UiStyle => SettingKey::UiStyle,
            UiKey::SettingsLayout => SettingKey::SettingsLayout,
            UiKey::CustomFontPath => SettingKey::CustomFontPath,
        },
        K::Background(key) => match key {
            BackgroundKey::CoverArtBackground => SettingKey::CoverArtBackground,
            BackgroundKey::CoverArtDarkening => SettingKey::CoverArtDarkening,
            BackgroundKey::CoverArtBlur => SettingKey::CoverArtBlur,
            BackgroundKey::CustomBackgroundPath => SettingKey::CustomBackgroundPath,
        },
        K::Lyrics(key) => match key {
            LyricsKey::PreferLocalLyrics => SettingKey::PreferLocalLyrics,
            LyricsKey::EnableMusixmatchLyrics => SettingKey::EnableMusixmatchLyrics,
            LyricsKey::LyricsOffsetMs => SettingKey::LyricsOffsetMs,
            LyricsKey::LyricsOffsetAuto => SettingKey::LyricsOffsetAuto,
            LyricsKey::LyricsDepthBlur => SettingKey::LyricsDepthBlur,
            LyricsKey::LyricsDepthBlurStrength => SettingKey::LyricsDepthBlurStrength,
        },
        K::Library(key) => match key {
            LibraryKey::LocalSources => SettingKey::LocalSources,
            LibraryKey::AutoFetchCovers => SettingKey::AutoFetchCovers,
            LibraryKey::CoverFetchStrategy => SettingKey::CoverFetchStrategy,
            LibraryKey::OfflineQuality => SettingKey::OfflineQuality,
            LibraryKey::SyncIntervalMinutes => SettingKey::SyncIntervalMinutes,
        },
        K::Integrations(key) => match key {
            IntegrationsKey::DiscordPresence => SettingKey::DiscordPresence,
            IntegrationsKey::DiscordPresenceSource => SettingKey::DiscordPresenceSource,
            IntegrationsKey::RadioRegistries => SettingKey::RadioRegistries,
        },
        K::Downloads(key) => match key {
            DownloadsKey::DownloaderOutputDir => SettingKey::DownloaderOutputDir,
            DownloadsKey::DownloaderOptions => SettingKey::DownloaderOptions,
        },
        K::Spotify(key) => match key {
            SpotifyKey::SpotifyBrowser => SettingKey::SpotifyBrowser,
            SpotifyKey::SpotifyPreferActiveDevice => SettingKey::SpotifyPreferActiveDevice,
        },
        K::System(key) => match key {
            SystemKey::TracingEnabled => SettingKey::TracingEnabled,
            SystemKey::AutoCheckUpdates => SettingKey::AutoCheckUpdates,
            SystemKey::MinimizeToTray => SettingKey::MinimizeToTray,
        },
    }
}

/// `None` for a key this build does not know.
pub fn setting_key_from_proto(value: i32) -> Option<config::SettingKey> {
    use config::SettingKey as K;
    Some(match SettingKey::try_from(value).ok()? {
        SettingKey::Unspecified => return None,
        SettingKey::VolumeScrollStep => K::Playback(PlaybackKey::VolumeScrollStep),
        SettingKey::CrossfadeSeconds => K::Playback(PlaybackKey::CrossfadeSeconds),
        SettingKey::BackBehavior => K::Playback(PlaybackKey::BackBehavior),
        SettingKey::ChannelMode => K::Playback(PlaybackKey::ChannelMode),
        SettingKey::Equalizer => K::Playback(PlaybackKey::Equalizer),
        SettingKey::ReplayGain => K::Playback(PlaybackKey::ReplayGain),
        SettingKey::DeviceChangeBehavior => K::Playback(PlaybackKey::DeviceChangeBehavior),
        SettingKey::SampleRateMode => K::Playback(PlaybackKey::SampleRateMode),
        SettingKey::StreamQuality => K::Playback(PlaybackKey::StreamQuality),
        SettingKey::AutoplayRadio => K::Playback(PlaybackKey::AutoplayRadio),
        SettingKey::SkipExplicit => K::Playback(PlaybackKey::SkipExplicit),
        SettingKey::PauseWatchHistory => K::Playback(PlaybackKey::PauseWatchHistory),
        SettingKey::Theme => K::Ui(UiKey::Theme),
        SettingKey::LiveThemePath => K::Ui(UiKey::LiveThemePath),
        SettingKey::CustomThemes => K::Ui(UiKey::CustomThemes),
        SettingKey::Language => K::Ui(UiKey::Language),
        SettingKey::ReduceAnimations => K::Ui(UiKey::ReduceAnimations),
        SettingKey::FullscreenUsePlayerBar => K::Ui(UiKey::FullscreenUsePlayerBar),
        SettingKey::ShowSourceToggle => K::Ui(UiKey::ShowSourceToggle),
        SettingKey::ShowRowImages => K::Ui(UiKey::ShowRowImages),
        SettingKey::TitlebarMode => K::Ui(UiKey::TitlebarMode),
        SettingKey::PlayerBarPosition => K::Ui(UiKey::PlayerBarPosition),
        SettingKey::UiStyle => K::Ui(UiKey::UiStyle),
        SettingKey::SettingsLayout => K::Ui(UiKey::SettingsLayout),
        SettingKey::CustomFontPath => K::Ui(UiKey::CustomFontPath),
        SettingKey::CoverArtBackground => K::Background(BackgroundKey::CoverArtBackground),
        SettingKey::CoverArtDarkening => K::Background(BackgroundKey::CoverArtDarkening),
        SettingKey::CoverArtBlur => K::Background(BackgroundKey::CoverArtBlur),
        SettingKey::CustomBackgroundPath => K::Background(BackgroundKey::CustomBackgroundPath),
        SettingKey::PreferLocalLyrics => K::Lyrics(LyricsKey::PreferLocalLyrics),
        SettingKey::EnableMusixmatchLyrics => K::Lyrics(LyricsKey::EnableMusixmatchLyrics),
        SettingKey::LyricsOffsetMs => K::Lyrics(LyricsKey::LyricsOffsetMs),
        SettingKey::LyricsOffsetAuto => K::Lyrics(LyricsKey::LyricsOffsetAuto),
        SettingKey::LyricsDepthBlur => K::Lyrics(LyricsKey::LyricsDepthBlur),
        SettingKey::LyricsDepthBlurStrength => K::Lyrics(LyricsKey::LyricsDepthBlurStrength),
        SettingKey::LocalSources => K::Library(LibraryKey::LocalSources),
        SettingKey::AutoFetchCovers => K::Library(LibraryKey::AutoFetchCovers),
        SettingKey::CoverFetchStrategy => K::Library(LibraryKey::CoverFetchStrategy),
        SettingKey::OfflineQuality => K::Library(LibraryKey::OfflineQuality),
        SettingKey::SyncIntervalMinutes => K::Library(LibraryKey::SyncIntervalMinutes),
        SettingKey::DiscordPresence => K::Integrations(IntegrationsKey::DiscordPresence),
        SettingKey::DiscordPresenceSource => {
            K::Integrations(IntegrationsKey::DiscordPresenceSource)
        }
        SettingKey::RadioRegistries => K::Integrations(IntegrationsKey::RadioRegistries),
        SettingKey::DownloaderOutputDir => K::Downloads(DownloadsKey::DownloaderOutputDir),
        SettingKey::DownloaderOptions => K::Downloads(DownloadsKey::DownloaderOptions),
        SettingKey::SpotifyBrowser => K::Spotify(SpotifyKey::SpotifyBrowser),
        SettingKey::SpotifyPreferActiveDevice => K::Spotify(SpotifyKey::SpotifyPreferActiveDevice),
        SettingKey::TracingEnabled => K::System(SystemKey::TracingEnabled),
        SettingKey::AutoCheckUpdates => K::System(SystemKey::AutoCheckUpdates),
        SettingKey::MinimizeToTray => K::System(SystemKey::MinimizeToTray),
    })
}

pub fn config_edit_to_proto(value: &config::SettingsEdit) -> ConfigEdit {
    let edit = match value.clone() {
        Se::Playback(edit) => match edit {
            Pb::VolumeScrollStep(value) => Edit::VolumeScrollStep(value),
            Pb::CrossfadeSeconds(value) => Edit::CrossfadeSeconds(u32::from(value)),
            Pb::BackBehavior(value) => Edit::BackBehavior(back_behavior_to_proto(value) as i32),
            Pb::ChannelMode(value) => Edit::ChannelMode(channel_mode_to_proto(value) as i32),
            Pb::Equalizer(value) => Edit::Equalizer(equalizer_to_proto(&value)),
            Pb::ReplayGain(value) => Edit::ReplayGain(replay_gain_to_proto(value)),
            Pb::DeviceChangeBehavior(value) => {
                Edit::DeviceChangeBehavior(device_change_behavior_to_proto(value) as i32)
            }
            Pb::SampleRateMode(value) => {
                Edit::SampleRateMode(sample_rate_mode_to_proto(value) as i32)
            }
            Pb::StreamQuality(value) => Edit::StreamQuality(stream_quality_to_proto(value) as i32),
            Pb::AutoplayRadio(value) => Edit::AutoplayRadio(value),
            Pb::SkipExplicit(value) => Edit::SkipExplicit(value),
            Pb::PauseWatchHistory(value) => Edit::PauseWatchHistory(value),
        },
        Se::Ui(edit) => match edit {
            UiEdit::Theme(value) => Edit::Theme(value),
            UiEdit::LiveThemePath(value) => Edit::LiveThemePath(value),
            UiEdit::CustomThemes(op) => Edit::CustomThemes(CustomThemeOp {
                op: Some(match op {
                    ThemeOp::Put { name, theme } => custom_theme_op::Op::Put(PutCustomTheme {
                        name,
                        theme: Some(custom_theme_to_proto(&theme)),
                    }),
                    ThemeOp::Delete { name } => {
                        custom_theme_op::Op::Delete(DeleteCustomTheme { name })
                    }
                }),
            }),
            UiEdit::Language(value) => Edit::Language(value),
            UiEdit::ReduceAnimations(value) => Edit::ReduceAnimations(value),
            UiEdit::FullscreenUsePlayerBar(value) => Edit::FullscreenUsePlayerBar(value),
            UiEdit::ShowSourceToggle(value) => Edit::ShowSourceToggle(value),
            UiEdit::ShowRowImages(value) => Edit::ShowRowImages(value),
            UiEdit::TitlebarMode(value) => Edit::TitlebarMode(titlebar_mode_to_proto(value) as i32),
            UiEdit::PlayerBarPosition(value) => {
                Edit::PlayerBarPosition(player_bar_position_to_proto(value) as i32)
            }
            UiEdit::UiStyle(value) => Edit::UiStyle(ui_style_to_proto(value) as i32),
            UiEdit::SettingsLayout(value) => {
                Edit::SettingsLayout(settings_layout_to_proto(value) as i32)
            }
            UiEdit::CustomFontPath(value) => Edit::CustomFontPath(value),
        },
        Se::Background(edit) => match edit {
            Bg::CoverArtBackground(value) => Edit::CoverArtBackground(value),
            Bg::CoverArtDarkening(value) => Edit::CoverArtDarkening(u32::from(value)),
            Bg::CoverArtBlur(value) => Edit::CoverArtBlur(u32::from(value)),
            Bg::CustomBackgroundPath(value) => Edit::CustomBackgroundPath(value),
        },
        Se::Lyrics(edit) => match edit {
            Ly::PreferLocalLyrics(value) => Edit::PreferLocalLyrics(value),
            Ly::EnableMusixmatchLyrics(value) => Edit::EnableMusixmatchLyrics(value),
            Ly::LyricsOffsetMs(value) => Edit::LyricsOffsetMs(value),
            Ly::LyricsOffsetAuto(value) => Edit::LyricsOffsetAuto(value),
            Ly::LyricsDepthBlur(value) => Edit::LyricsDepthBlur(value),
            Ly::LyricsDepthBlurStrength(value) => Edit::LyricsDepthBlurStrength(u32::from(value)),
        },
        Se::Library(edit) => match edit {
            Li::LocalSources(value) => Edit::FolderSources(FolderSources {
                sources: value.iter().map(saved_folder_source_to_proto).collect(),
            }),
            Li::AutoFetchCovers(value) => Edit::AutoFetchCovers(value),
            Li::CoverFetchStrategy(value) => {
                Edit::CoverFetchStrategy(fetch_strategy_to_proto(value) as i32)
            }
            Li::OfflineQuality(value) => {
                Edit::OfflineQuality(offline_quality_to_proto(value) as i32)
            }
            Li::SyncIntervalMinutes(value) => Edit::SyncIntervalMinutes(value),
        },
        Se::Integrations(edit) => match edit {
            In::DiscordPresence(value) => Edit::DiscordPresence(OptionalBool { value }),
            In::DiscordPresenceSource(value) => Edit::DiscordPresenceSource(OptionalBool { value }),
            In::RadioRegistries(op) => Edit::RadioRegistries(RadioRegistryOp {
                op: Some(match op {
                    RegistryOp::Put(entry) => {
                        radio_registry_op::Op::Put(registry_entry_to_proto(&entry))
                    }
                    RegistryOp::Remove { url } => {
                        radio_registry_op::Op::Remove(RemoveRadioRegistry { url })
                    }
                }),
            }),
        },
        Se::Downloads(edit) => match edit {
            Dl::DownloaderOutputDir(value) => Edit::DownloaderOutputDir(value),
            Dl::DownloaderOptions(value) => {
                Edit::DownloaderOptions(downloader_options_to_proto(&value))
            }
        },
        Se::Spotify(edit) => match edit {
            Sp::SpotifyBrowser(value) => Edit::SpotifyBrowser(OptionalString { value }),
            Sp::SpotifyPreferActiveDevice(value) => Edit::SpotifyPreferActiveDevice(value),
        },
        Se::System(edit) => match edit {
            Sy::TracingEnabled(value) => Edit::TracingEnabled(value),
            Sy::AutoCheckUpdates(value) => Edit::AutoCheckUpdates(value),
            Sy::MinimizeToTray(value) => Edit::MinimizeToTray(value),
        },
    };
    ConfigEdit { edit: Some(edit) }
}

/// Byte-sized values saturate, so an out-of-range one is refused by validation rather than wrapped; `None` for an edit this build cannot read.
pub fn config_edit_from_proto(value: &ConfigEdit) -> Option<config::SettingsEdit> {
    let byte = |value: u32| value.min(u32::from(u8::MAX)) as u8;
    Some(match value.edit.clone()? {
        Edit::VolumeScrollStep(value) => Se::Playback(Pb::VolumeScrollStep(value)),
        Edit::CrossfadeSeconds(value) => Se::Playback(Pb::CrossfadeSeconds(byte(value))),
        Edit::BackBehavior(value) => {
            Se::Playback(Pb::BackBehavior(back_behavior_from_proto(value)))
        }
        Edit::ChannelMode(value) => Se::Playback(Pb::ChannelMode(channel_mode_from_proto(value))),
        Edit::Equalizer(value) => {
            if value.bands.len() != config::EqualizerSettings::default().bands.len() {
                return None;
            }
            Se::Playback(Pb::Equalizer(equalizer_from_proto(Some(&value))))
        }
        Edit::ReplayGain(value) => {
            Se::Playback(Pb::ReplayGain(replay_gain_from_proto(Some(&value))))
        }
        Edit::DeviceChangeBehavior(value) => Se::Playback(Pb::DeviceChangeBehavior(
            device_change_behavior_from_proto(value),
        )),
        Edit::SampleRateMode(value) => {
            Se::Playback(Pb::SampleRateMode(sample_rate_mode_from_proto(value)))
        }
        Edit::StreamQuality(value) => {
            Se::Playback(Pb::StreamQuality(stream_quality_from_proto(value)))
        }
        Edit::AutoplayRadio(value) => Se::Playback(Pb::AutoplayRadio(value)),
        Edit::SkipExplicit(value) => Se::Playback(Pb::SkipExplicit(value)),
        Edit::PauseWatchHistory(value) => Se::Playback(Pb::PauseWatchHistory(value)),
        Edit::Theme(value) => Se::Ui(UiEdit::Theme(value)),
        Edit::LiveThemePath(value) => Se::Ui(UiEdit::LiveThemePath(value)),
        Edit::CustomThemes(value) => Se::Ui(UiEdit::CustomThemes(match value.op? {
            custom_theme_op::Op::Put(put) => ThemeOp::Put {
                theme: custom_theme_from_proto(put.theme.as_ref()?),
                name: put.name,
            },
            custom_theme_op::Op::Delete(delete) => ThemeOp::Delete { name: delete.name },
        })),
        Edit::Language(value) => Se::Ui(UiEdit::Language(value)),
        Edit::ReduceAnimations(value) => Se::Ui(UiEdit::ReduceAnimations(value)),
        Edit::FullscreenUsePlayerBar(value) => Se::Ui(UiEdit::FullscreenUsePlayerBar(value)),
        Edit::ShowSourceToggle(value) => Se::Ui(UiEdit::ShowSourceToggle(value)),
        Edit::ShowRowImages(value) => Se::Ui(UiEdit::ShowRowImages(value)),
        Edit::TitlebarMode(value) => Se::Ui(UiEdit::TitlebarMode(titlebar_mode_from_proto(value))),
        Edit::PlayerBarPosition(value) => Se::Ui(UiEdit::PlayerBarPosition(
            player_bar_position_from_proto(value),
        )),
        Edit::UiStyle(value) => Se::Ui(UiEdit::UiStyle(ui_style_from_proto(value))),
        Edit::SettingsLayout(value) => {
            Se::Ui(UiEdit::SettingsLayout(settings_layout_from_proto(value)))
        }
        Edit::CustomFontPath(value) => Se::Ui(UiEdit::CustomFontPath(value)),
        Edit::CoverArtBackground(value) => Se::Background(Bg::CoverArtBackground(value)),
        Edit::CoverArtDarkening(value) => Se::Background(Bg::CoverArtDarkening(byte(value))),
        Edit::CoverArtBlur(value) => Se::Background(Bg::CoverArtBlur(byte(value))),
        Edit::CustomBackgroundPath(value) => Se::Background(Bg::CustomBackgroundPath(value)),
        Edit::PreferLocalLyrics(value) => Se::Lyrics(Ly::PreferLocalLyrics(value)),
        Edit::EnableMusixmatchLyrics(value) => Se::Lyrics(Ly::EnableMusixmatchLyrics(value)),
        Edit::LyricsOffsetMs(value) => Se::Lyrics(Ly::LyricsOffsetMs(value)),
        Edit::LyricsOffsetAuto(value) => Se::Lyrics(Ly::LyricsOffsetAuto(value)),
        Edit::LyricsDepthBlur(value) => Se::Lyrics(Ly::LyricsDepthBlur(value)),
        Edit::LyricsDepthBlurStrength(value) => {
            Se::Lyrics(Ly::LyricsDepthBlurStrength(byte(value)))
        }
        Edit::FolderSources(value) => Se::Library(Li::LocalSources(
            value
                .sources
                .iter()
                .map(saved_folder_source_from_proto)
                .collect(),
        )),
        Edit::AutoFetchCovers(value) => Se::Library(Li::AutoFetchCovers(value)),
        Edit::CoverFetchStrategy(value) => {
            Se::Library(Li::CoverFetchStrategy(fetch_strategy_from_proto(value)))
        }
        Edit::OfflineQuality(value) => {
            Se::Library(Li::OfflineQuality(offline_quality_from_proto(value)))
        }
        Edit::SyncIntervalMinutes(value) => Se::Library(Li::SyncIntervalMinutes(value)),
        Edit::DiscordPresence(value) => Se::Integrations(In::DiscordPresence(value.value)),
        Edit::DiscordPresenceSource(value) => {
            Se::Integrations(In::DiscordPresenceSource(value.value))
        }
        Edit::RadioRegistries(value) => Se::Integrations(In::RadioRegistries(match value.op? {
            radio_registry_op::Op::Put(entry) => RegistryOp::Put(registry_entry_from_proto(&entry)),
            radio_registry_op::Op::Remove(remove) => RegistryOp::Remove { url: remove.url },
        })),
        Edit::DownloaderOutputDir(value) => Se::Downloads(Dl::DownloaderOutputDir(value)),
        Edit::DownloaderOptions(value) => Se::Downloads(Dl::DownloaderOptions(
            downloader_options_from_proto(Some(&value)),
        )),
        Edit::SpotifyBrowser(value) => Se::Spotify(Sp::SpotifyBrowser(value.value)),
        Edit::SpotifyPreferActiveDevice(value) => Se::Spotify(Sp::SpotifyPreferActiveDevice(value)),
        Edit::TracingEnabled(value) => Se::System(Sy::TracingEnabled(value)),
        Edit::AutoCheckUpdates(value) => Se::System(Sy::AutoCheckUpdates(value)),
        Edit::MinimizeToTray(value) => Se::System(Sy::MinimizeToTray(value)),
    })
}

pub fn edit_outcome_to_proto(value: &api::EditOutcome) -> EditOutcome {
    let outcome = match value {
        api::EditOutcome::Applied => edit_outcome::Outcome::Applied(Unit {}),
        api::EditOutcome::Locked => edit_outcome::Outcome::Locked(Unit {}),
        api::EditOutcome::Invalid { reason } => edit_outcome::Outcome::Invalid(InvalidEdit {
            reason: reason.clone(),
        }),
    };
    EditOutcome {
        outcome: Some(outcome),
    }
}

/// An outcome this build cannot read did not apply as far as it can tell.
pub fn edit_outcome_from_proto(value: &EditOutcome) -> api::EditOutcome {
    match &value.outcome {
        Some(edit_outcome::Outcome::Applied(_)) => api::EditOutcome::Applied,
        Some(edit_outcome::Outcome::Locked(_)) => api::EditOutcome::Locked,
        Some(edit_outcome::Outcome::Invalid(invalid)) => api::EditOutcome::Invalid {
            reason: invalid.reason.clone(),
        },
        None => api::EditOutcome::Invalid {
            reason: "the daemon sent an outcome this build does not know".into(),
        },
    }
}

pub fn edit_report_to_proto(value: &api::EditReport) -> EditReport {
    EditReport {
        outcomes: value.outcomes.iter().map(edit_outcome_to_proto).collect(),
        view: Some(config_view_to_proto(&value.view)),
    }
}

pub fn edit_report_from_proto(value: &EditReport) -> api::EditReport {
    api::EditReport {
        outcomes: value.outcomes.iter().map(edit_outcome_from_proto).collect(),
        view: config_view_from_proto(value.view.as_ref().unwrap_or(&ConfigView::default())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn every_edit_survives_the_wire() {
        for edit in config::edit_samples() {
            let bytes = config_edit_to_proto(&edit).encode_to_vec();
            let decoded = ConfigEdit::decode(bytes.as_slice()).expect("decodes");
            assert_eq!(config_edit_from_proto(&decoded), Some(edit));
        }
    }

    /// The field number of the first field an encoded message carries.
    fn first_field_number(bytes: &[u8]) -> u64 {
        let mut tag = 0u64;
        for (shift, byte) in bytes.iter().enumerate() {
            tag |= u64::from(byte & 0x7f) << (7 * shift);
            if byte & 0x80 == 0 {
                break;
            }
        }
        tag >> 3
    }

    #[test]
    fn every_key_survives_the_wire_and_shares_its_edit_case_number() {
        for edit in config::edit_samples() {
            let key = config::SettingKey::of(&edit);
            let wire = setting_key_to_proto(key);
            assert_eq!(setting_key_from_proto(wire as i32), Some(key));
            let case = config_edit_to_proto(&edit).encode_to_vec();
            assert_eq!(first_field_number(&case), wire as u64, "{key:?}");
        }
        assert_eq!(setting_key_from_proto(0), None);
        assert_eq!(setting_key_from_proto(9999), None);
    }

    #[test]
    fn an_edit_with_no_case_or_a_short_curve_is_unreadable() {
        assert_eq!(config_edit_from_proto(&ConfigEdit::default()), None);
        for opless in [
            Edit::CustomThemes(CustomThemeOp::default()),
            Edit::RadioRegistries(RadioRegistryOp::default()),
        ] {
            let edit = ConfigEdit { edit: Some(opless) };
            assert_eq!(config_edit_from_proto(&edit), None);
        }
        let short = ConfigEdit {
            edit: Some(Edit::Equalizer(EqualizerSettings {
                bands: vec![0.0; 5],
                ..Default::default()
            })),
        };
        assert_eq!(config_edit_from_proto(&short), None);
    }

    #[test]
    fn an_oversized_byte_saturates_so_validation_refuses_it() {
        let wide = ConfigEdit {
            edit: Some(Edit::CrossfadeSeconds(300)),
        };
        assert_eq!(
            config_edit_from_proto(&wide),
            Some(Se::Playback(Pb::CrossfadeSeconds(u8::MAX)))
        );
    }

    #[test]
    fn every_outcome_and_the_report_survive_the_wire() {
        let report = api::EditReport {
            outcomes: vec![
                api::EditOutcome::Applied,
                api::EditOutcome::Locked,
                api::EditOutcome::Invalid {
                    reason: "crossfade_seconds: greater than 12".into(),
                },
            ],
            view: api::ConfigView {
                settings: config::Settings::default(),
                locks: vec![api::SettingLock {
                    key: config::SettingKey::Ui(UiKey::Theme),
                    by: api::LockSource::Env("KOPUZ_CONFIG_THEME".into()),
                }],
                revision: 11,
            },
        };
        let bytes = edit_report_to_proto(&report).encode_to_vec();
        let decoded = EditReport::decode(bytes.as_slice()).expect("decodes");
        assert_eq!(edit_report_from_proto(&decoded), report);
        assert!(matches!(
            edit_outcome_from_proto(&EditOutcome::default()),
            api::EditOutcome::Invalid { .. }
        ));
    }
}
