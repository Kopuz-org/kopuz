//! What each service is, and what configuring one takes.
//!
//! Everything a client would otherwise have to know about a backend lives
//! here: its name, its mark, the form that adds one, the options it has once
//! it exists, and what signing it in means. A frontend renders the field lists
//! and sends the answers back, so adding a service is a spec in this file and
//! no change at all on the other side of the API.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use api::schema::{ChoiceOption, FieldKind, FieldSpec, FieldValue, Icon, Problem, Text, value_of};
use api::{
    ErrorReason, FolderProbe, ServerProbe, ServiceInfo, ServiceRef, SignInKind, SourceDraft,
};
use config::{Browser, MusicService, SavedServer, Settings};

/// The field keys a form and its answers agree on.
pub const URL: &str = "url";
pub const CLIENT_ID: &str = "client_id";
pub const AUTH_METHOD: &str = "auth_method";
pub const BROWSER: &str = "browser";
pub const STOREFRONT: &str = "storefront";
pub const LANGUAGE: &str = "language";
pub const TOKEN: &str = "token";
pub const PLAYBACK_BROWSER: &str = "playback_browser";
pub const PREFER_ACTIVE_DEVICE: &str = "prefer_active_device";
pub const DIRECTORIES: &str = "directories";

/// The service id of the folders the daemon scans itself.
pub const FOLDERS: &str = "folders";

/// `auth_method` values.
const BY_BROWSER: &str = "browser";
const ANONYMOUS: &str = "anonymous";
const MANUAL: &str = "manual";

const AUTOMATIC: &str = "auto";

const APPLE_STOREFRONTS: &[&str] = &[
    "us", "gb", "jp", "de", "fr", "au", "br", "mx", "kr", "nl", "it", "es", "ca", "ua", "tr",
];

const APPLE_LANGUAGES: &[&str] = &[
    "en", "ja", "de", "fr", "es", "pt", "it", "nl", "ko", "zh-Hans", "zh-Hant",
];

/// The Nextcloud mark, from the Simple Icons set (CC0); the vendored Font
/// Awesome 6 Free carries no glyph for it.
const NEXTCLOUD_MARK: &str = "M12.018 6.537c-2.5 0-4.6 1.712-5.241 4.015-.56-1.232-1.793-2.105-3.225-2.105A3.569 3.569 0 0 0 0 12a3.569 3.569 0 0 0 3.552 3.553c1.432 0 2.664-.874 3.224-2.106.641 2.304 2.742 4.016 5.242 4.016 2.487 0 4.576-1.693 5.231-3.977.569 1.21 1.783 2.067 3.198 2.067A3.568 3.568 0 0 0 24 12a3.569 3.569 0 0 0-3.553-3.553c-1.416 0-2.63.858-3.199 2.067-.654-2.284-2.743-3.978-5.23-3.977zm0 2.085c1.878 0 3.378 1.5 3.378 3.378 0 1.878-1.5 3.378-3.378 3.378A3.362 3.362 0 0 1 8.641 12c0-1.878 1.5-3.378 3.377-3.378zm-8.466 1.91c.822 0 1.467.645 1.467 1.468s-.644 1.467-1.467 1.468A1.452 1.452 0 0 1 2.085 12c0-.823.644-1.467 1.467-1.467zm16.895 0c.823 0 1.468.645 1.468 1.468s-.645 1.468-1.468 1.468A1.452 1.452 0 0 1 18.98 12c0-.823.644-1.467 1.467-1.467z";

/// The parts of a configured server the registry reads, borrowed from either
/// the stored row or the hydrated active one.
pub struct ServerView<'a> {
    pub service: MusicService,
    pub url: &'a str,
    pub browser: Option<Browser>,
    pub anonymous: bool,
    pub storefront: &'a str,
    pub language: &'a str,
}

impl<'a> From<&'a SavedServer> for ServerView<'a> {
    fn from(server: &'a SavedServer) -> Self {
        Self {
            service: server.service,
            url: &server.url,
            browser: server.yt_browser,
            anonymous: server.yt_anonymous,
            storefront: &server.apple_music_storefront,
            language: &server.apple_music_language,
        }
    }
}

impl<'a> From<&'a config::MusicServer> for ServerView<'a> {
    fn from(server: &'a config::MusicServer) -> Self {
        Self {
            service: server.service,
            url: &server.url,
            browser: server.yt_browser,
            anonymous: server.yt_anonymous,
            storefront: &server.apple_music_storefront,
            language: &server.apple_music_language,
        }
    }
}

pub fn all() -> Vec<ServiceInfo> {
    let mut services: Vec<ServiceInfo> = MusicService::ALL
        .iter()
        .filter(|service| **service != MusicService::AudioCd)
        .map(|service| ServiceInfo {
            id: service.id().to_string(),
            name: name(*service),
            summary: Text::key(summary(*service)),
            icon: icon(*service),
            accent: accent(*service).to_string(),
            experimental: matches!(service, MusicService::Spotify),
            fields: add_fields(*service),
        })
        .collect();
    services.push(folders());
    services
}

/// The folders a daemon scans for music, offered like any other service.
pub fn folders() -> ServiceInfo {
    ServiceInfo {
        id: FOLDERS.to_string(),
        name: Text::key("remote_music_folders"),
        summary: Text::key("service_summary_folders"),
        icon: Icon::Class("fa-solid fa-folder-tree".into()),
        accent: "#6366f1".to_string(),
        experimental: false,
        fields: vec![folder_source_field(&[])],
    }
}

pub fn folders_ref() -> ServiceRef {
    let service = folders();
    ServiceRef {
        id: service.id,
        name: service.name,
        icon: service.icon,
        accent: service.accent,
    }
}

/// The folder list of a source, with the paths it holds now.
pub fn directories_field(paths: &[String]) -> FieldSpec {
    FieldSpec {
        required: true,
        value: Some(api::schema::encode_directories(paths)),
        ..text_field(DIRECTORIES, "remote_music_folders", FieldKind::Directories)
    }
}

/// The folders of a folder source, which live in the settings file, so a layer that pins them renders the field locked.
pub fn folder_source_field(paths: &[String]) -> FieldSpec {
    FieldSpec {
        config_key: Some(
            config::SettingKey::Library(config::LibraryKey::LocalSources)
                .name()
                .to_string(),
        ),
        ..directories_field(paths)
    }
}

/// What is wrong with a folder source's answers.
pub fn check_folders(draft: &SourceDraft) -> Vec<FormProblem> {
    let mut problems = Vec::new();
    if draft.name.trim().is_empty() {
        problems.push(FormProblem::on("name", ErrorReason::SourceNameRequired));
    }
    let paths = folder_paths(&draft.values);
    if paths.iter().any(|path| path.trim().is_empty()) {
        problems.push(FormProblem::on(DIRECTORIES, ErrorReason::EmptySourceFolder));
    } else if paths.is_empty() {
        problems.push(FormProblem::on(
            DIRECTORIES,
            ErrorReason::SourceFolderRequired,
        ));
    }
    problems
}

/// The folders a draft or a settings answer names, none if it does not.
pub fn folder_paths(values: &[FieldValue]) -> Vec<String> {
    value_of(values, DIRECTORIES)
        .map(api::schema::decode_directories)
        .unwrap_or_default()
}

pub fn service_ref(service: MusicService) -> ServiceRef {
    ServiceRef {
        id: service.id().to_string(),
        name: name(service),
        icon: icon(service),
        accent: accent(service).to_string(),
    }
}

fn name(service: MusicService) -> Text {
    match service {
        MusicService::Jellyfin => Text::key("jellyfin"),
        MusicService::Subsonic => Text::key("subsonic"),
        MusicService::Custom => Text::key("custom_manual"),
        other => Text::literal(other.display_name()),
    }
}

/// The i18n key of the line a service picker shows under its name.
fn summary(service: MusicService) -> &'static str {
    match service {
        MusicService::Jellyfin => "service_summary_jellyfin",
        MusicService::Subsonic => "service_summary_subsonic",
        MusicService::Custom => "service_summary_custom",
        MusicService::YtMusic => "service_summary_ytmusic",
        MusicService::SoundCloud => "service_summary_soundcloud",
        MusicService::AppleMusic => "service_summary_applemusic",
        MusicService::Spotify => "service_summary_spotify",
        MusicService::Nextcloud => "service_summary_nextcloud",
        MusicService::Clippsly => "service_summary_clippsly",
        MusicService::Smb => "service_summary_smb",
        MusicService::AudioCd => "removable_temporary",
    }
}

fn icon(service: MusicService) -> Icon {
    match service {
        MusicService::YtMusic => Icon::Class("fa-brands fa-youtube".into()),
        MusicService::SoundCloud => Icon::Class("fa-brands fa-soundcloud".into()),
        MusicService::AppleMusic => Icon::Class("fa-brands fa-apple".into()),
        MusicService::Spotify => Icon::Class("fa-brands fa-spotify".into()),
        MusicService::Jellyfin => Icon::Class("fa-solid fa-server".into()),
        MusicService::Subsonic | MusicService::Custom => {
            Icon::Class("fa-solid fa-compact-disc".into())
        }
        MusicService::Nextcloud => Icon::Svg(NEXTCLOUD_MARK.into()),
        MusicService::Clippsly => Icon::Class("fa-solid fa-cloud".into()),
        MusicService::AudioCd => Icon::Class("fa-solid fa-compact-disc".into()),
        MusicService::Smb => Icon::Class("fa-solid fa-server".into()),
    }
}

fn accent(service: MusicService) -> &'static str {
    match service {
        MusicService::YtMusic => "#ff3355",
        MusicService::SoundCloud => "#ff7a33",
        MusicService::AppleMusic => "#ffffff",
        MusicService::Spotify => "#1DB954",
        MusicService::Jellyfin => "#b277ee",
        MusicService::Subsonic | MusicService::Custom => "#f0a84b",
        MusicService::Nextcloud => "#0082c9",
        MusicService::Clippsly => "#8064e9",
        MusicService::Smb => "#6366f1",
        MusicService::AudioCd => "#a3a3a3",
    }
}

fn text_field(key: &str, label: &str, kind: FieldKind) -> FieldSpec {
    FieldSpec {
        key: key.to_string(),
        label: Text::key(label),
        kind,
        ..Default::default()
    }
}

fn options_for(browsers: &[Browser]) -> Vec<ChoiceOption> {
    browsers
        .iter()
        .map(|browser| ChoiceOption {
            value: browser.id().to_string(),
            label: Text::literal(browser.label()),
            ..Default::default()
        })
        .collect()
}

/// Which browser opens the sign-in window. The system default leads and is
/// what an unanswered field means, so a Firefox user signs in through Firefox
/// without having to find this option first.
fn browser_field(value: Option<&str>, show_when: Option<FieldValue>) -> FieldSpec {
    let mut options = vec![ChoiceOption {
        value: AUTOMATIC.to_string(),
        label: Text::key("sign_in_browser_auto"),
        ..Default::default()
    }];
    options.extend(options_for(Browser::ALL));
    FieldSpec {
        key: BROWSER.to_string(),
        label: Text::key("sign_in_browser"),
        kind: FieldKind::Choice {
            options,
            custom: false,
        },
        value: Some(value.unwrap_or(AUTOMATIC).to_string()),
        show_when,
        ..Default::default()
    }
}

fn code_options(codes: &[&str]) -> Vec<ChoiceOption> {
    codes
        .iter()
        .map(|code| ChoiceOption {
            value: (*code).to_string(),
            label: Text::literal(*code),
            ..Default::default()
        })
        .collect()
}

/// A row that is only an explanation. It carries no answer, so its key is
/// never read back and its label is the help text below it.
fn note_field() -> FieldSpec {
    FieldSpec {
        key: "note".to_string(),
        kind: FieldKind::Note,
        ..Default::default()
    }
}

fn url_field(placeholder: &str) -> FieldSpec {
    FieldSpec {
        key: URL.to_string(),
        label: Text::key("server_url"),
        placeholder: Some(Text::key(placeholder)),
        kind: FieldKind::Url,
        required: true,
        ..Default::default()
    }
}

/// The form that adds one of these.
pub fn add_fields(service: MusicService) -> Vec<FieldSpec> {
    let fields = match service {
        MusicService::YtMusic => vec![
            FieldSpec {
                required: true,
                value: Some(BY_BROWSER.to_string()),
                ..text_field(
                    AUTH_METHOD,
                    "sign_in_method",
                    FieldKind::Radio {
                        options: vec![
                            ChoiceOption {
                                value: BY_BROWSER.to_string(),
                                label: Text::key("sign_in_with_browser"),
                                ..Default::default()
                            },
                            ChoiceOption {
                                value: ANONYMOUS.to_string(),
                                label: Text::key("sign_in_anonymously"),
                                ..Default::default()
                            },
                        ],
                    },
                )
            },
            FieldSpec {
                help: Some(Text::key("yt_anonymous_help")),
                show_when: Some(FieldValue::new(AUTH_METHOD, ANONYMOUS)),
                ..note_field()
            },
            browser_field(None, Some(FieldValue::new(AUTH_METHOD, BY_BROWSER))),
        ],
        MusicService::SoundCloud => vec![
            FieldSpec {
                help: Some(Text::key("soundcloud_sign_in_help")),
                ..note_field()
            },
            browser_field(None, None),
        ],
        MusicService::AppleMusic => vec![
            FieldSpec {
                required: true,
                value: Some("us".to_string()),
                ..text_field(
                    STOREFRONT,
                    "apple_music_storefront",
                    FieldKind::Choice {
                        options: code_options(APPLE_STOREFRONTS),
                        custom: true,
                    },
                )
            },
            FieldSpec {
                value: Some("en".to_string()),
                ..text_field(
                    LANGUAGE,
                    "apple_music_language",
                    FieldKind::Choice {
                        options: code_options(APPLE_LANGUAGES),
                        custom: true,
                    },
                )
            },
            FieldSpec {
                required: true,
                value: Some(BY_BROWSER.to_string()),
                ..text_field(
                    AUTH_METHOD,
                    "sign_in_method",
                    FieldKind::Radio {
                        options: vec![
                            ChoiceOption {
                                value: BY_BROWSER.to_string(),
                                label: Text::key("sign_in_with_browser"),
                                ..Default::default()
                            },
                            ChoiceOption {
                                value: MANUAL.to_string(),
                                label: Text::key("apple_music_paste_token"),
                                ..Default::default()
                            },
                        ],
                    },
                )
            },
            FieldSpec {
                placeholder: Some(Text::literal("media-user-token")),
                show_when: Some(FieldValue::new(AUTH_METHOD, MANUAL)),
                ..text_field(TOKEN, "apple_music_token", FieldKind::Secret)
            },
            browser_field(None, Some(FieldValue::new(AUTH_METHOD, BY_BROWSER))),
        ],
        MusicService::Spotify => vec![
            FieldSpec {
                required: true,
                placeholder: Some(Text::literal("Spotify Client ID")),
                help: Some(Text::key("spotify_client_id_help")),
                ..text_field(CLIENT_ID, "spotify_client_id", FieldKind::Text)
            },
            browser_field(None, None),
        ],
        MusicService::Clippsly => vec![FieldSpec {
            help: Some(Text::key("clippsly_help")),
            ..note_field()
        }],
        MusicService::Nextcloud => vec![
            url_field("nextcloud_url_placeholder"),
            FieldSpec {
                help: Some(Text::key("nextcloud_help")),
                ..note_field()
            },
        ],
        MusicService::AudioCd => Vec::new(),
        MusicService::Smb => vec![
            url_field("smb_url_placeholder"),
            FieldSpec {
                help: Some(Text::key("smb_help")),
                ..note_field()
            },
        ],
        MusicService::Jellyfin => vec![url_field("server_url_placeholder")],
        MusicService::Subsonic | MusicService::Custom => {
            vec![url_field("subsonic_url_placeholder")]
        }
    };
    #[cfg(target_os = "android")]
    let fields = webview_fields(fields);
    fields
}

#[cfg(any(target_os = "android", test))]
fn webview_fields(mut fields: Vec<FieldSpec>) -> Vec<FieldSpec> {
    fields.retain(|field| field.key != BROWSER && field.key != PLAYBACK_BROWSER);
    for field in &mut fields {
        if field.key == AUTH_METHOD
            && let FieldKind::Radio { options } = &mut field.kind
        {
            for option in options {
                if option.value == BY_BROWSER {
                    option.label = Text::key("sign_in_with_webview");
                }
            }
        }
        if field.help == Some(Text::key("soundcloud_sign_in_help")) {
            field.help = Some(Text::key("webview_sign_in_help"));
        }
    }
    fields
}

/// The options a configured source has, with the values it currently holds.
pub fn settings(server: &ServerView<'_>, config: &Settings) -> Vec<FieldSpec> {
    let browser = server.browser.map(|browser| browser.id().to_string());
    let fields = match server.service {
        MusicService::YtMusic | MusicService::SoundCloud if !server.anonymous => {
            vec![browser_field(browser.as_deref(), None)]
        }
        MusicService::AppleMusic => vec![
            FieldSpec {
                value: Some(server.storefront.to_string()),
                ..text_field(
                    STOREFRONT,
                    "apple_music_storefront",
                    FieldKind::Choice {
                        options: code_options(APPLE_STOREFRONTS),
                        custom: true,
                    },
                )
            },
            FieldSpec {
                value: Some(server.language.to_string()),
                ..text_field(
                    LANGUAGE,
                    "apple_music_language",
                    FieldKind::Choice {
                        options: code_options(APPLE_LANGUAGES),
                        custom: true,
                    },
                )
            },
            browser_field(browser.as_deref(), None),
        ],
        MusicService::Spotify => {
            let mut hosts = vec![ChoiceOption {
                value: AUTOMATIC.to_string(),
                label: Text::key("playback_browser_auto"),
                ..Default::default()
            }];
            hosts.extend(options_for(Browser::CHROMIUM_FAMILY));
            vec![
                FieldSpec {
                    value: Some(
                        config
                            .spotify
                            .spotify_browser
                            .clone()
                            .unwrap_or_else(|| AUTOMATIC.to_string()),
                    ),
                    config_key: Some("spotify_browser".to_string()),
                    ..text_field(
                        PLAYBACK_BROWSER,
                        "playback_browser",
                        FieldKind::Choice {
                            options: hosts,
                            custom: false,
                        },
                    )
                },
                FieldSpec {
                    value: Some(config.spotify.spotify_prefer_active_device.to_string()),
                    config_key: Some("spotify_prefer_active_device".to_string()),
                    help: Some(Text::key("prefer_active_device_help")),
                    ..text_field(
                        PREFER_ACTIVE_DEVICE,
                        "prefer_active_device",
                        FieldKind::Toggle,
                    )
                },
            ]
        }
        _ => Vec::new(),
    };
    #[cfg(target_os = "android")]
    let fields = webview_fields(fields);
    fields
}

/// The line under a source's name.
pub fn detail(server: &ServerView<'_>) -> Option<String> {
    match server.service {
        MusicService::Spotify | MusicService::YtMusic | MusicService::SoundCloud => None,
        MusicService::AppleMusic => Some(server.storefront.to_uppercase()),
        _ => Some(server.url.to_string()),
    }
}

/// What making this source usable would take, given what it already has.
pub fn sign_in(server: &ServerView<'_>, authenticated: bool) -> SignInKind {
    if authenticated {
        return SignInKind::None;
    }
    draft_sign_in(server.service, server.anonymous)
}

fn draft_sign_in(service: MusicService, anonymous: bool) -> SignInKind {
    if anonymous || service == MusicService::AudioCd {
        SignInKind::None
    } else if service.uses_browser_signin() {
        SignInKind::Browser
    } else {
        SignInKind::Password
    }
}

fn anonymous_draft(draft: &SourceDraft) -> bool {
    value_of(&draft.values, AUTH_METHOD) == Some(ANONYMOUS)
}

/// What saving this draft would need, and what is wrong with it.
pub fn check(service: MusicService, draft: &SourceDraft) -> (SignInKind, Vec<FormProblem>) {
    let mut problems = Vec::new();
    if draft.name.trim().is_empty() {
        problems.push(FormProblem::on("name", ErrorReason::SourceNameRequired));
    }
    let value = |key: &str| value_of(&draft.values, key).unwrap_or_default().trim();
    match service {
        MusicService::Spotify => {
            if value(CLIENT_ID).is_empty() {
                problems.push(FormProblem::on(
                    CLIENT_ID,
                    ErrorReason::SpotifyClientIdRequired,
                ));
            }
        }
        MusicService::AppleMusic => {
            if value(STOREFRONT).is_empty() {
                problems.push(FormProblem::on(
                    STOREFRONT,
                    ErrorReason::AppleMusicStorefrontRequired,
                ));
            }
            if value_of(&draft.values, AUTH_METHOD) == Some(MANUAL)
                && value_of(&draft.secrets, TOKEN)
                    .map(str::trim)
                    .unwrap_or_default()
                    .is_empty()
            {
                problems.push(FormProblem::on(TOKEN, ErrorReason::AppleMusicTokenRequired));
            }
        }
        MusicService::YtMusic | MusicService::SoundCloud | MusicService::Clippsly => {}
        MusicService::AudioCd => {}
        MusicService::Smb => {
            if server::smb::Location::parse(value(URL)).is_err() {
                problems.push(FormProblem::on(URL, ErrorReason::InvalidSmbUrl));
            }
        }
        _ => {
            if !value(URL).starts_with("http") {
                problems.push(FormProblem::on(URL, ErrorReason::InvalidServerUrl));
            }
        }
    }
    (draft_sign_in(service, anonymous_draft(draft)), problems)
}

/// Fold a draft's answers into the row that is stored.
pub fn apply(service: MusicService, draft: &SourceDraft, saved: &mut SavedServer) {
    let value = |key: &str| {
        value_of(&draft.values, key)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    saved.service = service;
    saved.name = draft.name.trim().to_string();
    saved.url = match service {
        MusicService::Spotify => value(CLIENT_ID),
        MusicService::Clippsly => server::clippsly::BASE_URL.to_string(),
        _ => value(URL).trim_end_matches('/').to_string(),
    };
    saved.yt_anonymous = anonymous_draft(draft);
    saved.yt_browser = if saved.yt_anonymous {
        None
    } else {
        value_of(&draft.values, BROWSER).and_then(Browser::from_id)
    };
    if service == MusicService::AppleMusic {
        let storefront = value(STOREFRONT);
        if !storefront.is_empty() {
            saved.apple_music_storefront = storefront;
        }
        let language = value(LANGUAGE);
        if !language.is_empty() {
            saved.apple_music_language = language;
        }
    }
}

/// Which settings keys belong to the config rather than the server row.
pub fn config_keys(server: &SavedServer, values: &[FieldValue]) -> Vec<config::SettingKey> {
    use config::{SettingKey, SpotifyKey};
    if server.service != MusicService::Spotify {
        return Vec::new();
    }
    let mut keys = Vec::new();
    if value_of(values, PLAYBACK_BROWSER).is_some() {
        keys.push(SettingKey::Spotify(SpotifyKey::SpotifyBrowser));
    }
    if value_of(values, PREFER_ACTIVE_DEVICE).is_some() {
        keys.push(SettingKey::Spotify(SpotifyKey::SpotifyPreferActiveDevice));
    }
    keys
}

/// Fold answered options into the stored row. An absent key is left alone.
pub fn apply_server_settings(values: &[FieldValue], saved: &mut SavedServer) {
    if let Some(browser) = value_of(values, BROWSER) {
        saved.yt_browser = Browser::from_id(browser);
    }
    if let Some(storefront) = value_of(values, STOREFRONT).map(str::trim)
        && !storefront.is_empty()
    {
        saved.apple_music_storefront = storefront.to_string();
    }
    if let Some(language) = value_of(values, LANGUAGE).map(str::trim)
        && !language.is_empty()
    {
        saved.apple_music_language = language.to_string();
    }
}

/// The same for the answers that live in the settings rather than on the row.
pub fn apply_config_settings(service: MusicService, values: &[FieldValue], config: &mut Settings) {
    if service != MusicService::Spotify {
        return;
    }
    if let Some(host) = value_of(values, PLAYBACK_BROWSER) {
        config.spotify.spotify_browser = (host != AUTOMATIC).then(|| host.to_string());
    }
    if let Some(prefer) = value_of(values, PREFER_ACTIVE_DEVICE) {
        config.spotify.spotify_prefer_active_device = prefer == "true";
    }
}

/// What a refusal says when a draft is saved without being checked first. A
/// client renders the [`Problem`] itself; this is for the log.
/// One thing wrong with a draft: a field a form marks and translates, and the reason a save that skipped the check is refused with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormProblem {
    pub field: &'static str,
    pub reason: ErrorReason,
}

impl FormProblem {
    pub fn on(field: &'static str, reason: ErrorReason) -> Self {
        Self { field, reason }
    }

    pub fn problem(&self) -> Problem {
        Problem::on(self.field, Text::key(self.reason.message_key()))
    }

    pub fn error(&self) -> api::ApiError {
        let message = match self.reason {
            ErrorReason::SourceNameRequired => "a source needs a name",
            ErrorReason::SourceFolderRequired => "a folder source needs at least one folder",
            ErrorReason::EmptySourceFolder => "a source directory cannot be empty",
            ErrorReason::SpotifyClientIdRequired => "a Spotify source needs a client id",
            ErrorReason::AppleMusicStorefrontRequired => "an Apple Music source needs a storefront",
            ErrorReason::AppleMusicTokenRequired => "an Apple Music token sign-in needs a token",
            ErrorReason::InvalidSmbUrl => "not an SMB address",
            ErrorReason::InvalidServerUrl => "the server address must be an http(s) URL",
            _ => "the source form is not complete",
        };
        api::ApiError::invalid_input(message).because(self.reason)
    }
}

pub fn problems(found: &[FormProblem]) -> Vec<Problem> {
    found.iter().map(FormProblem::problem).collect()
}

/// Audio files a folder probe counts before it reports "at least".
pub const PROBE_FILE_CAP: u32 = 10_000;
/// How long a folder probe walks one folder before it reports "at least".
pub const PROBE_WALK_BUDGET: Duration = Duration::from_secs(5);
/// How long a server probe waits for its one answer.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// What probing a draft tries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeTarget {
    Folders(Vec<String>),
    Server(String),
    Nothing,
}

/// The folders or the address a draft's probe tries; a server address that is not an http(s) URL is refused.
pub fn probe_target(draft: &SourceDraft) -> Result<ProbeTarget, api::ApiError> {
    if draft.service == FOLDERS {
        return Ok(ProbeTarget::Folders(folder_paths(&draft.values)));
    }
    let service = MusicService::from_id(&draft.service)
        .ok_or_else(|| api::ApiError::invalid_input("no such service"))?;
    match service {
        MusicService::Jellyfin
        | MusicService::Subsonic
        | MusicService::Custom
        | MusicService::Nextcloud => {
            let url = value_of(&draft.values, URL).unwrap_or_default().trim();
            match reqwest::Url::parse(url) {
                Ok(parsed) if matches!(parsed.scheme(), "http" | "https") && parsed.has_host() => {
                    Ok(ProbeTarget::Server(url.to_string()))
                }
                _ => Err(FormProblem::on(URL, ErrorReason::InvalidServerUrl).error()),
            }
        }
        MusicService::YtMusic
        | MusicService::SoundCloud
        | MusicService::AppleMusic
        | MusicService::Spotify
        | MusicService::Clippsly
        | MusicService::Smb
        | MusicService::AudioCd => Ok(ProbeTarget::Nothing),
    }
}

/// Walk one folder as the scanner does, counting audio files by its file-type rule, until `cap` files or `deadline`.
pub fn probe_folder(path: &str, cap: u32, deadline: Instant) -> FolderProbe {
    let root = Path::new(path);
    let mut probe = FolderProbe {
        path: path.to_string(),
        exists: root.exists(),
        ..FolderProbe::default()
    };
    let Ok(first) = std::fs::read_dir(root) else {
        return probe;
    };
    probe.readable = true;
    // A symlinked directory is followed like the scanner follows it, so each real one is walked once.
    let mut walked: HashSet<PathBuf> = root.canonicalize().into_iter().collect();
    let mut pending: Vec<PathBuf> = Vec::new();
    let mut entries = Some(first);
    while let Some(listing) = entries.take() {
        for entry in listing.flatten() {
            if probe.audio_files >= cap || Instant::now() >= deadline {
                probe.at_least = true;
                return probe;
            }
            let path = entry.path();
            match std::fs::metadata(&path) {
                Ok(meta) if meta.is_dir() => {
                    if path.canonicalize().is_ok_and(|real| walked.insert(real)) {
                        pending.push(path);
                    }
                }
                Ok(_) => {
                    if reader::scanner::is_audio_file(&path) {
                        probe.audio_files += 1;
                    }
                }
                Err(_) => {}
            }
        }
        while entries.is_none() {
            let Some(next) = pending.pop() else { break };
            entries = std::fs::read_dir(next).ok();
        }
    }
    probe
}

pub fn probe_client() -> Result<reqwest::Client, api::ApiError> {
    utils::http::builder()
        .timeout(PROBE_TIMEOUT)
        .build()
        .map_err(|error| {
            api::ApiError::internal(format!("an HTTP client failed to start: {error}"))
        })
}

/// One GET to a server draft's address: a success status is reachable, any other status is reported, no answer is unreachable.
pub async fn probe_server(client: &reqwest::Client, url: &str) -> ServerProbe {
    match client.get(url).send().await {
        Ok(response) if response.status().is_success() => ServerProbe::Reachable,
        Ok(response) => ServerProbe::Status(response.status().as_u16()),
        Err(error) => {
            tracing::debug!(error = %error.without_url(), "a server draft did not answer");
            ServerProbe::Unreachable
        }
    }
}

#[cfg(test)]
mod summary_tests {
    use super::*;

    #[test]
    fn every_service_says_what_it_is_in_the_baseline_locale() {
        let baseline = include_str!("../../../apps/common/i18n/locales/en.ftl");
        let services = all();
        assert_eq!(
            services.len(),
            MusicService::ALL.len(),
            "folders are configurable; audio CDs are detected"
        );
        for service in services {
            let Text::Key(key) = &service.summary else {
                panic!("{} has no summary key", service.id);
            };
            assert!(
                baseline
                    .lines()
                    .any(|line| line.starts_with(&format!("{key} = "))),
                "{key} is missing from en.ftl"
            );
        }
    }
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn later() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    fn draft(service: &str, values: Vec<FieldValue>) -> SourceDraft {
        SourceDraft {
            name: "Draft".into(),
            service: service.into(),
            values,
            ..SourceDraft::default()
        }
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, b"").expect("write");
    }

    #[test]
    fn a_folder_counts_its_audio_by_the_scanner_rule_and_nothing_else() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("a.mp3"));
        touch(&dir.path().join("deep/er/b.FLAC"));
        touch(&dir.path().join("deep/cover.jpg"));
        touch(&dir.path().join("notes.txt"));

        let probe = probe_folder(&dir.path().to_string_lossy(), PROBE_FILE_CAP, later());

        assert!(probe.exists && probe.readable);
        assert_eq!(probe.audio_files, 2);
        assert!(!probe.at_least);
    }

    #[test]
    fn a_missing_folder_neither_exists_nor_reads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let gone = dir.path().join("gone").to_string_lossy().into_owned();

        let probe = probe_folder(&gone, PROBE_FILE_CAP, later());

        assert_eq!(
            probe,
            FolderProbe {
                path: gone,
                ..FolderProbe::default()
            }
        );
    }

    #[test]
    fn counting_stops_at_the_cap_and_says_at_least() {
        let dir = tempfile::tempdir().expect("tempdir");
        for n in 0..5 {
            touch(&dir.path().join(format!("{n}.opus")));
        }

        let probe = probe_folder(&dir.path().to_string_lossy(), 3, later());

        assert_eq!(probe.audio_files, 3);
        assert!(probe.at_least);
    }

    #[test]
    fn a_walk_past_its_deadline_says_at_least() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("a.mp3"));

        let probe = probe_folder(
            &dir.path().to_string_lossy(),
            PROBE_FILE_CAP,
            Instant::now(),
        );

        assert!(probe.readable && probe.at_least);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_loop_is_walked_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("inner/a.wav"));
        std::os::unix::fs::symlink(dir.path(), dir.path().join("inner/loop")).expect("symlink");

        let probe = probe_folder(&dir.path().to_string_lossy(), PROBE_FILE_CAP, later());

        assert_eq!(probe.audio_files, 1);
        assert!(!probe.at_least);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_listed_exists_but_is_unreadable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let locked = dir.path().join("locked");
        touch(&locked.join("a.mp3"));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        // Root lists anything, so there is nothing to see as root.
        let listable = std::fs::read_dir(&locked).is_ok();

        let probe = probe_folder(&locked.to_string_lossy(), PROBE_FILE_CAP, later());

        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        if !listable {
            assert!(probe.exists && !probe.readable);
            assert_eq!(probe.audio_files, 0);
        }
    }

    #[test]
    fn folders_and_servers_are_probed_and_the_rest_are_not() {
        let folders = vec![FieldValue::new(
            DIRECTORIES,
            api::schema::encode_directories(&["/a".to_string(), "/b".to_string()]),
        )];
        assert_eq!(
            probe_target(&draft(FOLDERS, folders)),
            Ok(ProbeTarget::Folders(vec!["/a".into(), "/b".into()]))
        );
        for service in ["jellyfin", "subsonic", "custom", "nextcloud"] {
            let url = vec![FieldValue::new(URL, " https://music.example:8096 ")];
            assert_eq!(
                probe_target(&draft(service, url)),
                Ok(ProbeTarget::Server("https://music.example:8096".into())),
                "{service}"
            );
        }
        for &service in MusicService::ALL {
            if !matches!(
                service,
                MusicService::Jellyfin
                    | MusicService::Subsonic
                    | MusicService::Custom
                    | MusicService::Nextcloud
            ) {
                let url = vec![FieldValue::new(URL, "smb://nas.local/music")];
                assert_eq!(
                    probe_target(&draft(service.id(), url)),
                    Ok(ProbeTarget::Nothing),
                    "{service:?}"
                );
            }
        }
    }

    #[test]
    fn a_server_address_that_is_not_http_is_refused_by_reason() {
        for url in ["", "jelly.local", "ftp://jelly.local", "http://"] {
            let error = probe_target(&draft("jellyfin", vec![FieldValue::new(URL, url)]))
                .expect_err("not an address to probe");
            assert_eq!(error.reason, Some(ErrorReason::InvalidServerUrl), "{url:?}");
        }
        let unknown = probe_target(&draft("gopher", Vec::new())).expect_err("no such service");
        assert_eq!(unknown.code, api::ErrorCode::InvalidInput);
    }

    /// A server that answers every request with `status` and an empty body.
    async fn answering(status: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !request.windows(4).any(|end| end == b"\r\n\r\n") {
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => request.extend_from_slice(&chunk[..read]),
                        }
                    }
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        url
    }

    #[tokio::test]
    async fn a_server_that_answers_is_reachable_or_reports_its_status() {
        let client = probe_client().expect("client");

        assert_eq!(
            probe_server(&client, &answering("200 OK").await).await,
            ServerProbe::Reachable
        );
        assert_eq!(
            probe_server(&client, &answering("401 Unauthorized").await).await,
            ServerProbe::Status(401)
        );
        assert_eq!(
            probe_server(&client, &answering("502 Bad Gateway").await).await,
            ServerProbe::Status(502)
        );
    }

    #[tokio::test]
    async fn a_closed_port_or_a_silent_server_is_unreachable() {
        let closed = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            format!("http://{}", listener.local_addr().expect("addr"))
        };
        let client = probe_client().expect("client");
        assert_eq!(
            probe_server(&client, &closed).await,
            ServerProbe::Unreachable
        );

        let silent = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let url = format!("http://{}", silent.local_addr().expect("addr"));
        let held = tokio::spawn(async move {
            let mut streams = Vec::new();
            while let Ok((stream, _)) = silent.accept().await {
                streams.push(stream);
            }
        });
        let impatient = utils::http::builder()
            .timeout(Duration::from_millis(200))
            .build()
            .expect("client");
        assert_eq!(
            probe_server(&impatient, &url).await,
            ServerProbe::Unreachable
        );
        held.abort();
    }
}

#[cfg(test)]
mod webview_tests {
    use super::*;

    #[test]
    fn audio_cd_is_discovered_instead_of_manually_configured() {
        assert!(!all().iter().any(|service| service.id == "cdda"));
        assert!(add_fields(MusicService::AudioCd).is_empty());
    }

    #[test]
    fn android_forms_have_no_external_browser_or_registered_app_setup() {
        for service in [
            MusicService::YtMusic,
            MusicService::SoundCloud,
            MusicService::AppleMusic,
            MusicService::Spotify,
        ] {
            let fields = webview_fields(add_fields(service));
            assert!(
                fields
                    .iter()
                    .all(|field| field.key != BROWSER && field.key != PLAYBACK_BROWSER)
            );
            if service != MusicService::Spotify {
                assert!(fields.iter().all(|field| field.key != CLIENT_ID
                    && field.key != "client_secret"
                    && field.key != "developer_token"));
            }
            for field in fields {
                if let FieldKind::Radio { options } = field.kind {
                    for option in options {
                        if option.value == BY_BROWSER {
                            assert_eq!(option.label, Text::key("sign_in_with_webview"));
                        }
                    }
                }
            }
        }
    }
}
