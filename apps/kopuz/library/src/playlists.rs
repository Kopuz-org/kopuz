//! The playlists facet's own parts: folders, the add-playlist dialog, and what the source keeps in sync.

use std::collections::HashSet;
use std::rc::Rc;

use api::{PlaylistCapability, PlaylistFolderInfo, PlaylistInfo};
use components::dialogs::FormDialog;
use components::dialogs::FormFooter;
use components::format::t;
use components::{ErrorBanner, IconButton, icons, motion};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{WindowExt as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, Global, IntoElement, ParentElement as _,
    PathPromptOptions, Render, SharedString, Styled as _, Subscription, Window,
};
use store::{DownloadsModel, JobsModel, PlaylistsModel, Route, SourcesModel, Target, actions};

use crate::{Library, Tile};

impl Library {
    /// The playlists or the source changed: take every playlist's keys afresh.
    pub(crate) fn rekey_offline(&mut self, cx: &App) {
        self.playlist_offline.clear();
        if !SourcesModel::global(cx).read(cx).capabilities().downloads {
            return;
        }
        let downloads = DownloadsModel::global(cx).read(cx);
        for playlist in PlaylistsModel::global(cx).read(cx).playlists() {
            let keys: Rc<[String]> = playlist.track_keys.clone().into();
            let offline = downloads.offline(&keys);
            self.playlist_offline
                .insert(playlist.id.clone(), (keys, offline));
        }
    }

    /// A download moved: the keys stand, only what the downloads say of them is asked again.
    pub(crate) fn restate_offline(&mut self, cx: &App) {
        let downloads = DownloadsModel::global(cx).read(cx);
        for (keys, offline) in self.playlist_offline.values_mut() {
            *offline = downloads.offline(keys);
        }
    }

    /// Folders first, then what sits outside them; a search reaches into the folders too.
    pub(crate) fn playlist_tiles(&self, cx: &Context<Self>) -> Vec<Tile> {
        let query = self.filters().search.trim().to_lowercase();
        let matches = |name: &str| query.is_empty() || name.to_lowercase().contains(&query);
        let playing = self.playing.track();
        let capabilities = SourcesModel::global(cx).read(cx).capabilities();
        let model = PlaylistsModel::global(cx).read(cx);
        let playlist = |playlist: &PlaylistInfo| {
            let mut tile = crate::grid::playlist_tile(playlist, playing);
            tile.offline = self.playlist_offline.get(&playlist.id).cloned();
            // A server's playlists all come from it, which the Dioxus flat layout said on every card.
            if !capabilities.folders {
                tile.subtitle =
                    SharedString::from(format!("{} • {}", t("server_playlist"), tile.subtitle));
            }
            tile
        };
        if !capabilities.folders {
            return model
                .playlists()
                .iter()
                .filter(|held| matches(&held.name))
                .map(playlist)
                .collect();
        }
        let filed: HashSet<&str> = model
            .folders()
            .iter()
            .flat_map(|folder| folder.playlist_ids.iter().map(String::as_str))
            .collect();
        let folders = model
            .folders()
            .iter()
            .filter(|folder| matches(&folder.name))
            .map(|folder| folder_tile(folder, model.playlists()));
        let loose = model
            .playlists()
            .iter()
            .filter(|held| matches(&held.name))
            .filter(|held| !query.is_empty() || !filed.contains(held.id.as_str()))
            .map(playlist);
        folders.chain(loose).collect()
    }

    /// New folder and Add playlist, the two buttons the Dioxus playlists page carried.
    pub(crate) fn playlist_buttons(&self, cx: &App) -> Vec<IconButton> {
        let mut buttons = Vec::new();
        if SourcesModel::global(cx).read(cx).capabilities().folders {
            buttons.push(
                IconButton::new("library-new-folder", icons::FOLDER)
                    .tooltip(t("new_folder"))
                    .on_click(|_, _, cx| new_folder(cx)),
            );
        }
        buttons.push(
            IconButton::new("library-add-playlist", icons::PLUS)
                .tooltip(t("add_playlist"))
                .on_click(|_, window, cx| open_add_playlist(window, cx)),
        );
        buttons
    }
}

/// A folder named the way the Dioxus page named one, renamed from its card's menu.
pub(crate) fn new_folder(cx: &mut App) {
    let name = t("new_folder").to_string();
    actions::run(cx, move |api| async move {
        api.create_playlist_folder(name).await
    });
}

/// The keys of every playlist a folder holds, in the folder's order.
pub(crate) fn folder_keys(folder: &PlaylistFolderInfo, playlists: &[PlaylistInfo]) -> Vec<String> {
    folder
        .playlist_ids
        .iter()
        .filter_map(|id| playlists.iter().find(|playlist| &playlist.id == id))
        .flat_map(|playlist| playlist.track_keys.iter().cloned())
        .collect()
}

fn folder_tile(folder: &PlaylistFolderInfo, playlists: &[PlaylistInfo]) -> Tile {
    let cover = folder
        .playlist_ids
        .first()
        .and_then(|id| playlists.iter().find(|playlist| &playlist.id == id))
        .and_then(|playlist| playlist.artwork.clone());
    let count = folder.playlist_ids.len();
    Tile {
        id: SharedString::from(folder.id.clone()),
        title: SharedString::from(folder.name.clone()),
        // TODO(i18n): the Dioxus card spells this count out in English too.
        subtitle: SharedString::from(format!("{count} playlists")),
        artwork: cover,
        sort_key: folder.name.to_lowercase(),
        sort_rank: count as u64,
        playing: false,
        target: Target::Folder {
            id: folder.id.clone().into(),
            name: folder.name.clone().into(),
        },
        route: Route::PlaylistFolder(folder.id.clone().into()),
        pictured: None,
        placeholder: icons::FOLDER,
        folder: true,
        offline: None,
    }
}

/// Jobs a page starts by itself, at most once per source for the life of the app.
#[derive(Default)]
struct Started(Vec<(api::JobKind, String)>);

impl Global for Started {}

/// Start `kind` for `source` unless this app already did, so revisiting the page does not sync again.
pub(crate) fn start_once(kind: api::JobKind, source: &str, cx: &mut App) {
    let started = &mut cx.default_global::<Started>().0;
    if started
        .iter()
        .any(|(held, id)| *held == kind && id == source)
    {
        return;
    }
    started.push((kind, source.to_string()));
    JobsModel::global(cx).update(cx, |jobs, cx| jobs.start(kind, cx));
}

/// Ask for a name and create a playlist, or import a directory as one.
struct AddPlaylist {
    name: Entity<InputState>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl AddPlaylist {
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().to_string();
        if name.trim().is_empty() {
            return;
        }
        // A source that cannot mutate playlists gets the friendly line, not a raw refusal.
        if SourcesModel::global(cx).read(cx).capabilities().playlists == PlaylistCapability::None {
            self.error = Some(t("error_server_not_configured"));
            cx.notify();
            return;
        }
        PlaylistsModel::global(cx)
            .update(cx, |playlists, cx| playlists.create(name, Vec::new(), cx));
        window.close_dialog(cx);
    }

    fn import_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        window.close_dialog(cx);
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            let raw = path.to_string_lossy().to_string();
            let prefix = if raw.ends_with(std::path::MAIN_SEPARATOR) {
                raw
            } else {
                format!("{raw}{}", std::path::MAIN_SEPARATOR)
            };
            cx.update(|cx| {
                actions::run(cx, move |api| async move {
                    let page = api.folder_tracks(prefix, store::all()).await?;
                    let keys = page.items.into_iter().map(|track| track.key).collect();
                    api.create_playlist(name, keys).await
                });
            });
        })
        .detach();
    }
}

impl Render for AddPlaylist {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let folders = SourcesModel::global(cx).read(cx).capabilities().folders;
        // Asked for every frame, so the channel outlives the banner and the next error fades in.
        let reveal = motion::amount(
            "add-playlist-error",
            if self.error.is_some() { 1. } else { 0. },
            motion::FAST,
            window,
            cx,
        );
        v_flex()
            .gap_4()
            .w_full()
            .when_some(self.error.clone(), |column, error| {
                column.child(ErrorBanner::new(error).reveal(reveal))
            })
            .child(Input::new(&self.name))
            .when(folders, |column| {
                column.child(
                    Button::new("import-folder")
                        .outline()
                        .text_sm()
                        .icon(icons::icon(icons::FOLDER))
                        .label(t("add_folder"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.import_folder(window, cx)),
                        ),
                )
            })
    }
}

/// Open the add-playlist dialog.
pub(crate) fn open_add_playlist(window: &mut Window, cx: &mut App) {
    let dialog = cx.new(|cx| {
        let name =
            cx.new(|cx| InputState::new(window, cx).placeholder(t("playlist_name_placeholder")));
        let subscription = cx.subscribe(&name, |this: &mut AddPlaylist, _, event, cx| {
            if let InputEvent::Change = event {
                this.error = None;
                cx.notify();
            }
        });
        AddPlaylist {
            name,
            error: None,
            _subscriptions: vec![subscription],
        }
    });
    let field = dialog.read(cx).name.clone();
    let save = dialog.clone();
    FormDialog::new(t("add_playlist"))
        .footer(FormFooter::CancelSave)
        .on_submit(move |window, cx| save.update(cx, |this, cx| this.save(window, cx)))
        .open(dialog, window, cx);
    field.update(cx, |state, cx| state.focus(window, cx));
}
