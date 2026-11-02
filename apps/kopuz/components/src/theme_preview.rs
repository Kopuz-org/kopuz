//! A small player drawn in a theme's own colours, cross-fading as the colours change.

use std::collections::HashMap;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    App, ElementId, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, SharedString, Styled, Window, div, relative,
};

use crate::color_field::parse_color;
use crate::format::t;
use crate::{motion, style};

/// What the preview shows of a theme, given its palette tokens by name.
#[derive(IntoElement)]
pub struct ThemePreview {
    vars: HashMap<String, String>,
}

impl ThemePreview {
    pub fn new(vars: HashMap<String, String>) -> Self {
        Self { vars }
    }

    /// Loading a saved theme swaps thirteen colours at once; the swatches cross-fade rather than cut.
    fn swatch(&self, key: &'static str, fallback: Hsla, window: &mut Window, cx: &mut App) -> Hsla {
        let target = self
            .vars
            .get(key)
            .and_then(|value| parse_color(value))
            .unwrap_or(fallback);
        motion::color_over(
            (ElementId::from("swatch"), key),
            target,
            motion::MEDIUM,
            window,
            cx,
        )
    }
}

impl RenderOnce for ThemePreview {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let (background, muted, border, foreground, muted_foreground, primary, danger) = (
            theme.background,
            theme.muted,
            theme.border,
            theme.foreground,
            theme.muted_foreground,
            theme.primary,
            theme.danger,
        );
        let bg = self.swatch("bg", background, window, cx);
        let raised = self.swatch("raised", muted, window, cx);
        let surface = self.swatch("surface", border, window, cx);
        let text = self.swatch("text", foreground, window, cx);
        let muted = self.swatch("text-muted", muted_foreground, window, cx);
        let progress = self.swatch("progress", primary, window, cx);
        let accent = self.swatch("accent", primary, window, cx);
        let highlight = self.swatch("highlight", primary, window, cx);
        let danger = self.swatch("danger", danger, window, cx);
        let chip = |key: &'static str, label: SharedString, color: Hsla| {
            div()
                .px_2()
                .py_0p5()
                .id(key)
                .rounded_full()
                .bg(raised)
                .text_xs()
                .text_color(color)
                .child(label)
        };
        v_flex()
            .w_full()
            .gap_2()
            .p_4()
            .rounded(style::radius_lg())
            .bg(bg)
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(text)
                            .child(t("track_title")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(SharedString::from("3:42")),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h_1()
                    .rounded_full()
                    .bg(surface)
                    .child(div().w(relative(0.66)).h_1().rounded_full().bg(progress)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .mt_1()
                    .child(chip("preview-accent", t("accent"), accent))
                    .child(chip("preview-highlight", t("highlight"), highlight))
                    .child(chip("preview-danger", t("danger"), danger)),
            )
    }
}
