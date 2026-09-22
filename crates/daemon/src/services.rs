//! What each service is, and what configuring one takes.
//!
//! Everything a client would otherwise have to know about a backend lives
//! here: its name, its mark, the form that adds one, the options it has once
//! it exists, and what signing it in means. A frontend renders the field lists
//! and sends the answers back, so adding a service is a spec in this file and
//! no change at all on the other side of the API.

use api::schema::{ChoiceOption, FieldKind, FieldSpec, FieldValue, Icon, Problem, Text, value_of};
use api::{ServerDraft, ServiceInfo, ServiceRef, SignInKind};
use config::{AppConfig, Browser, MusicService, SavedServer};

/// The field keys a form and its answers agree on.
pub const URL: &str = "url";
pub const AUTH_METHOD: &str = "auth_method";
pub const BROWSER: &str = "browser";
pub const STOREFRONT: &str = "storefront";
pub const LANGUAGE: &str = "language";
pub const TOKEN: &str = "token";

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
    MusicService::ALL
        .iter()
        .map(|service| ServiceInfo {
            id: service.id().to_string(),
            name: name(*service),
            icon: icon(*service),
            accent: accent(*service).to_string(),
            experimental: matches!(service, MusicService::Spotify),
            fields: add_fields(*service),
        })
        .collect()
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
    match service {
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
                            },
                            ChoiceOption {
                                value: ANONYMOUS.to_string(),
                                label: Text::key("sign_in_anonymously"),
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
                            },
                            ChoiceOption {
                                value: MANUAL.to_string(),
                                label: Text::key("apple_music_paste_token"),
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
        // Spotify signs in through the system browser with the desktop
        // client's own id, so there is nothing to fill in but the name.
        MusicService::Spotify => vec![FieldSpec {
            help: Some(Text::key("spotify_help")),
            ..note_field()
        }],
        MusicService::Nextcloud => vec![
            url_field("nextcloud_url_placeholder"),
            FieldSpec {
                help: Some(Text::key("nextcloud_help")),
                ..note_field()
            },
        ],
        MusicService::Jellyfin | MusicService::Subsonic | MusicService::Custom => {
            vec![url_field("server_url_placeholder")]
        }
    }
}

/// The options a configured source has, with the values it currently holds.
pub fn settings(server: &ServerView<'_>, _config: &AppConfig) -> Vec<FieldSpec> {
    let browser = server.browser.map(|browser| browser.id().to_string());
    match server.service {
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
        _ => Vec::new(),
    }
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
    if anonymous {
        SignInKind::None
    } else if service.uses_browser_signin() {
        SignInKind::Browser
    } else {
        SignInKind::Password
    }
}

fn anonymous_draft(draft: &ServerDraft) -> bool {
    value_of(&draft.values, AUTH_METHOD) == Some(ANONYMOUS)
}

/// What saving this draft would need, and what is wrong with it.
pub fn check(service: MusicService, draft: &ServerDraft) -> (SignInKind, Vec<Problem>) {
    let mut problems = Vec::new();
    if draft.name.trim().is_empty() {
        problems.push(Problem::on("name", Text::key("server_name_required")));
    }
    let value = |key: &str| value_of(&draft.values, key).unwrap_or_default().trim();
    match service {
        MusicService::AppleMusic => {
            if value(STOREFRONT).is_empty() {
                problems.push(Problem::on(
                    STOREFRONT,
                    Text::key("apple_music_storefront_required"),
                ));
            }
            if value_of(&draft.values, AUTH_METHOD) == Some(MANUAL)
                && value_of(&draft.secrets, TOKEN)
                    .map(str::trim)
                    .unwrap_or_default()
                    .is_empty()
            {
                problems.push(Problem::on(TOKEN, Text::key("apple_music_token_required")));
            }
        }
        MusicService::YtMusic | MusicService::SoundCloud | MusicService::Spotify => {}
        _ => {
            if !value(URL).starts_with("http") {
                problems.push(Problem::on(URL, Text::key("invalid_server_url")));
            }
        }
    }
    (draft_sign_in(service, anonymous_draft(draft)), problems)
}

/// Fold a draft's answers into the row that is stored.
pub fn apply(service: MusicService, draft: &ServerDraft, saved: &mut SavedServer) {
    let value = |key: &str| {
        value_of(&draft.values, key)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    saved.service = service;
    saved.name = draft.name.trim().to_string();
    saved.url = match service {
        // Spotify has no address: the session finds its own access point.
        MusicService::Spotify => String::new(),
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
/// None do today; the seam stays so a service option can live there again.
pub fn config_keys(_server: &SavedServer, _values: &[FieldValue]) -> Vec<&'static str> {
    Vec::new()
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
pub fn apply_config_settings(
    _service: MusicService,
    _values: &[FieldValue],
    _config: &mut AppConfig,
) {
}

/// What a refusal says when a draft is saved without being checked first. A
/// client renders the [`Problem`] itself; this is for the log.
pub fn problem_text(problem: &Problem) -> String {
    match &problem.label {
        Text::Key(key) => key.clone(),
        Text::Literal(text) => text.clone(),
    }
}
