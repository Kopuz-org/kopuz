use crate::constants::*;
use crate::showcase::{self, SortField};
use dioxus::core::Element;
use dioxus::prelude::*;

fn icon_class(
    sort_state: &Option<Signal<Option<(SortField, showcase::SortDirection)>>>,
    field: SortField,
) -> String {
    if let Some(s) = sort_state {
        showcase::sort_icon(*s.read(), field).to_string()
    } else {
        String::new()
    }
}

#[component]
pub fn Header(
    is_vaxry: bool,
    is_album: bool,
    #[props(default = false)] is_selection_mode: bool,
    #[props(default = None)] on_select_all: Option<EventHandler<bool>>,
    #[props(default = false)] all_selected: bool,
    #[props(default = None)] sort_state: Option<
        Signal<Option<(SortField, showcase::SortDirection)>>,
    >,
    #[props(default = false)] is_reorderable: bool,
) -> Element {
    let columns_vaxry = if is_album {
        COLUMNS_VAXRY_ALBUM
    } else {
        COLUMNS_VAXRY
    };

    let columns_normal = if is_album {
        COLUMNS_NORMAL_ALBUM
    } else {
        COLUMNS_NORMAL
    };

    if cfg!(target_os = "android") {
        if sort_state.is_none() && !is_selection_mode {
            return rsx! {};
        }
        let chip = move |field: SortField, label: String| {
            let active = sort_state
                .map(|state| matches!(*state.read(), Some((current, _)) if current == field))
                .unwrap_or(false);
            let chip_class = if active {
                "shrink-0 h-8 px-3 rounded-full bg-white/15 text-white text-[11px] font-semibold flex items-center gap-1.5 active:scale-95 transition-transform"
            } else {
                "shrink-0 h-8 px-3 rounded-full bg-white/5 text-white/55 text-[11px] font-medium flex items-center gap-1.5 active:scale-95 transition-transform"
            };
            rsx! {
                button {
                    class: "{chip_class}",
                    onclick: move |_| {
                        if let Some(sort_state) = sort_state {
                            showcase::toggle_sort_state(sort_state, field);
                        }
                    },
                    "{label}"
                    i { class: "{icon_class(&sort_state, field)} text-[9px]" }
                }
            }
        };

        return rsx! {
            div { class: "flex items-center gap-2 px-1 pb-2 mb-1 overflow-x-auto",
                if is_selection_mode {
                    if let Some(handler) = on_select_all {
                        button {
                            class: if all_selected {
                                "shrink-0 w-8 h-8 rounded-full bg-indigo-500 text-white flex items-center justify-center active:scale-95 transition-transform"
                            } else {
                                "shrink-0 w-8 h-8 rounded-full bg-white/5 text-white/55 flex items-center justify-center active:scale-95 transition-transform"
                            },
                            aria_label: if all_selected { i18n::t("deselect_all_tracks") } else { i18n::t("select_all_tracks") },
                            onclick: move |_| handler.call(!all_selected),
                            i { class: "fa-solid fa-check text-[11px]" }
                        }
                    }
                }
                if sort_state.is_some() {
                    {chip(SortField::Title, i18n::t("title"))}
                    {chip(SortField::Artist, i18n::t("artist"))}
                    if !is_album {
                        {chip(SortField::Album, i18n::t("album"))}
                    }
                    {chip(SortField::Duration, i18n::t("duration"))}
                }
            }
        };
    }

    let show_row_images = use_context::<Signal<config::Settings>>()
        .read()
        .ui
        .show_row_images;
    let covers = !is_album && show_row_images;

    // Every sort target fills its column at the header's full height, and the
    // lead cell reaches out the same way the rows' does, so the select-all box
    // sits over the row checkboxes and the title label over the row titles.
    let sort_button = move |field: SortField, class: &'static str, label: Element| {
        if sort_state.is_none() {
            // Nothing to sort by here, so the label is only a label.
            return rsx! {
                div { class: "justify-self-stretch flex items-center gap-1 {class}", {label} }
            };
        }
        rsx! {
            button {
                r#type: "button",
                class: "justify-self-stretch flex items-center gap-1 rounded-md text-left hover:bg-white/5 hover:text-white active:bg-white/10 transition-colors {class}",
                onclick: move |_| {
                    if let Some(sort_state) = sort_state {
                        showcase::toggle_sort_state(sort_state, field);
                    }
                },
                {label}
            }
        }
    };
    let lead = move |reach: &'static str| {
        rsx! {
            HeaderLead {
                reach,
                is_selection_mode,
                on_select_all,
                all_selected,
            }
        }
    };

    if is_vaxry {
        let (title_class, other_class, duration_class) = (
            if covers {
                "h-10 ps-10 pe-3"
            } else {
                "h-10 pe-3"
            },
            "h-10 pe-3",
            "h-10 justify-end",
        );
        return rsx! {
            div {
                class: "grid px-3 text-[10px] font-bold text-white/50 border-white/10 border-b mb-1",
                style: "grid-template-columns: {columns_vaxry};",
                {lead("h-10 -ms-2")}
                {sort_button(SortField::Title, title_class, rsx! {
                    "{i18n::t(\"title\")}"
                    i { class: "{icon_class(&sort_state, SortField::Title)} text-[9px]" }
                })}
                {sort_button(SortField::Artist, other_class, rsx! {
                    "{i18n::t(\"artist\")}"
                    i { class: "{icon_class(&sort_state, SortField::Artist)} text-[9px]" }
                })}
                if !is_album {
                    {sort_button(SortField::Album, other_class, rsx! {
                        "{i18n::t(\"album\")}"
                        i { class: "{icon_class(&sort_state, SortField::Album)} text-[9px]" }
                    })}
                }
                {sort_button(SortField::Duration, duration_class, rsx! {
                    i { class: "fa-regular fa-clock" }
                    i { class: "{icon_class(&sort_state, SortField::Duration)} text-[9px]" }
                })}
                div {}
            }
        };
    }

    // Matches the rows: the lead cell takes the whole gap up to a cover and
    // half of it before bare text; the other columns split each gap.
    let (lead_reach, title_class) = if covers {
        ("h-12 -ms-2 -me-6", "h-12 -me-3 ps-14 pe-3")
    } else {
        ("h-12 -ms-2 -me-3", "h-12 -me-3 pe-3")
    };
    let other_class = "h-12 -mx-3 px-3";
    let duration_class = "h-12 -mx-3 px-3 justify-end";

    rsx! {
        div { class: "flex items-center mb-2",
            div {
                class: "grid flex-1 gap-6 px-2 border-b border-white/10 text-sm font-medium text-white/50 items-center",
                style: "grid-template-columns: {columns_normal};",
                {lead(lead_reach)}
                {sort_button(SortField::Title, title_class, rsx! {
                    "{i18n::t(\"title\")}"
                    i { class: "{icon_class(&sort_state, SortField::Title)} text-[10px]" }
                })}
                {sort_button(SortField::Artist, other_class, rsx! {
                    "{i18n::t(\"artist\")}"
                    i { class: "{icon_class(&sort_state, SortField::Artist)} text-[10px]" }
                })}
                if !is_album {
                    {sort_button(SortField::Album, other_class, rsx! {
                        "{i18n::t(\"album\")}"
                        i { class: "{icon_class(&sort_state, SortField::Album)} text-[10px]" }
                    })}
                }
                {sort_button(SortField::Duration, duration_class, rsx! {
                    i { class: "fa-regular fa-clock" }
                    i { class: "{icon_class(&sort_state, SortField::Duration)} text-[10px]" }
                })}
                div {}
            }
            if is_reorderable && !is_selection_mode {
                div { class: "shrink-0", style: "width: 2.25rem;" }
            }
        }
    }
}

/// The header's first column: `#`, or in selection mode a select-all target
/// spanning the same area as the rows' lead cells.
#[component]
fn HeaderLead(
    reach: &'static str,
    is_selection_mode: bool,
    on_select_all: Option<EventHandler<bool>>,
    all_selected: bool,
) -> Element {
    let Some(handler) = on_select_all.filter(|_| is_selection_mode) else {
        return rsx! {
            div { class: "flex items-center justify-center {reach}", "#" }
        };
    };
    rsx! {
        button {
            r#type: "button",
            class: "group/lead flex items-center justify-center rounded-s-lg hover:bg-white/5 active:bg-white/10 transition-colors {reach}",
            aria_label: if all_selected { i18n::t("deselect_all_tracks") } else { i18n::t("select_all_tracks") },
            onclick: move |_| handler.call(!all_selected),
            span {
                class: if all_selected {
                    "w-4 h-4 rounded border border-indigo-400 bg-indigo-500 text-white flex items-center justify-center transition-colors"
                } else {
                    "w-4 h-4 rounded border border-white/20 bg-white/5 group-hover/lead:border-white/50 transition-colors"
                },
                if all_selected {
                    i { class: "fa-solid fa-check", style: "font-size: 9px;" }
                }
            }
        }
    }
}
