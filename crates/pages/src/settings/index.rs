use super::navigation::SettingsCategory;
use components::settings_items::SettingsGroup;
use config::AppConfig;
use dioxus::prelude::*;

#[component]
pub(super) fn SettingsIndex(
    config: Signal<AppConfig>,
    active_source: Option<api::SourceInfo>,
    on_select: EventHandler<SettingsCategory>,
) -> Element {
    let language = config.read().language.clone();
    let language_name = i18n::available_languages()
        .iter()
        .find(|(code, _)| *code == language)
        .map(|(_, name)| (*name).to_string());
    let crossfade_seconds = config.read().crossfade_seconds;
    let crossfade = format!(
        "{} {}",
        i18n::t("crossfade"),
        if crossfade_seconds == 0 {
            i18n::t("crossfade_off")
        } else {
            format!("{crossfade_seconds}s")
        }
    );
    let equalizer = if config.read().equalizer.enabled {
        i18n::t("enabled")
    } else {
        i18n::t("disabled")
    };
    let quality = config.read().offline_quality.label().to_string();

    rsx! {
        div { id: "settings-index", class: "settings-index",
            if let Some(source) = active_source {
                {
                    let service = components::forms::text(&source.service.name);
                    let signed_in = source.needs_network && source.authenticated;
                    rsx! {
                        section { class: "settings-section rounded-xl overflow-hidden",
                            SettingsGroup { label: i18n::t("source") }
                            IndexRow {
                                icon: "fa-server",
                                title: source.name.clone(),
                                summary: if signed_in {
                                    format!("{service} · {}", i18n::t("signed_in"))
                                } else {
                                    service
                                },
                                online: signed_in,
                                on_click: move |_| on_select.call(SettingsCategory::Library),
                            }
                        }
                    }
                }
            }
            section { class: "settings-section rounded-xl overflow-hidden",
                SettingsGroup { label: i18n::t("settings_group_app") }
                IndexRow {
                    icon: "fa-sliders",
                    title: SettingsCategory::General.title(),
                    on_click: move |_| on_select.call(SettingsCategory::General),
                }
                IndexRow {
                    icon: "fa-paintbrush",
                    title: SettingsCategory::Customization.title(),
                    summary: language_name.unwrap_or_default(),
                    on_click: move |_| on_select.call(SettingsCategory::Customization),
                }
                IndexRow {
                    icon: "fa-music",
                    title: SettingsCategory::Library.title(),
                    on_click: move |_| on_select.call(SettingsCategory::Library),
                }
            }
            section { class: "settings-section rounded-xl overflow-hidden",
                SettingsGroup { label: i18n::t("settings_group_playback") }
                IndexRow {
                    icon: "fa-wave-square",
                    title: SettingsCategory::Player.title(),
                    summary: crossfade,
                    on_click: move |_| on_select.call(SettingsCategory::Player),
                }
                IndexRow {
                    icon: "fa-chart-simple",
                    title: SettingsCategory::Equalizer.title(),
                    summary: equalizer,
                    on_click: move |_| on_select.call(SettingsCategory::Equalizer),
                }
                IndexRow {
                    icon: "fa-cloud-arrow-down",
                    title: SettingsCategory::Downloads.title(),
                    summary: quality,
                    on_click: move |_| on_select.call(SettingsCategory::Downloads),
                }
            }
            section { class: "settings-section rounded-xl overflow-hidden",
                SettingsGroup { label: i18n::t("settings_group_services") }
                IndexRow {
                    icon: "fa-tags",
                    title: SettingsCategory::Metadata.title(),
                    on_click: move |_| on_select.call(SettingsCategory::Metadata),
                }
                IndexRow {
                    icon: "fa-satellite-dish",
                    title: SettingsCategory::Connectivity.title(),
                    on_click: move |_| on_select.call(SettingsCategory::Connectivity),
                }
            }
        }
    }
}

#[component]
fn IndexRow(
    icon: &'static str,
    title: String,
    #[props(default)] summary: String,
    #[props(default)] online: bool,
    on_click: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: "settings-index-row",
            onclick: move |evt| on_click.call(evt),
            i { class: "fa-solid {icon} settings-index-icon", aria_hidden: "true" }
            span { class: "settings-index-text",
                span { class: "settings-index-title", "{title}" }
                if !summary.is_empty() {
                    span { class: "settings-index-summary",
                        if online {
                            i { class: "settings-index-online", aria_hidden: "true" }
                        }
                        "{summary}"
                    }
                }
            }
            i { class: "fa-solid fa-chevron-right settings-index-chevron", aria_hidden: "true" }
        }
    }
}
