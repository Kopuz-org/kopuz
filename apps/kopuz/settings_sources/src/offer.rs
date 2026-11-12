//! A source a `kopuz://add-source` link offers: the add form answered as the link says, saved only by the person.

use components::dialogs::{DialogSize, FormDialog};
use components::format::t;
use gpui_kit::{App, AppContext as _, Window};
use store::SourcesModel;
use store::deep_link::SourceOffer;

use crate::add_source::{AddSource, SourceAdded};

pub fn open(offer: SourceOffer, window: &mut Window, cx: &mut App) {
    let form = cx.new(|cx| AddSource::new(window, cx));
    form.update(cx, |form, cx| form.offer(&offer, window, cx));
    window
        .subscribe(&form, cx, move |_, added: &SourceAdded, window, cx| {
            crate::login::expect(
                added.0.clone(),
                offer.username.clone(),
                offer.password.clone(),
                cx,
            );
            SourcesModel::global(cx).update(cx, |sources, cx| {
                sources.switch(added.0.clone(), window, cx)
            });
        })
        .detach();
    let submit = form.clone();
    FormDialog::new(t("add_source"))
        .size(DialogSize::Wide)
        .on_submit(move |window, cx| submit.update(cx, |form, cx| form.save(window, cx)))
        .open(form.clone(), window, cx);
    form.update(cx, |form, cx| form.focus(window, cx));
}
