//! The dialogs the playlists page opens: add playlist, rename, move to folder, and the delete confirmations.

use std::rc::Rc;

use api::PlaylistCapability;
use components::dialogs::{DialogSize, FormDialog, FormFooter};
use components::format::t;
use components::{ErrorBanner, PickItem, PickList, QuickCreate};
use components::{icons, motion, style};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{WindowExt as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, PathPromptOptions,
    Render, SharedString, Styled as _, Subscription, Window, div, px,
};
use store::{PlaylistsModel, SourcesModel, actions};

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

type Save = Rc<dyn Fn(String, &mut App)>;

/// A one-field prompt: Enter or Save hands the trimmed name to `save`.
struct TextPrompt {
    value: Entity<InputState>,
    save: Save,
}

impl TextPrompt {
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.value.read(cx).value().trim().to_string();
        if value.is_empty() {
            return;
        }
        (self.save)(value, cx);
        window.close_dialog(cx);
    }
}

impl Render for TextPrompt {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w_full().child(Input::new(&self.value))
    }
}

/// Open a rename prompt titled `title`, starting from `value`.
pub(crate) fn open_rename(
    title: SharedString,
    value: impl Into<SharedString>,
    save: impl Fn(String, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let value = value.into();
    let prompt = cx.new(|cx| TextPrompt {
        value: cx.new(|cx| InputState::new(window, cx).default_value(value.clone())),
        save: Rc::new(save),
    });
    let field = prompt.read(cx).value.clone();
    let submit = prompt.clone();
    FormDialog::new(title)
        .size(DialogSize::Prompt)
        .footer(FormFooter::CancelSave)
        .on_submit(move |window, cx| submit.update(cx, |this, cx| this.submit(window, cx)))
        .open(prompt, window, cx);
    field.update(cx, |state, cx| state.focus(window, cx));
}

/// Pick the folder a playlist moves into, or make one on the spot.
struct FolderPicker {
    playlist_id: String,
    name: Entity<InputState>,
    creating: bool,
}

impl FolderPicker {
    fn move_into(&mut self, folder: String, window: &mut Window, cx: &mut Context<Self>) {
        let playlist = self.playlist_id.clone();
        actions::run(cx, move |api| async move {
            api.move_playlist(playlist, Some(folder)).await
        });
        window.close_dialog(cx);
    }

    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        let playlist = self.playlist_id.clone();
        actions::run(cx, move |api| async move {
            let folder = api.create_playlist_folder(name).await?;
            api.move_playlist(playlist, Some(folder)).await
        });
        window.close_dialog(cx);
    }
}

impl Render for FolderPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let folders: Vec<PickItem> = PlaylistsModel::global(cx)
            .read(cx)
            .folders()
            .iter()
            .map(|folder| PickItem::new(folder.id.clone(), folder.name.clone()).icon(icons::FOLDER))
            .collect();
        let (on_pick, on_create) = (cx.weak_entity(), cx.weak_entity());
        v_flex()
            .gap_3()
            .w_full()
            .when(folders.is_empty(), |column| {
                column.child(
                    div()
                        .text_sm()
                        .text_color(style::slate_500(cx))
                        .child(t("no_folders_yet")),
                )
            })
            .child(
                PickList::new("folder-list", folders)
                    .max_height(px(192.))
                    .on_pick(move |id, window, cx| {
                        on_pick
                            .update(cx, |this, cx| this.move_into(id.to_string(), window, cx))
                            .ok();
                    }),
            )
            .when(self.creating, |column| {
                column.child(QuickCreate::new(&self.name).on_create(move |window, cx| {
                    on_create
                        .update(cx, |this, cx| this.create(window, cx))
                        .ok();
                }))
            })
            .child(
                Button::new("new-folder")
                    .outline()
                    .text_sm()
                    .icon(icons::icon(icons::FOLDER))
                    .label(t("new_folder"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.creating = !this.creating;
                        cx.notify();
                    })),
            )
    }
}

/// Open the folder picker for `playlist_id`.
pub(crate) fn open_folder_picker(playlist_id: String, window: &mut Window, cx: &mut App) {
    let picker = cx.new(|cx| FolderPicker {
        playlist_id,
        name: cx.new(|cx| InputState::new(window, cx).placeholder(t("folder_name"))),
        creating: false,
    });
    let create = picker.clone();
    FormDialog::new(t("move_to_folder"))
        .size(DialogSize::Prompt)
        .footer(FormFooter::Cancel)
        .on_submit(move |window, cx| create.update(cx, |this, cx| this.create(window, cx)))
        .open(picker, window, cx);
}

/// Hand the shared rows the two dialogs only this page knows how to open.
pub fn register(cx: &mut App) {
    store::RENAME.register(
        |rename, window, cx| {
            let save = rename.save;
            open_rename(
                rename.title,
                rename.value,
                move |name, cx| save(name, cx),
                window,
                cx,
            )
        },
        cx,
    );
    store::FOLDER_PICKER.register(open_folder_picker, cx);
}
