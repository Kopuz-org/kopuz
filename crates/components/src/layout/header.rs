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

    // Phones get sort chips instead of a column header. The Android track row
    // is artwork plus two lines, so a "# / Title / Artist / Album / clock"
    // strip labels columns that are not on screen — and the sort it carries is
    // the only sort affordance the album and playlist pages have.
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
                            aria_label: if all_selected { "Deselect all tracks" } else { "Select all tracks" },
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

    if is_vaxry {
        return rsx! {
            div {
                class: "grid px-3 py-2 text-[10px] font-bold text-white/50 border-white/10 border-b mb-1",
                style: "grid-template-columns: {columns_vaxry};",
                div {
                    class: "flex items-center h-4 shrink-0",
                    if is_selection_mode {
                        if let Some(handler) = on_select_all {
                            div { class: "flex items-center w-6 h-6 shrink-0",
                                  button {
                                      class: if all_selected {
                                          "w-4 h-4 rounded border border-indigo-400 bg-indigo-500 text-white flex items-center justify-center transition-colors"
                                      } else {
                                          "w-4 h-4 rounded border border-white/20 bg-white/5 hover:border-white/50 transition-colors"
                                      },
                                      aria_label: if all_selected { "Deselect all tracks" } else { "Select all tracks" },
                                      onclick: move |_| handler.call(!all_selected),
                                      if all_selected {
                                          i { class: "fa-solid fa-check", style: "font-size: 9px;" }
                                      }
                                  }
                            }
                        }
                    } else {
                        "#"
                    }
                }
                button {
                    class: "flex items-center gap-1 text-left hover:text-white transition-colors",
                    onclick: move |_| {
                        if let Some(sort_state) = sort_state {
                            showcase::toggle_sort_state(sort_state, SortField::Title);
                        }
                    },
                    "{i18n::t(\"title\")}"
                        i { class: "{icon_class(&sort_state, SortField::Title)} text-[9px]" }
                }
                button {
                    class: "flex items-center gap-1 text-left hover:text-white transition-colors",
                    onclick: move |_| {
                        if let Some(sort_state) = sort_state {
                            showcase::toggle_sort_state(sort_state, SortField::Artist);
                        }
                    },
                    "{i18n::t(\"artist\")}"
                        i { class: "{icon_class(&sort_state, SortField::Artist)} text-[9px]" }
                }
                if !is_album {
                    button {
                        class: "flex items-center gap-1 text-left hover:text-white transition-colors",
                        onclick: move |_| {
                            if let Some(sort_state) = sort_state {
                                showcase::toggle_sort_state(sort_state, SortField::Album);
                            }
                        },
                        "{i18n::t(\"album\")}"
                            i { class: "{icon_class(&sort_state, SortField::Album)} text-[9px]" }
                    }
                }
                button {
                    class: "flex items-center justify-end gap-1 text-right hover:text-white transition-colors",
                    onclick: move |_| {
                        if let Some(sort_state) = sort_state {
                            showcase::toggle_sort_state(sort_state, SortField::Duration);
                        }
                    },
                    i { class: "fa-regular fa-clock" }
                    i { class: "{icon_class(&sort_state, SortField::Duration)} text-[9px]" }
                }
                div {}
            }
        };
    } else {
        rsx! {
            div { class: "flex items-center mb-2",
                  div {
                      class: if cfg!(target_os = "android") { "grid flex-1 gap-2 px-2 py-2 border-b border-white/10 text-sm font-medium text-white/50 items-center" } else { "grid flex-1 gap-6 px-2 py-2 border-b border-white/10 text-sm font-medium text-white/50 items-center" },
                      style: "grid-template-columns: {columns_normal};",
                      div { class: "flex justify-center items-center h-6 shrink-0",
                            if is_selection_mode {
                                if let Some(handler) = on_select_all {
                                    div { class: "flex items-center justify-center shrink-0",
                                          button {
                                              class: if all_selected {
                                                  "w-4 h-4 rounded border border-indigo-400 bg-indigo-500 text-white flex items-center justify-center transition-colors"
                                              } else {
                                                  "w-4 h-4 rounded border border-white/20 bg-white/5 hover:border-white/50 transition-colors"
                                              },
                                              aria_label: if all_selected { "Deselect all tracks" } else { "Select all tracks" },
                                              onclick: move |_| handler.call(!all_selected),
                                              if all_selected {
                                                  i { class: "fa-solid fa-check", style: "font-size: 9px;" }
                                              }
                                          }
                                    }
                                }
                            } else {
                                "#"
                            }
                      }
                      button {
                          class: "flex items-center gap-1 text-left hover:text-white transition-colors",
                          onclick: move |_| {
                              if let Some(sort_state) = sort_state {
                                  showcase::toggle_sort_state(sort_state, SortField::Title);
                              }
                          },
                          "{i18n::t(\"title\")}"
                              i { class: "{icon_class(&sort_state, SortField::Title)} text-[10px]" }
                      }
                      button {
                          class: "flex items-center gap-1 text-left hover:text-white transition-colors",
                          onclick: move |_| {
                              if let Some(sort_state) = sort_state {
                                  showcase::toggle_sort_state(sort_state, SortField::Artist);
                              }
                          },
                          "{i18n::t(\"artist\")}"
                              i { class: "{icon_class(&sort_state, SortField::Artist)} text-[10px]" }
                      }
                      if !is_album {
                          button {
                              class: "flex items-center gap-1 text-left hover:text-white transition-colors",
                              onclick: move |_| {
                                  if let Some(sort_state) = sort_state {
                                      showcase::toggle_sort_state(sort_state, SortField::Album);
                                  }
                              },
                              "{i18n::t(\"album\")}"
                                  i { class: "{icon_class(&sort_state, SortField::Album)} text-[10px]" }
                          }
                      }
                      button {
                          class: "flex items-center justify-end gap-1 text-right hover:text-white transition-colors",
                          onclick: move |_| {
                              if let Some(sort_state) = sort_state {
                                  showcase::toggle_sort_state(sort_state, SortField::Duration);
                              }
                          },
                          i { class: "fa-regular fa-clock" }
                          i { class: "{icon_class(&sort_state, SortField::Duration)} text-[10px]" }
                      }
                      div {}
                  }
                  if is_reorderable && !is_selection_mode {
                      div { class: "pr-2 shrink-0", style: "width: 22px;" }
                  }
            }
        }
    }
}
