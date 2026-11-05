//! The catalog verbs: rate, save, follow, take out of history, and an opened source playlist's own menu.
//! An entry exists only when the source's capabilities take the verb and the item carries the ref for it.

use api::{CatalogActions, CatalogItemKind, PlaylistCapability, PlaylistEdit, Rating};
use components::format::{t, t_with};
use gpui_kit::{App, SharedString, Window};
use store::{CatalogEntry, CatalogMarks, Navigator, Route, SourcesModel, Target, actions};

use crate::ops::caps;

/// The refs a target carries, as the source now has them.
pub fn refs(target: &Target, cx: &App) -> Option<CatalogActions> {
    let marks = CatalogMarks::global(cx).read(cx);
    match target {
        Target::Catalog(entry) => Some(marks.current(&entry.actions)),
        Target::Tracks { keys, .. } if keys.len() == 1 => marks.for_track(&keys[0]),
        _ => None,
    }
}

fn entry(target: &Target) -> Option<&CatalogEntry> {
    match target {
        Target::Catalog(entry) => Some(entry),
        _ => None,
    }
}

/// An opened source playlist, whose menu deletes it instead of offering a second save toggle.
fn opened_playlist(target: &Target) -> Option<&CatalogEntry> {
    entry(target).filter(|entry| entry.opened && entry.kind == CatalogItemKind::Playlist)
}

pub fn can_rate(target: &Target, cx: &App) -> bool {
    let caps = caps(cx);
    // Rating a playlist or album saves it, so its heart and its bookmark would be one action twice.
    refs(target, cx).is_some_and(|refs| {
        caps.rate && refs.rate_ref.is_some() && !(caps.save && refs.rate_ref == refs.save_ref)
    })
}

pub fn rating(target: &Target, cx: &App) -> Rating {
    refs(target, cx)
        .and_then(|refs| refs.rating)
        .unwrap_or_default()
}

/// Press like or dislike: the same one again takes it back.
pub fn rate(target: &Target, wanted: Rating, cx: &mut App) {
    let Some(item) = refs(target, cx).and_then(|refs| refs.rate_ref) else {
        return;
    };
    let rating = match rating(target, cx) == wanted {
        true => Rating::None,
        false => wanted,
    };
    CatalogMarks::rate(item, rating, cx);
}

pub fn can_save(target: &Target, cx: &App) -> bool {
    let caps = caps(cx);
    if opened_playlist(target).is_some_and(|entry| entry.privacy.is_none())
        && caps.playlists != PlaylistCapability::None
    {
        return false;
    }
    caps.save && refs(target, cx).is_some_and(|refs| refs.save_ref.is_some())
}

pub fn is_saved(target: &Target, cx: &App) -> bool {
    refs(target, cx).is_some_and(|refs| refs.saved == Some(true))
}

pub fn toggle_save(target: &Target, cx: &mut App) {
    let Some(item) = refs(target, cx).and_then(|refs| refs.save_ref) else {
        return;
    };
    let saved = !is_saved(target, cx);
    CatalogMarks::save(item, saved, cx);
}

pub fn can_follow(target: &Target, cx: &App) -> bool {
    caps(cx).follow && refs(target, cx).is_some_and(|refs| refs.follow_ref.is_some())
}

pub fn is_followed(target: &Target, cx: &App) -> bool {
    refs(target, cx).is_some_and(|refs| refs.followed == Some(true))
}

pub fn toggle_follow(target: &Target, cx: &mut App) {
    let Some(artist) = refs(target, cx).and_then(|refs| refs.follow_ref) else {
        return;
    };
    let follow = !is_followed(target, cx);
    CatalogMarks::follow(artist, follow, cx);
}

pub fn can_remove_from_history(target: &Target, cx: &App) -> bool {
    caps(cx).remove_from_history
        && refs(target, cx).is_some_and(|refs| refs.history_token.is_some())
}

pub fn remove_from_history(target: &Target, cx: &mut App) {
    if let Some(token) = refs(target, cx).and_then(|refs| refs.history_token) {
        CatalogMarks::remove_from_history(token, cx);
    }
}

/// The source marks a playlist the account owns by giving it a privacy.
pub fn can_edit_details(target: &Target, cx: &App) -> bool {
    caps(cx).playlist_details
        && opened_playlist(target).is_some_and(|entry| entry.privacy.is_some())
}

pub fn edit_details(target: &Target, window: &mut Window, cx: &mut App) {
    let Some(entry) = opened_playlist(target) else {
        return;
    };
    let Some(privacy) = entry.privacy else { return };
    let id = entry.id.to_string();
    components::playlist_details::open(
        entry.description.clone().unwrap_or_default(),
        privacy,
        move |description, privacy, cx| {
            let id = id.clone();
            let edit = PlaylistEdit {
                name: None,
                description: Some(description),
                privacy: Some(privacy),
            };
            actions::run(
                cx,
                move |api| async move { api.edit_playlist(id, edit).await },
            );
        },
        window,
        cx,
    );
}

/// Deleting an owned playlist deletes it at the source; a saved one only leaves the library.
pub fn can_delete_at_source(target: &Target, cx: &App) -> bool {
    caps(cx).playlists != PlaylistCapability::None
        && opened_playlist(target)
            .is_some_and(|entry| entry.privacy.is_some() || entry.actions.save_ref.is_some())
}

pub fn delete_label(target: &Target) -> SharedString {
    match opened_playlist(target).is_some_and(|entry| entry.privacy.is_none()) {
        true => t("catalog_unsave"),
        false => t("delete_playlist"),
    }
}

pub fn delete_at_source(target: &Target, window: &mut Window, cx: &mut App) {
    let Some(entry) = opened_playlist(target).cloned() else {
        return;
    };
    let service = SourcesModel::global(cx)
        .read(cx)
        .active()
        .map(|source| components::format::text(&source.service.name))
        .unwrap_or_default();
    let args = [
        ("name", entry.title.to_string().into()),
        ("service", service.to_string().into()),
    ];
    let owned = entry.privacy.is_some();
    let (title, body) = match owned {
        true => (
            t("playlist_delete_title"),
            t_with("playlist_delete_body", &args),
        ),
        false => (
            t("playlist_unsave_title"),
            t_with("playlist_unsave_body", &args),
        ),
    };
    components::dialogs::confirm(
        components::dialogs::ConfirmKind::Delete,
        title,
        body,
        move |_, cx| match owned {
            true => delete_owned(entry.id.to_string(), cx),
            false => {
                if let Some(item) = entry.actions.save_ref.clone() {
                    CatalogMarks::save(item, false, cx);
                }
            }
        },
        window,
        cx,
    );
}

/// Once the source has dropped it, leave its page rather than show what is gone.
fn delete_owned(id: String, cx: &mut App) {
    let sent = id.clone();
    let call =
        store::Link::global(cx).call(move |api| async move { api.delete_playlist(sent).await });
    cx.spawn(async move |cx| {
        let result = call.await;
        cx.update(|cx| match result {
            Ok(()) => {
                let here = match Navigator::global(cx).read(cx).current() {
                    Route::DiscoverPlaylist { id: at, .. } | Route::Playlist(at) => *at == id,
                    _ => false,
                };
                if here {
                    Navigator::back_or(Route::Library, cx);
                }
            }
            Err(error) => store::Notices::failed(&error, cx),
        });
    })
    .detach();
}
