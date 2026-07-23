//! Stacked multi-priority sort control (pr #510 follow-up): one dropdown per
//! criterion — field + direction — where the first field decides and the rest
//! break ties. Generic over the field enum via [`LibrarySortField`], so the
//! album grid, library track list and artist album grid share one control.

use config::{AlbumSortField, ArtistSortField, SortCriterion, SortDirection, TrackSortField};
use dioxus::prelude::*;

/// UI label lookup for a sortable field enum. Lives here rather than in
/// `config` because the i18n label keys are a presentation concern.
pub trait LibrarySortField: Copy + PartialEq + 'static {
    fn label_key(&self) -> &'static str;
}

impl LibrarySortField for AlbumSortField {
    fn label_key(&self) -> &'static str {
        match self {
            Self::Title => "sort_field_title",
            Self::Artist => "sort_field_artist",
            Self::Year => "sort_field_year",
            Self::Genre => "sort_field_genre",
        }
    }
}

impl LibrarySortField for TrackSortField {
    fn label_key(&self) -> &'static str {
        match self {
            Self::Title => "sort_field_title",
            Self::Artist => "sort_field_artist",
            Self::Album => "sort_field_album",
            Self::Duration => "sort_field_duration",
            Self::DateAdded => "sort_field_date_added",
        }
    }
}

impl LibrarySortField for ArtistSortField {
    fn label_key(&self) -> &'static str {
        match self {
            Self::Name => "sort_field_name",
            Self::Tracks => "sort_field_tracks",
            Self::Albums => "sort_field_albums",
        }
    }
}

fn direction_arrow(direction: SortDirection) -> &'static str {
    match direction {
        SortDirection::Asc => "fa-solid fa-arrow-up-short-wide",
        SortDirection::Desc => "fa-solid fa-arrow-down-wide-short",
    }
}

#[component]
pub fn SortControl<F: LibrarySortField + Eq + std::fmt::Debug + 'static>(
    mut criteria: Signal<Vec<SortCriterion<F>>>,
    available: Vec<F>,
) -> Element {
    let mut is_open = use_signal(|| false);
    let fields = available;

    let summary = criteria.read().first().map(|c| {
        let label = i18n::t(c.field.label_key()).to_string();
        (label, direction_arrow(c.direction))
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
                        p { class: "px-2 py-2 text-xs text-white/40", "{i18n::t(\"sort_none\")}" }
                    }

                    for (idx, criterion) in criteria.read().iter().enumerate() {
                        {
                            let current = criterion.field;
                            let direction = criterion.direction;
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
                                            option {
                                                key: "{pos}",
                                                value: "{pos}",
                                                selected: pos == selected_pos,
                                                "{i18n::t(field.label_key())}"
                                            }
                                        }
                                    }

                                    button {
                                        class: "shrink-0 w-7 h-7 flex items-center justify-center rounded-full bg-white/5 hover:bg-white/10 text-white/70 hover:text-white transition-colors active:scale-95",
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
                                        class: "shrink-0 w-7 h-7 flex items-center justify-center rounded-full text-white/30 hover:text-red-300 hover:bg-red-500/10 transition-colors active:scale-95",
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

                    if criteria.read().len() < fields.len() {
                        button {
                            class: "w-full mt-1 px-2 py-1.5 text-xs rounded-md text-white/60 hover:text-white hover:bg-white/5 flex items-center gap-2 transition-colors",
                            onclick: move |evt| {
                                evt.stop_propagation();
                                let used: Vec<F> = criteria.peek().iter().map(|c| c.field).collect();
                                if let Some(field) =
                                    fields.iter().copied().find(|f| !used.contains(f))
                                {
                                    criteria.write().push(SortCriterion::new(field, SortDirection::Asc));
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
