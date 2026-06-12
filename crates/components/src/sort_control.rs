//! Multi-criteria sort control shared by the library and album tabs.
//!
//! Renders a compact "Sort" pill that opens a popover where the user stacks
//! ordered sort criteria (field + direction). Generic over the tab's field
//! enum via [`config::LibrarySortField`], so the same component drives both
//! the Tracks and Albums tabs. Backs issues #265 and #351.

use config::{LibrarySortField, SortCriterion, SortDirection};
use dioxus::prelude::*;

/// `criteria` is the persisted, ordered list of sort criteria for a tab.
/// Mutating it re-sorts the view (the caller mirrors it into config).
///
/// `available` is the set of fields to offer, in display order — the caller
/// derives it from the data actually loaded (see
/// `reader::sort::available_track_fields`), so fields with no backing data
/// (e.g. release year / date added for YT Music) are never shown as dead
/// options. Always contains at least Title and Artist.
#[component]
pub fn SortControl<F: LibrarySortField + Clone + PartialEq + 'static>(
    mut criteria: Signal<Vec<SortCriterion<F>>>,
    available: Vec<F>,
) -> Element {
    let mut is_open = use_signal(|| false);
    let fields = available;

    // Button summary: primary field + direction, or a bare "Sort" label.
    let summary = criteria.read().first().map(|c| {
        let label = i18n::t(c.field.label_key()).to_string();
        let arrow = direction_arrow(c.direction);
        (label, arrow)
    });

    rsx! {
        div { class: "relative",
            button {
                class: "flex items-center gap-2 px-3 py-1.5 text-xs rounded-lg bg-white/5 border border-white/5 text-white/70 hover:text-white hover:bg-white/10 transition-all",
                onclick: move |evt| {
                    evt.stop_propagation();
                    let next = !*is_open.peek();
                    is_open.set(next);
                },
                i { class: "fa-solid fa-arrow-down-short-wide", style: "font-size: 11px;" }
                match summary {
                    Some((label, arrow)) => rsx! {
                        span { "{i18n::t(\"sort_by\")}: {label}" }
                        i { class: "{arrow}", style: "font-size: 10px;" }
                    },
                    None => rsx! {
                        span { "{i18n::t(\"sort_by\")}" }
                    },
                }
            }

            if *is_open.read() {
                // Click-outside backdrop.
                div {
                    class: "fixed inset-0 z-40",
                    onclick: move |evt| {
                        evt.stop_propagation();
                        is_open.set(false);
                    }
                }

                div {
                    class: "absolute right-0 top-full mt-1 z-50 w-72 bg-neutral-900 border border-white/10 rounded-xl shadow-2xl p-2 space-y-1",
                    onclick: move |evt| evt.stop_propagation(),

                    if criteria.read().is_empty() {
                        p { class: "px-2 py-2 text-xs text-white/40",
                            "{i18n::t(\"sort_none\")}"
                        }
                    }

                    for (idx, criterion) in criteria.read().iter().enumerate() {
                        {
                            let current = criterion.field;
                            let direction = criterion.direction;
                            // Offer the available fields, plus this row's own
                            // field if it isn't available in the current data
                            // (e.g. a Year criterion persisted from another
                            // source) so the select still renders correctly.
                            let mut row_fields = fields.clone();
                            if !row_fields.contains(&current) {
                                row_fields.insert(0, current);
                            }
                            let selected_pos =
                                row_fields.iter().position(|f| *f == current).unwrap_or(0);
                            let onchange_fields = row_fields.clone();
                            rsx! {
                                div {
                                    key: "{idx}",
                                    class: "flex items-center gap-1.5",

                                    span { class: "w-10 shrink-0 text-[10px] uppercase tracking-wider text-white/30",
                                        if idx == 0 { "{i18n::t(\"sort_by\")}" } else { "{i18n::t(\"sort_then\")}" }
                                    }

                                    select {
                                        class: "flex-1 min-w-0 bg-neutral-800 text-white text-xs rounded-md px-2 py-1.5 border border-white/10 focus:outline-none focus:border-white/30",
                                        value: "{selected_pos}",
                                        onchange: move |evt| {
                                            if let Ok(pos) = evt.value().parse::<usize>()
                                                && let Some(field) = onchange_fields.get(pos).copied()
                                                && let Some(c) = criteria.write().get_mut(idx)
                                            {
                                                c.field = field;
                                            }
                                        },
                                        for (pos, field) in row_fields.iter().enumerate() {
                                            // Mark the match `selected` declaratively: on
                                            // remount (e.g. switching tabs and back) the
                                            // `value` attribute can be applied before the
                                            // options exist, leaving the wrong option shown.
                                            option {
                                                key: "{pos}",
                                                value: "{pos}",
                                                selected: pos == selected_pos,
                                                "{i18n::t(field.label_key())}"
                                            }
                                        }
                                    }

                                    button {
                                        class: "shrink-0 w-7 h-7 flex items-center justify-center rounded-md bg-white/5 hover:bg-white/10 text-white/70 hover:text-white transition-colors",
                                        title: if direction == SortDirection::Asc { "{i18n::t(\"sort_ascending\")}" } else { "{i18n::t(\"sort_descending\")}" },
                                        onclick: move |evt| {
                                            evt.stop_propagation();
                                            if let Some(c) = criteria.write().get_mut(idx) {
                                                c.direction = match c.direction {
                                                    SortDirection::Asc => SortDirection::Desc,
                                                    SortDirection::Desc => SortDirection::Asc,
                                                };
                                            }
                                        },
                                        i { class: "{direction_arrow(direction)}", style: "font-size: 11px;" }
                                    }

                                    button {
                                        class: "shrink-0 w-7 h-7 flex items-center justify-center rounded-md text-white/30 hover:text-red-300 hover:bg-red-500/10 transition-colors",
                                        title: "{i18n::t(\"sort_remove\")}",
                                        onclick: move |evt| {
                                            evt.stop_propagation();
                                            let mut list = criteria.write();
                                            if idx < list.len() {
                                                list.remove(idx);
                                            }
                                        },
                                        i { class: "fa-solid fa-xmark", style: "font-size: 11px;" }
                                    }
                                }
                            }
                        }
                    }

                    // Only offer "add" while there are unused fields to add.
                    if criteria.read().len() < fields.len() {
                        button {
                            class: "w-full mt-1 px-2 py-1.5 text-xs rounded-md text-white/60 hover:text-white hover:bg-white/5 flex items-center gap-2 transition-colors",
                            onclick: move |evt| {
                                evt.stop_propagation();
                                if let Some(first) = fields.first().copied() {
                                    criteria.write().push(SortCriterion::new(first, SortDirection::Asc));
                                }
                            },
                            i { class: "fa-solid fa-plus", style: "font-size: 10px;" }
                            "{i18n::t(\"sort_add_criterion\")}"
                        }
                    }
                }
            }
        }
    }
}

fn direction_arrow(direction: SortDirection) -> &'static str {
    match direction {
        SortDirection::Asc => "fa-solid fa-arrow-up-short-wide",
        SortDirection::Desc => "fa-solid fa-arrow-down-wide-short",
    }
}
