//! The header menu of a playlist the source keeps on its side: rating it,
//! editing its description and privacy, and deleting it.
//!
//! What a delete does depends on whose playlist it is. One the account owns
//! is deleted at the source; one it only saved is taken out of the account's
//! library and stays where it is. The source says which by giving an owned
//! playlist a privacy, so the menu reads the playlist before it offers either.

use crate::catalog_actions::{self, CatalogAction};
use crate::dots_menu::{DotsMenu, MenuAction};
use api::{CatalogDetail, PlaylistPrivacy};
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Catalog(CatalogAction),
    EditDetails,
    Delete,
}

#[component]
pub fn RemotePlaylistMenu(
    playlist_id: String,
    name: String,
    on_deleted: EventHandler<()>,
) -> Element {
    let caps = hooks::sources::use_capabilities();
    let source = hooks::sources::use_active_source_info();
    let mut detail = use_signal(|| None::<CatalogDetail>);
    let mut open = use_signal(|| false);
    let mut editing = use_signal(|| false);
    let mut confirming = use_signal(|| false);

    let load_id = playlist_id.clone();
    use_effect(move || {
        let id = load_id.clone();
        let api = hooks::consume_api();
        spawn(async move {
            let request = api::CatalogDetailRequest::new(api::CatalogItemKind::Playlist, id);
            match api.catalog_detail(request).await {
                Ok(answer) => detail.set(Some(answer)),
                Err(error) => tracing::debug!(%error, "reading the playlist's details failed"),
            }
        });
    });

    let Some(loaded) = detail.read().clone() else {
        return rsx! {};
    };
    let capabilities = caps.read().clone();
    let owned = loaded.privacy.is_some();
    let service = source
        .read()
        .as_ref()
        .map(|source| crate::forms::text(&source.service.name))
        .unwrap_or_default();

    let deletes = capabilities.playlists != api::PlaylistCapability::None;
    let mut entries: Vec<(Action, MenuAction)> =
        catalog_actions::menu_entries(&loaded.actions, &capabilities)
            .into_iter()
            // Deleting a saved playlist is taking it out of the library, with a
            // confirmation, so a second unconfirmed entry for the same would be noise.
            .filter(|(action, _)| !(deletes && !owned && *action == CatalogAction::Save))
            .map(|(action, entry)| (Action::Catalog(action), entry))
            .collect();
    if owned && capabilities.playlist_details {
        entries.push((
            Action::EditDetails,
            MenuAction::new(i18n::t("playlist_edit_details"), "fa-solid fa-pen"),
        ));
    }
    if deletes {
        entries.push((
            Action::Delete,
            if owned {
                MenuAction::new(i18n::t("delete_playlist"), "fa-solid fa-trash").destructive()
            } else {
                MenuAction::new(i18n::t("catalog_unsave"), "fa-solid fa-bookmark").destructive()
            },
        ));
    }
    if entries.is_empty() {
        return rsx! {};
    }
    let dispatch: Vec<Action> = entries.iter().map(|(action, _)| *action).collect();
    let actions: Vec<MenuAction> = entries.into_iter().map(|(_, entry)| entry).collect();
    let state = loaded.actions.clone();

    rsx! {
        DotsMenu {
            actions,
            is_open: open(),
            aria_label: i18n::t_with("more_actions_for", &[("name", name.clone())]),
            title: name.clone(),
            icon: "fa-solid fa-ellipsis".to_string(),
            button_class: "w-14! h-14! text-xl!".to_string(),
            anchor: "left".to_string(),
            on_open: move |_| open.set(true),
            on_close: move |_| open.set(false),
            on_action: move |idx: usize| {
                open.set(false);
                match dispatch.get(idx).copied() {
                    Some(Action::Catalog(action)) => catalog_actions::run(
                        action,
                        &state,
                        Some(EventHandler::new(move |next| {
                            if let Some(current) = detail.write().as_mut() {
                                current.actions = next;
                            }
                        })),
                    ),
                    Some(Action::EditDetails) => editing.set(true),
                    Some(Action::Delete) => confirming.set(true),
                    None => {}
                }
            },
        }

        if editing() {
            EditDetailsPopup {
                playlist_id: playlist_id.clone(),
                description: loaded.description.clone().unwrap_or_default(),
                privacy: loaded.privacy.unwrap_or(PlaylistPrivacy::Private),
                on_close: move |_| editing.set(false),
                on_saved: move |(description, privacy): (String, PlaylistPrivacy)| {
                    if let Some(current) = detail.write().as_mut() {
                        current.description = Some(description);
                        current.privacy = Some(privacy);
                    }
                    editing.set(false);
                },
            }
        }

        if confirming() {
            {
                let (title, body, confirm) = if owned {
                    (
                        i18n::t("playlist_delete_title"),
                        i18n::t_with("playlist_delete_body", &[("name", name.clone()), ("service", service.clone())]),
                        i18n::t("delete_playlist"),
                    )
                } else {
                    (
                        i18n::t("playlist_unsave_title"),
                        i18n::t_with("playlist_unsave_body", &[("name", name.clone()), ("service", service.clone())]),
                        i18n::t("catalog_unsave"),
                    )
                };
                let id = playlist_id.clone();
                rsx! {
                    div {
                        class: "overlay",
                        onclick: move |_| confirming.set(false),
                        div {
                            class: "popup",
                            role: "alertdialog",
                            aria_modal: "true",
                            onclick: |evt| evt.stop_propagation(),
                            h2 { "{title}" }
                            p { class: "text-sm text-white/60 text-pretty", "{body}" }
                            div { class: "flex justify-end gap-3 mt-2",
                                button {
                                    class: "app-button-outlined px-4 py-2 rounded-lg text-sm font-medium text-slate-400 hover:text-white border border-white/10 hover:bg-white/5 transition-colors active:scale-95 cursor-pointer",
                                    onclick: move |_| confirming.set(false),
                                    "{i18n::t(\"cancel\")}"
                                }
                                button {
                                    class: "app-button-tonal app-button-danger px-4 py-2 rounded-lg text-sm font-medium bg-red-500/20 hover:bg-red-500/30 text-red-300 transition-colors active:scale-95 cursor-pointer",
                                    onclick: move |_| {
                                        confirming.set(false);
                                        // Leaving the page drops this component and every task it
                                        // spawned, so the page is left only once the source answered.
                                        let api = hooks::consume_api();
                                        let id = id.clone();
                                        spawn(async move {
                                            match api.delete_playlist(id).await {
                                                Ok(()) => on_deleted.call(()),
                                                Err(error) => {
                                                    tracing::warn!(%error, "deleting a playlist failed");
                                                    hooks::toast::toast_error(&error.to_string());
                                                }
                                            }
                                        });
                                    },
                                    "{confirm}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn EditDetailsPopup(
    playlist_id: String,
    description: String,
    privacy: PlaylistPrivacy,
    on_close: EventHandler<()>,
    on_saved: EventHandler<(String, PlaylistPrivacy)>,
) -> Element {
    let mut text = use_signal(|| description.clone());
    let mut picked = use_signal(|| privacy);
    let mut saving = use_signal(|| false);
    let choices = [
        (PlaylistPrivacy::Public, "playlist_privacy_public"),
        (PlaylistPrivacy::Unlisted, "playlist_privacy_unlisted"),
        (PlaylistPrivacy::Private, "playlist_privacy_private"),
    ];
    rsx! {
        div {
            class: "overlay",
            onclick: move |_| on_close.call(()),
            div {
                class: "popup",
                role: "dialog",
                aria_modal: "true",
                onclick: |evt| evt.stop_propagation(),
                h2 { "{i18n::t(\"playlist_edit_details\")}" }
                label { class: "flex flex-col gap-2 text-sm text-white/60",
                    "{i18n::t(\"playlist_description\")}"
                    textarea {
                        class: "min-h-24 bg-white/5 border border-white/10 rounded-lg px-3 py-2.5 text-sm text-white focus:outline-none focus:border-white/20 resize-y",
                        value: "{text}",
                        oninput: move |evt| text.set(evt.value()),
                        onkeydown: move |evt| evt.stop_propagation(),
                    }
                }
                div { class: "flex flex-col gap-2",
                    span { class: "text-sm text-white/60", "{i18n::t(\"playlist_privacy\")}" }
                    div { class: "flex gap-2", role: "radiogroup",
                        for (choice, key) in choices {
                            button {
                                key: "{key}",
                                role: "radio",
                                aria_checked: if picked() == choice { "true" } else { "false" },
                                class: if picked() == choice {
                                    "app-chip text-xs px-3 py-1.5 rounded-lg bg-white/20 text-white font-medium transition-colors cursor-pointer"
                                } else {
                                    "app-chip text-xs px-3 py-1.5 rounded-lg bg-white/5 text-slate-400 hover:text-white hover:bg-white/10 transition-colors cursor-pointer"
                                },
                                onclick: move |_| picked.set(choice),
                                "{i18n::t(key)}"
                            }
                        }
                    }
                }
                div { class: "actions",
                    button { onclick: move |_| on_close.call(()), "{i18n::t(\"cancel\")}" }
                    button {
                        disabled: saving(),
                        onclick: {
                            let id = playlist_id.clone();
                            move |_| {
                                saving.set(true);
                                let description = text.peek().clone();
                                let privacy = *picked.peek();
                                let edit = api::PlaylistEdit {
                                    name: None,
                                    description: Some(description.clone()),
                                    privacy: Some(privacy),
                                };
                                hooks::playlist_actions::edit(id.clone(), edit, move || {
                                    on_saved.call((description, privacy));
                                });
                            }
                        },
                        "{i18n::t(\"save\")}"
                    }
                }
            }
        }
    }
}
