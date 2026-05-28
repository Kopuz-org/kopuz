use crate::constants::*;
use crate::showcase::{self, SortField};
use dioxus::core::Element;
use dioxus::prelude::*;

fn icon_class(
    sort_state: &Option<Signal<Option<(SortField, showcase::SortDirection)>>>,
    field: SortField,
) -> String {
    if sort_state.is_some() {
        showcase::sort_icon(*sort_state.unwrap().read(), field).to_string()
    } else {
        "".to_string()
    }
}

#[component]
pub fn Header(
    is_modern: bool,
    is_album: bool,
    #[props(default = false)] is_selection_mode: bool,
    #[props(default = None)] on_select_all: Option<EventHandler<bool>>,
    #[props(default = false)] all_selected: bool,
    #[props(default = None)] sort_state: Option<
        Signal<Option<(SortField, showcase::SortDirection)>>,
    >,
    #[props(default = false)] is_reorderable: bool,
) -> Element {
    let columns_modern = if is_album {
        COLUMNS_MODERN_ALBUM
    } else {
        COLUMNS_MODERN
    };

    let columns_normal = if is_album {
        COLUMNS_NORMAL_ALBUM
    } else {
        COLUMNS_NORMAL
    };

    if is_modern {
        return rsx! {
            div {
                class: "grid px-3 py-2 text-[10px] font-bold text-white/50 border-white/10 uppercase tracking-widest border-b mb-1",
                style: "grid-template-columns: {columns_modern};",
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
                    class: "flex items-center gap-1 uppercase tracking-widest text-left hover:text-white transition-colors",
                    onclick: move |_| {
                        if sort_state.is_some() {
                            showcase::toggle_sort_state(sort_state.unwrap(), SortField::Title);
                        }
                    },
                    "{i18n::t(\"title\")}"
                        i { class: "{icon_class(&sort_state, SortField::Title)} text-[9px]" }
                }
                button {
                    class: "flex items-center gap-1 uppercase tracking-widest text-left hover:text-white transition-colors",
                    onclick: move |_| {
                        if sort_state.is_some() {
                            showcase::toggle_sort_state(sort_state.unwrap(), SortField::Artist);
                        }
                    },
                    "{i18n::t(\"artist\")}"
                        i { class: "{icon_class(&sort_state, SortField::Artist)} text-[9px]" }
                }
                if !is_album {
                    button {
                        class: "flex items-center gap-1 uppercase tracking-widest text-left hover:text-white transition-colors",
                        onclick: move |_| {
                            if sort_state.is_some() {
                                showcase::toggle_sort_state(sort_state.unwrap(), SortField::Album);
                            }
                        },
                        "{i18n::t(\"album\")}"
                            i { class: "{icon_class(&sort_state, SortField::Album)} text-[9px]" }
                    }
                }
                button {
                    class: "flex items-center justify-end gap-1 uppercase tracking-widest text-right hover:text-white transition-colors",
                    onclick: move |_| {
                        if sort_state.is_some() {
                            showcase::toggle_sort_state(sort_state.unwrap(), SortField::Duration);
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
                      class: if cfg!(target_os = "android") { "grid flex-1 gap-2 px-2 py-2 border-b border-white/10 text-sm font-medium text-white/50 uppercase tracking-wider items-center" } else { "grid flex-1 gap-6 px-2 py-2 border-b border-white/10 text-sm font-medium text-white/50 uppercase tracking-wider items-center" },
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
                          class: "flex items-center gap-1 uppercase tracking-wider text-left hover:text-white transition-colors",
                          onclick: move |_| {
                              if sort_state.is_some() {
                                  showcase::toggle_sort_state(sort_state.unwrap(), SortField::Title);
                              }
                          },
                          "{i18n::t(\"title\")}"
                              i { class: "{icon_class(&sort_state, SortField::Title)} text-[10px]" }
                      }
                      button {
                          class: "flex items-center gap-1 uppercase tracking-wider text-left hover:text-white transition-colors",
                          onclick: move |_| {
                              if sort_state.is_some() {
                                  showcase::toggle_sort_state(sort_state.unwrap(), SortField::Artist);
                              }
                          },
                          "{i18n::t(\"artist\")}"
                              i { class: "{icon_class(&sort_state, SortField::Artist)} text-[10px]" }
                      }
                      if !is_album {
                          button {
                              class: "flex items-center gap-1 uppercase tracking-wider text-left hover:text-white transition-colors",
                              onclick: move |_| {
                                  if sort_state.is_some() {
                                      showcase::toggle_sort_state(sort_state.unwrap(), SortField::Album);
                                  }
                              },
                              "{i18n::t(\"album\")}"
                                  i { class: "{icon_class(&sort_state, SortField::Album)} text-[10px]" }
                          }
                      }
                      button {
                          class: "flex items-center justify-end gap-1 uppercase tracking-wider text-right hover:text-white transition-colors",
                          onclick: move |_| {
                              if sort_state.is_some() {
                                  showcase::toggle_sort_state(sort_state.unwrap(), SortField::Duration);
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
