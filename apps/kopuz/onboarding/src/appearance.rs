//! How Kopuz looks: its theme, chosen from examples drawn in each theme's own colours, and what the cover does to the background.

use components::format::t;
use components::{Segment, Segmented, SettingRow};
use config::SettingsEdit;
use gpui_kit::component::v_flex;
use gpui_kit::{AnyElement, Context, IntoElement, ParentElement as _, SharedString, Styled};
use store::{ConfigModel, SetupModel};

use crate::FirstRun;
use crate::flow::{self, Backdrop};

fn label(look: Backdrop) -> SharedString {
    t(match look {
        Backdrop::Off => "cover_art_background_off",
        Backdrop::Cover => "cover_art_background_cover",
        Backdrop::Blurred => "cover_art_background_blurred",
    })
}

impl FirstRun {
    /// Once per first setup, a background still as installed starts on the blurred cover.
    pub(crate) fn offer_backdrop(&mut self, cx: &mut Context<Self>) {
        if self.backdrop_offered || !SetupModel::global(cx).read(cx).unprompted() {
            return;
        }
        let config = ConfigModel::global(cx);
        if !config.read(cx).is_loaded() {
            return;
        }
        self.backdrop_offered = true;
        let background = config.read(cx).config().background.clone();
        if flow::starts_blurred(&background) {
            for edit in Backdrop::Blurred.edits(&background) {
                ConfigModel::edit(cx, SettingsEdit::Background(edit));
            }
        }
    }

    pub(crate) fn appearance_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let config = ConfigModel::global(cx).read(cx);
        let settings = config.config().clone();
        let pinned = config.is_locked("cover_art_background") || config.is_locked("cover_art_blur");
        let current = Backdrop::of(&settings.background);
        let backdrop = Segmented::new("onboarding-backdrop")
            .segments(Backdrop::ALL.map(|look| Segment::new(label(look)).label(label(look))))
            .selected(Backdrop::ALL.iter().position(|look| *look == current))
            .on_select(move |index, _, cx| {
                let Some(look) = Backdrop::ALL.get(*index).copied() else {
                    return;
                };
                // A managed file pins these; the row says so, and a press must not try to write them.
                if pinned {
                    return;
                }
                let background = ConfigModel::global(cx).read(cx).config().background.clone();
                for edit in look.edits(&background) {
                    ConfigModel::edit(cx, SettingsEdit::Background(edit));
                }
            });
        v_flex()
            .w_full()
            .child(
                crate::ui::card(cx)
                    .child(
                        SettingRow::new(t("appearance"))
                            .description(t("theme_description"))
                            .stacked()
                            .setting(settings_appearance::theme_gallery(
                                "onboarding-theme",
                                &settings,
                            ))
                            .bind("theme", cx),
                    )
                    .child(
                        SettingRow::new(t("cover_art_background"))
                            .description(t("cover_art_background_description"))
                            .control(backdrop)
                            .bind("cover_art_background", cx),
                    ),
            )
            .into_any_element()
    }
}
