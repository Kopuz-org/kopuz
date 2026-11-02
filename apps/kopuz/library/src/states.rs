//! What the library shows instead of rows, and the one button that gets you out of it.

use api::ApiError;
use components::format::t;
use components::{EmptyState, icons};
use gpui_kit::{App, ClickEvent, SharedString, Window};
use store::{Navigator, Route, SourcesModel};

use crate::facet::Facet;

/// The shared icon set has no sign-in glyph, which is the one an anonymous source wants.
pub const SIGN_IN: &str = "icons/log-in.svg";

/// A read that failed says so, and offers the one thing worth trying: asking again.
pub fn failed(
    error: &ApiError,
    retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> EmptyState {
    EmptyState::failed(t("library_read_failed"), retry).detail(store::human(error))
}

/// What an empty facet says depends on, about the active source.
#[derive(Clone, Default)]
pub struct Active {
    capabilities: api::SourceCapabilities,
    source: Option<api::SourceInfo>,
    offline: bool,
}

impl Active {
    pub fn read(cx: &App) -> Self {
        let sources = SourcesModel::global(cx).read(cx);
        Self {
            capabilities: sources.capabilities(),
            source: sources.active().cloned(),
            offline: sources.offline_reads(),
        }
    }
}

/// A facet with nothing in it: the button is whatever would put something there for this source.
pub fn empty_library(facet: Facet, active: &Active) -> EmptyState {
    let capabilities = &active.capabilities;
    let source = active.source.clone();
    let anonymous = source.as_ref().is_some_and(|source| source.anonymous);
    let signed_out = source
        .as_ref()
        .is_some_and(|source| !source.authenticated && !source.anonymous);

    // Syncing or signing in cannot fill a library offline mode keeps to its copies here.
    if active.offline {
        return EmptyState::offline();
    }
    if signed_out || anonymous {
        let Some(source) = source else {
            return EmptyState::new(t("no_tracks_found")).icon(icons::LIBRARY);
        };
        return EmptyState::new(t("library_empty_signed_out"))
            .icon(SIGN_IN)
            .detail(SharedString::from(source.name.clone()))
            .action(t("library_sign_in"), move |_, window, cx| {
                store::sign_in_again(&source, window, cx)
            });
    }
    // Playlists are made, not scanned or synced, so no folder or sync would fill this facet.
    if facet == Facet::Playlists {
        return EmptyState::new(t("no_playlists_yet")).icon(icons::LIST);
    }
    let has_folders = source
        .as_ref()
        .is_some_and(|source| !settings_sources::folders::of(source).is_empty());
    if capabilities.scan_folders && has_folders {
        return EmptyState::new(t("no_tracks_in_library"))
            .icon(icons::FOLDER)
            .action(t("rescan_library"), |_, _, cx| {
                store::JobsModel::global(cx)
                    .update(cx, |jobs, cx| jobs.start(api::JobKind::Scan, cx));
            });
    }
    if capabilities.scan_folders {
        return EmptyState::new(t("library_empty_no_folder"))
            .icon(icons::FOLDER)
            .detail(t("library_empty_no_folder_detail"))
            .action(t("library_add_folder"), |_, _, cx| {
                Navigator::navigate(
                    Route::Settings {
                        section: Some("library".into()),
                    },
                    cx,
                )
            });
    }
    if capabilities.sync {
        return EmptyState::new(t("library_empty_not_synced"))
            .icon(icons::REFRESH)
            .detail(t("library_empty_not_synced_detail"))
            .action(t("refresh_music_library"), |_, _, cx| {
                store::JobsModel::global(cx)
                    .update(cx, |jobs, cx| jobs.start(api::JobKind::LibrarySync, cx));
            });
    }
    EmptyState::new(t("no_tracks_found")).icon(icons::LIBRARY)
}

/// The Favorites page before anything has been liked.
pub fn no_favorites() -> EmptyState {
    EmptyState::new(t("no_favorites")).icon(icons::HEART)
}

/// Everything is there, the chips simply exclude it: the button takes the chips off.
pub fn no_matches(clear: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> EmptyState {
    EmptyState::new(t("library_no_matches"))
        .icon(icons::SEARCH)
        .detail(t("library_no_matches_detail"))
        .action(t("clear"), clear)
}
