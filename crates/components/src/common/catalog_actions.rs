//! Rating, saving, following and history removal for a catalog entity.
//!
//! What can be done comes from two places: the source's capabilities say
//! whether it takes the action at all, and the entity's [`CatalogActions`]
//! carry the ref that does it and the state it is in. An entry or a button
//! exists only when both say yes. The source answers first, and the new state
//! is reported back through `on_change` once it has.

use crate::dots_menu::MenuAction;
use api::{CatalogActions, Rating, SourceCapabilities};
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CatalogAction {
    Like,
    Dislike,
    Save,
    Follow,
    RemoveFromHistory,
}

/// The menu entries an entity offers, in the order a menu lists them.
pub fn menu_entries(
    actions: &CatalogActions,
    caps: &SourceCapabilities,
) -> Vec<(CatalogAction, MenuAction)> {
    let mut entries = Vec::new();
    if caps.rate && actions.rate_ref.is_some() {
        let rating = actions.rating.unwrap_or_default();
        entries.push((
            CatalogAction::Like,
            if rating == Rating::Like {
                MenuAction::new(i18n::t("catalog_unlike"), "fa-solid fa-heart")
            } else {
                MenuAction::new(i18n::t("catalog_like"), "fa-regular fa-heart")
            },
        ));
        entries.push((
            CatalogAction::Dislike,
            if rating == Rating::Dislike {
                MenuAction::new(i18n::t("catalog_undislike"), "fa-solid fa-thumbs-down")
            } else {
                MenuAction::new(i18n::t("catalog_dislike"), "fa-regular fa-thumbs-down")
            },
        ));
    }
    if caps.save && actions.save_ref.is_some() {
        entries.push((
            CatalogAction::Save,
            if actions.saved == Some(true) {
                MenuAction::new(i18n::t("catalog_unsave"), "fa-solid fa-bookmark")
            } else {
                MenuAction::new(i18n::t("catalog_save"), "fa-regular fa-bookmark")
            },
        ));
    }
    if caps.follow && actions.follow_ref.is_some() {
        entries.push((
            CatalogAction::Follow,
            if actions.followed == Some(true) {
                MenuAction::new(i18n::t("catalog_unfollow"), "fa-solid fa-user-minus")
            } else {
                MenuAction::new(i18n::t("catalog_follow"), "fa-solid fa-user-plus")
            },
        ));
    }
    if caps.remove_from_history && actions.history_token.is_some() {
        entries.push((
            CatalogAction::RemoveFromHistory,
            MenuAction::new(
                i18n::t("catalog_remove_history"),
                "fa-solid fa-clock-rotate-left",
            )
            .destructive(),
        ));
    }
    entries
}

/// Carry an action out, then report the entity's new state. A removed
/// history row comes back without its token, which is a list's cue to drop it.
pub fn run(
    action: CatalogAction,
    actions: &CatalogActions,
    on_change: Option<EventHandler<CatalogActions>>,
) {
    let mut next = actions.clone();
    let report = move |next: CatalogActions| {
        if let Some(handler) = on_change {
            handler.call(next);
        }
    };
    match action {
        CatalogAction::Like | CatalogAction::Dislike => {
            let Some(item) = actions.rate_ref.clone() else {
                return;
            };
            let wanted = if action == CatalogAction::Like {
                Rating::Like
            } else {
                Rating::Dislike
            };
            // Picking the rating it already has takes it back.
            let rating = if actions.rating == Some(wanted) {
                Rating::None
            } else {
                wanted
            };
            next.rating = Some(rating);
            hooks::library_actions::rate(item, rating, move || report(next));
        }
        CatalogAction::Save => {
            let Some(item) = actions.save_ref.clone() else {
                return;
            };
            let saved = actions.saved != Some(true);
            next.saved = Some(saved);
            hooks::library_actions::save(item, saved, move || report(next));
        }
        CatalogAction::Follow => {
            let Some(artist) = actions.follow_ref.clone() else {
                return;
            };
            let follow = actions.followed != Some(true);
            next.followed = Some(follow);
            hooks::library_actions::follow(artist, follow, move || report(next));
        }
        CatalogAction::RemoveFromHistory => {
            let Some(token) = actions.history_token.clone() else {
                return;
            };
            next.history_token = None;
            hooks::library_actions::remove_from_history(token, move || report(next));
        }
    }
}

/// The same actions as buttons, for a page header. Rating and saving sit with
/// the header's other round buttons; following is a labelled pill, as it reads
/// as a state rather than a one-off action.
#[component]
pub fn CatalogActionButtons(
    actions: CatalogActions,
    on_change: EventHandler<CatalogActions>,
    /// The ringed size the album page's own header buttons use.
    #[props(default)]
    outlined: bool,
) -> Element {
    let caps = use_context::<Signal<SourceCapabilities>>().read().clone();
    let entries = menu_entries(&actions, &caps);
    if entries.is_empty() {
        return rsx! {};
    }
    rsx! {
        for (action, entry) in entries
            .into_iter()
            .filter(|(action, _)| *action != CatalogAction::RemoveFromHistory)
        {
            {
                let on = match action {
                    CatalogAction::Like => actions.rating == Some(Rating::Like),
                    CatalogAction::Dislike => actions.rating == Some(Rating::Dislike),
                    CatalogAction::Save => actions.saved == Some(true),
                    CatalogAction::Follow => actions.followed == Some(true),
                    CatalogAction::RemoveFromHistory => false,
                };
                let current = actions.clone();
                let click = move |_| run(action, &current, Some(on_change));
                if action == CatalogAction::Follow {
                    rsx! {
                        button {
                            key: "{action:?}",
                            class: if on {
                                "app-button-outlined inline-flex items-center gap-2 px-5 py-2.5 rounded-full border border-white/40 text-white hover:bg-white/10 transition-colors active:scale-95 cursor-pointer"
                            } else {
                                "app-button-outlined inline-flex items-center gap-2 px-5 py-2.5 rounded-full border border-white/20 text-white/70 hover:text-white hover:border-white/40 hover:bg-white/10 transition-colors active:scale-95 cursor-pointer"
                            },
                            aria_pressed: if on { "true" } else { "false" },
                            onclick: click,
                            i { class: "{entry.icon} text-[11px]", aria_hidden: "true" }
                            span { class: "text-sm font-bold",
                                if on { "{i18n::t(\"catalog_following\")}" } else { "{i18n::t(\"catalog_follow\")}" }
                            }
                        }
                    }
                } else {
                    rsx! {
                        button {
                            key: "{action:?}",
                            class: match (outlined, on) {
                                (true, true) => "w-11 h-11 rounded-full border border-white/30 flex items-center justify-center text-indigo-500 transition-colors active:scale-95 cursor-pointer",
                                (true, false) => "w-11 h-11 rounded-full border border-white/15 flex items-center justify-center text-slate-300 hover:text-white hover:border-white/30 transition-colors active:scale-95 cursor-pointer",
                                (false, true) => "w-14 h-14 rounded-full flex items-center justify-center text-indigo-500 hover:bg-white/10 transition-colors active:scale-95 cursor-pointer",
                                (false, false) => "w-14 h-14 rounded-full flex items-center justify-center text-slate-400 hover:text-white hover:bg-white/10 transition-colors active:scale-95 cursor-pointer",
                            },
                            title: "{entry.label}",
                            aria_label: "{entry.label}",
                            aria_pressed: if on { "true" } else { "false" },
                            onclick: click,
                            i {
                                class: if outlined { "{entry.icon}" } else { "{entry.icon} text-xl" },
                                aria_hidden: "true",
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_ref() -> CatalogActions {
        CatalogActions {
            rate_ref: Some("r".into()),
            rating: Some(Rating::Like),
            save_ref: Some("s".into()),
            saved: Some(false),
            follow_ref: Some("f".into()),
            followed: Some(true),
            history_token: Some("h".into()),
        }
    }

    fn kinds(entries: Vec<(CatalogAction, MenuAction)>) -> Vec<CatalogAction> {
        entries.into_iter().map(|(action, _)| action).collect()
    }

    #[test]
    fn a_source_that_takes_nothing_offers_nothing() {
        assert!(menu_entries(&every_ref(), &SourceCapabilities::default()).is_empty());
    }

    #[test]
    fn an_entry_needs_both_the_capability_and_the_ref() {
        let caps = SourceCapabilities {
            rate: true,
            save: true,
            follow: true,
            remove_from_history: true,
            ..SourceCapabilities::default()
        };
        assert_eq!(
            kinds(menu_entries(&every_ref(), &caps)),
            vec![
                CatalogAction::Like,
                CatalogAction::Dislike,
                CatalogAction::Save,
                CatalogAction::Follow,
                CatalogAction::RemoveFromHistory,
            ]
        );
        let bare = CatalogActions::default();
        assert!(menu_entries(&bare, &caps).is_empty());
    }
}
