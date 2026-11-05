//! The dialogs a playlist or folder menu opens: rename, and move to folder.

use std::rc::Rc;

use components::dialogs::{DialogSize, FormDialog, FormFooter};
use components::format::t;
use components::{PickItem, PickList, QuickCreate};
use components::{icons, style};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{WindowExt as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Window, div, px,
};
use store::{PlaylistsModel, actions};

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
