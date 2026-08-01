use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsCategory {
    General,
    Customization,
    Library,
    Connectivity,
    Downloads,
    Metadata,
    Player,
    Tools,
}

#[component]
fn SettingsFanItem(
    category: SettingsCategory,
    selected: SettingsCategory,
    label: String,
    icon: &'static str,
    angle: &'static str,
    label_x: &'static str,
    label_y: &'static str,
    label_tilt: &'static str,
    on_select: EventHandler<SettingsCategory>,
) -> Element {
    let is_selected = category == selected;
    let selected_class = if is_selected { "is-selected" } else { "" };

    rsx! {
        button {
            r#type: "button",
            class: "settings-fan-item {selected_class}",
            style: "--fan-angle: {angle};",
            aria_label: "{label}",
            aria_pressed: is_selected,
            title: "{label}",
            onclick: move |_| on_select.call(category),
            span { class: "settings-fan-mobile-label", aria_hidden: "true",
                i { class: "fa-solid {icon}" }
                span { "{label}" }
            }
        }
        span {
            class: "settings-fan-label {selected_class}",
            style: "--label-x: {label_x}; --label-y: {label_y}; --label-tilt: {label_tilt};",
            aria_hidden: "true",
            i { class: "fa-solid {icon}" }
            span { "{label}" }
        }
    }
}

#[component]
pub(super) fn SettingsNavigation(
    selected: SettingsCategory,
    on_select: EventHandler<SettingsCategory>,
) -> Element {
    rsx! {
        nav {
            class: "settings-fan",
            aria_label: i18n::t("settings"),
            div { class: "settings-fan-disc",
                SettingsFanItem {
                    category: SettingsCategory::General,
                    selected,
                    label: i18n::t("general").to_string(),
                    icon: "fa-sliders",
                    angle: "-78.75deg",
                    label_x: "12.92%",
                    label_y: "-32.49%",
                    label_tilt: "-78.75deg",
                    on_select,
                }
                SettingsFanItem {
                    category: SettingsCategory::Customization,
                    selected,
                    label: i18n::t("appearance").to_string(),
                    icon: "fa-paintbrush",
                    angle: "-56.25deg",
                    label_x: "36.81%",
                    label_y: "-27.54%",
                    label_tilt: "-56.25deg",
                    on_select,
                }
                SettingsFanItem {
                    category: SettingsCategory::Library,
                    selected,
                    label: i18n::t("library").to_string(),
                    icon: "fa-music",
                    angle: "-33.75deg",
                    label_x: "55.08%",
                    label_y: "-18.4%",
                    label_tilt: "-33.75deg",
                    on_select,
                }
                SettingsFanItem {
                    category: SettingsCategory::Connectivity,
                    selected,
                    label: i18n::t("connectivity").to_string(),
                    icon: "fa-satellite-dish",
                    angle: "-11.25deg",
                    label_x: "64.98%",
                    label_y: "-6.46%",
                    label_tilt: "-11.25deg",
                    on_select,
                }
                SettingsFanItem {
                    category: SettingsCategory::Downloads,
                    selected,
                    label: i18n::t("offline_downloads").to_string(),
                    icon: "fa-cloud-arrow-down",
                    angle: "11.25deg",
                    label_x: "64.98%",
                    label_y: "6.46%",
                    label_tilt: "11.25deg",
                    on_select,
                }
                SettingsFanItem {
                    category: SettingsCategory::Metadata,
                    selected,
                    label: i18n::t("metadata").to_string(),
                    icon: "fa-tags",
                    angle: "33.75deg",
                    label_x: "55.08%",
                    label_y: "18.4%",
                    label_tilt: "33.75deg",
                    on_select,
                }
                SettingsFanItem {
                    category: SettingsCategory::Player,
                    selected,
                    label: i18n::t("player_settings").to_string(),
                    icon: "fa-wave-square",
                    angle: "56.25deg",
                    label_x: "36.81%",
                    label_y: "27.54%",
                    label_tilt: "56.25deg",
                    on_select,
                }
                if !cfg!(target_os = "android") {
                    SettingsFanItem {
                        category: SettingsCategory::Tools,
                        selected,
                        label: i18n::t("logs").to_string(),
                        icon: "fa-screwdriver-wrench",
                        angle: "78.75deg",
                        label_x: "12.92%",
                        label_y: "32.49%",
                        label_tilt: "78.75deg",
                        on_select,
                    }
                }
            }
        }
    }
}
