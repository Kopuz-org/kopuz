//! A source playlist's description and who can see it, which the source keeps beside its name.

use std::rc::Rc;

use api::PlaylistPrivacy;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::{WindowExt as _, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Window, rems,
};

use crate::dialogs::{DialogSize, FormDialog, FormFooter};
use crate::fields::{FieldFrame, RadioGroup};
use crate::format::t;

type Save = Rc<dyn Fn(String, PlaylistPrivacy, &mut App)>;

const CHOICES: [(PlaylistPrivacy, &str, &str); 3] = [
    (PlaylistPrivacy::Public, "public", "playlist_privacy_public"),
    (
        PlaylistPrivacy::Unlisted,
        "unlisted",
        "playlist_privacy_unlisted",
    ),
    (
        PlaylistPrivacy::Private,
        "private",
        "playlist_privacy_private",
    ),
];

struct PlaylistDetails {
    description: Entity<TextareaState>,
    privacy: PlaylistPrivacy,
    save: Save,
}

impl PlaylistDetails {
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let description = self.description.read(cx).value().to_string();
        (self.save)(description, self.privacy, cx);
        window.close_dialog(cx);
    }
}

impl Render for PlaylistDetails {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chosen = CHOICES
            .iter()
            .find(|(privacy, ..)| *privacy == self.privacy)
            .map_or("private", |(_, value, _)| *value);
        let form = cx.weak_entity();
        let radios = CHOICES.iter().fold(
            RadioGroup::new("playlist-privacy", chosen),
            |group, (_, value, label)| group.option(*value, t(label)),
        );
        v_flex()
            .gap_4()
            .w_full()
            .child(
                FieldFrame::new("playlist-description", t("playlist_description"))
                    .child(Textarea::new(&self.description).h(rems(6.))),
            )
            .child(
                FieldFrame::new("playlist-privacy", t("playlist_privacy")).child(radios.on_change(
                    move |value, _, cx| {
                        let Some((privacy, ..)) = CHOICES.iter().find(|(_, at, _)| *at == value)
                        else {
                            return;
                        };
                        let privacy = *privacy;
                        form.update(cx, |form, cx| {
                            form.privacy = privacy;
                            cx.notify();
                        })
                        .ok();
                    },
                )),
            )
    }
}

/// Ask for a playlist's description and privacy; `save` runs with both once Save is pressed.
pub fn open(
    description: SharedString,
    privacy: PlaylistPrivacy,
    save: impl Fn(String, PlaylistPrivacy, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let form = cx.new(|cx| PlaylistDetails {
        description: cx.new(|cx| TextareaState::new(window, cx).default_value(description)),
        privacy,
        save: Rc::new(save),
    });
    let submit = form.clone();
    FormDialog::new(t("playlist_edit_details"))
        .size(DialogSize::Form)
        .footer(FormFooter::CancelSave)
        .on_submit(move |window, cx| submit.update(cx, |form, cx| form.submit(window, cx)))
        .open(form, window, cx);
}
