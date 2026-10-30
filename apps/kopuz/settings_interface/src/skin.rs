//! Which registered skin draws the app; the palette beside it stays a separate question.

use components::format::t;
use components::settings::{SettingRow, choice, section};
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, AppContext as _, Context, IntoElement, ParentElement as _, Render, SharedString, Styled,
    Window,
};
use store::LayoutModel;
use store::layout::Section as Group;

pub fn register(cx: &mut App) {
    crate::register(
        "skin",
        "interface_skin",
        1,
        Some(Group::Skin),
        |_, cx| cx.new(|_| Skin).into(),
        cx,
    );
}

struct Skin;

impl Render for Skin {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = store::layout(cx).skin.clone();
        let options: Vec<(String, SharedString)> = components::skin::registered()
            .into_iter()
            .map(|(id, title)| (id.to_string(), t(title)))
            .collect();
        v_flex()
            .w_full()
            .child(
                section(t("interface_skin")).children(
                    SettingRow::new(t("interface_skin"))
                        .description(t("interface_skin_description"))
                        .control(choice("interface-skin", &current, options, |value, cx| {
                            LayoutModel::edit(cx, |layout| layout.skin = value)
                        }))
                        .shown(cx),
                ),
            )
            .into_any_element()
    }
}
