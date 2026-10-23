//! Settings → Interface: what the palette remembers and what it puts first.

use components::format::t;
use components::settings::{SettingRow, choice, section, toggle_row};
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, AppContext as _, Context, IntoElement, ParentElement as _, Render, SharedString, Styled,
    Window,
};
use store::LayoutModel;
use store::layout::Section as Group;

/// How many recents the list may hold, as the setting offers them.
const KEEP: &[usize] = &[3, 5, 10, 20];

/// Contribute the palette's own section; the app calls this once at startup.
pub fn register(cx: &mut App) {
    settings_interface::register(
        "palette",
        "interface_palette",
        10,
        Some(Group::Palette),
        |_, cx| cx.new(|_| Prefs).into(),
        cx,
    );
}

struct Prefs;

impl Render for Prefs {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let held = store::layout(cx).palette.clone();
        let keep = held.recents;
        v_flex()
            .w_full()
            .child(
                section(t("interface_palette"))
                    .children(
                        toggle_row(
                            "palette-history",
                            t("palette_history"),
                            held.history,
                            |on, cx| {
                                LayoutModel::edit(cx, |layout| {
                                    layout.palette.history = on;
                                    if !on {
                                        layout.palette.recent_commands.clear();
                                        layout.palette.recent_searches.clear();
                                    }
                                });
                            },
                        )
                        .description(t("palette_history_description"))
                        .shown(cx),
                    )
                    .children(
                        SettingRow::new(t("palette_recents"))
                            .description(t("palette_recents_description"))
                            .control(choice(
                                "palette-recents",
                                &keep,
                                KEEP.iter().map(|kept| (*kept, count(*kept))).collect(),
                                |value, cx| {
                                    LayoutModel::edit(cx, |layout| layout.palette.recents = value)
                                },
                            ))
                            .shown(cx),
                    )
                    .children(
                        toggle_row(
                            "palette-current-track-first",
                            t("palette_current_track_first"),
                            held.current_track_first,
                            |on, cx| {
                                LayoutModel::edit(cx, |layout| {
                                    layout.palette.current_track_first = on
                                });
                            },
                        )
                        .description(t("palette_current_track_first_description"))
                        .shown(cx),
                    ),
            )
            .into_any_element()
    }
}

fn count(kept: usize) -> SharedString {
    components::format::t_with("palette_recents_count", &[("count", kept.into())])
}
