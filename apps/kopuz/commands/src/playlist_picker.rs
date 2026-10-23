//! "Add to playlist": pick one of the source's playlists, or type a name to make one.

use components::dialogs::{FormDialog, FormFooter};
use components::format::t;
use components::icons;
use components::{PickItem, PickList, QuickCreate};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::InputState;
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Window,
};
use store::{Notices, PlaylistsModel};

struct PlaylistPicker {
    keys: Vec<String>,
    name: Entity<InputState>,
}

impl PlaylistPicker {
    fn add(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let keys = self.keys.clone();
        PlaylistsModel::global(cx).update(cx, |playlists, cx| playlists.add_tracks(id, keys, cx));
        window.close_dialog(cx);
    }

    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().to_string();
        if name.trim().is_empty() {
            Notices::error(t("playlist_name_required"), cx);
            return;
        }
        let keys = self.keys.clone();
        PlaylistsModel::global(cx).update(cx, |playlists, cx| playlists.create(name, keys, cx));
        window.close_dialog(cx);
    }
}

impl Render for PlaylistPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items: Vec<PickItem> = PlaylistsModel::global(cx)
            .read(cx)
            .playlists()
            .iter()
            .map(|playlist| PickItem::new(playlist.id.clone(), playlist.name.clone()))
            .collect();
        let (on_create, on_pick) = (cx.weak_entity(), cx.weak_entity());
        v_flex()
            .gap_4()
            .w_full()
            .child(
                QuickCreate::new(&self.name)
                    .icon(icons::PLUS)
                    .on_create(move |window, cx| {
                        on_create
                            .update(cx, |this, cx| this.create(window, cx))
                            .ok();
                    }),
            )
            .child(PickList::new("playlist-list", items).raised(true).on_pick(
                move |id, window, cx| {
                    on_pick
                        .update(cx, |this, cx| this.add(id.to_string(), window, cx))
                        .ok();
                },
            ))
    }
}

/// Open the picker for `keys`.
pub fn open(keys: Vec<String>, window: &mut Window, cx: &mut App) {
    let picker = cx.new(|cx| PlaylistPicker {
        keys,
        name: cx.new(|cx| InputState::new(window, cx).placeholder(t("new_playlist_name"))),
    });
    let create = picker.clone();
    FormDialog::new(t("add_to_playlist"))
        .closable(true)
        .footer(FormFooter::None)
        .on_submit(move |window, cx| create.update(cx, |this, cx| this.create(window, cx)))
        .open(picker, window, cx);
}
