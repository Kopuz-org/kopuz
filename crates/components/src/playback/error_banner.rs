use dioxus::prelude::*;
use hooks::{PlaybackError, PlayerController};

/// What the banner offers for a failure. The first is the fix the code
/// points at; copying the details is always there, for a bug report.
#[derive(Clone, Copy, PartialEq)]
enum Fix {
    SignIn,
    Retry,
    Skip,
}

fn advice(code: Option<api::ErrorCode>, source: &str) -> Option<(String, Fix)> {
    let key_fix = match code? {
        api::ErrorCode::SourceAuthExpired => ("playback_error_session_expired", Fix::SignIn),
        api::ErrorCode::SourceChallenged => ("playback_error_challenged", Fix::Retry),
        api::ErrorCode::SourceRefused => ("playback_error_refused", Fix::Retry),
        api::ErrorCode::SourceUnreachable => ("playback_error_unreachable", Fix::Retry),
        api::ErrorCode::ItemUnavailable => ("playback_error_unavailable", Fix::Skip),
        _ => return None,
    };
    Some((
        i18n::t_with(key_fix.0, &[("source", source.into())]),
        key_fix.1,
    ))
}

/// The bar under the title bar that says why a track did not play, and what
/// to do about it.
#[component]
pub fn PlaybackErrorBanner(error: PlaybackError, on_sign_in: EventHandler<()>) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let sources = hooks::sources::use_sources();
    let source = sources
        .read()
        .iter()
        .flatten()
        .find(|source| source.active)
        .map(|source| crate::forms::text(&source.service.name))
        .unwrap_or_default();
    let advice = advice(error.code, &source);
    let shown = advice
        .as_ref()
        .map(|(text, _)| text.clone())
        .unwrap_or_else(|| error.message.clone());
    let details = format!(
        "{}\nCode: {:?}\n{}",
        utils::build_info::summary(),
        error.code,
        error.message
    );

    rsx! {
        div { class: "flex-shrink-0",
            div {
                role: "alert",
                class: "flex flex-wrap items-center justify-between gap-x-3 gap-y-1 px-4 py-2 bg-rose-500/15 border-b border-rose-500/20 text-rose-200 text-sm",
                div { class: "flex min-w-0 flex-1 items-center gap-2 whitespace-pre-line",
                    i { class: "fa-solid fa-triangle-exclamation text-xs", aria_hidden: "true" }
                    span { "{shown}" }
                }
                div { class: "flex shrink-0 items-center gap-1",
                    match advice.map(|(_, fix)| fix) {
                        Some(Fix::SignIn) => rsx! {
                            BannerButton {
                                label: i18n::t("playback_sign_in_again"),
                                onclick: move |_| {
                                    ctrl.playback_error.set(None);
                                    on_sign_in.call(());
                                },
                            }
                        },
                        Some(Fix::Retry) => rsx! {
                            BannerButton {
                                label: i18n::t("playback_try_again"),
                                onclick: move |_| {
                                    ctrl.playback_error.set(None);
                                    ctrl.resume();
                                },
                            }
                        },
                        Some(Fix::Skip) => rsx! {
                            BannerButton {
                                label: i18n::t("playback_skip"),
                                onclick: move |_| {
                                    ctrl.playback_error.set(None);
                                    ctrl.play_next();
                                },
                            }
                        },
                        None => rsx! {},
                    }
                    BannerButton {
                        label: i18n::t("playback_copy_details"),
                        onclick: move |_| {
                            crate::common::shared::copy_to_clipboard(&details);
                            hooks::toast::toast(&i18n::t("playback_details_copied"));
                        },
                    }
                    button {
                        class: "opacity-50 hover:opacity-100 focus-visible:opacity-100 transition-opacity p-1.5",
                        aria_label: i18n::t("close"),
                        onclick: move |_| ctrl.playback_error.set(None),
                        i { class: "fa-solid fa-xmark text-xs", aria_hidden: "true" }
                    }
                }
            }
        }
    }
}

#[component]
fn BannerButton(label: String, onclick: EventHandler<MouseEvent>) -> Element {
    rsx! {
        button {
            class: "rounded px-2 py-1 text-xs font-medium text-rose-100 bg-rose-500/20 hover:bg-rose-500/30 active:scale-[0.96] transition-[background-color,scale]",
            onclick: move |event| onclick.call(event),
            "{label}"
        }
    }
}
