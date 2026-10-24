//! The internet: said quietly across the top of the page while it is down, and offered back once it returns.

use components::format::{t, t_with};
use components::{Chip, ChipTone, icons, motion, style};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render,
    StatefulInteractiveElement as _, Styled, Subscription, Window, div, px,
};
use store::{NetworkNotice, SourcesModel};

const GLYPH: Pixels = px(12.);
// A scrim like the scan line's, since nothing is wrong with the app itself.
const FILL: f32 = 0.45;

/// The network strip. It renders nothing while there is nothing to say.
pub struct NetworkBanner {
    // The offer someone closed, so it stays gone until the daemon offers a different source.
    dismissed: Option<NetworkNotice>,
    _subscriptions: Vec<Subscription>,
}

impl NetworkBanner {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let sources = SourcesModel::global(cx);
        Self {
            dismissed: None,
            _subscriptions: vec![cx.observe(&sources, |_, _, cx| cx.notify())],
        }
    }

    /// What the strip says now, short of what was closed.
    fn notice(&self, cx: &gpui_kit::App) -> Option<NetworkNotice> {
        let notice = SourcesModel::global(cx).read(cx).network_notice()?;
        (self.dismissed.as_ref() != Some(&notice)).then_some(notice)
    }

    /// Whether the strip has anything to say, for whoever lays it out.
    pub fn is_showing(&self, cx: &gpui_kit::App) -> bool {
        self.notice(cx).is_some()
    }

    fn strip(cx: &gpui_kit::App) -> gpui_kit::Div {
        h_flex()
            .w_full()
            .flex_shrink_0()
            .occlude()
            .items_center()
            .gap_2()
            .px_4()
            .py_1p5()
            .text_xs()
            .bg(style::black(FILL, cx))
            .border_b_1()
            .border_color(style::white(style::BORDER_SOFT, cx))
            .text_color(style::white(style::TEXT_SECONDARY, cx))
    }
}

impl Render for NetworkBanner {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ink = style::white(style::TEXT_SECONDARY, cx);
        match self.notice(cx) {
            None => div().into_any_element(),
            Some(NetworkNotice::Down) => Self::strip(cx)
                .child(
                    icons::icon(icons::WIFI_OFF)
                        .size(GLYPH)
                        .flex_shrink_0()
                        .text_color(ink),
                )
                .child(div().flex_1().min_w_0().child(t("error_network_down")))
                .into_any_element(),
            Some(NetworkNotice::BackOnline { source }) => {
                let sources = SourcesModel::global(cx).read(cx);
                let name = sources
                    .sources()
                    .iter()
                    .find(|row| row.id == source)
                    .map_or_else(|| source.clone(), |row| row.name.clone());
                let offer = NetworkNotice::BackOnline {
                    source: source.clone(),
                };
                let (over, pointer) = motion::hovered("network-dismiss-hover", window, cx);
                let dismiss = motion::amount(
                    "network-dismiss-alpha",
                    if over { 1. } else { 0.5 },
                    motion::FAST,
                    window,
                    cx,
                );
                Self::strip(cx)
                    .child(
                        icons::icon(icons::WIFI)
                            .size(GLYPH)
                            .flex_shrink_0()
                            .text_color(cx.theme().success),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(t_with("back_online_switch_to", &[("source", name.into())])),
                    )
                    .child(
                        Chip::new("network-switch-back")
                            .label(t("switch_to_server"))
                            .tone(ChipTone::Ink(cx.theme().primary))
                            .on_click(move |_, window, cx| {
                                let source = source.clone();
                                SourcesModel::global(cx).update(cx, |sources, cx| {
                                    sources.back_online(source, window, cx)
                                });
                            }),
                    )
                    .child(
                        div()
                            .id("network-dismiss")
                            .flex()
                            .flex_shrink_0()
                            .p_1()
                            .cursor_pointer()
                            .opacity(dismiss)
                            .on_hover(motion::track(pointer))
                            .tooltip(|window, cx| Tooltip::new(t("close")).build(window, cx))
                            .child(icons::icon(icons::CLOSE).size(GLYPH))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.dismissed = Some(offer.clone());
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            }
        }
    }
}
