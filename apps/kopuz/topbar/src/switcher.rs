use std::rc::Rc;

use api::SourceInfo;
use components::format::{t, text};
use components::source_mark::{self, SourceMark};
use components::{StatusDot, icons, style};
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, App, Context, DismissEvent, Div, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, RenderOnce, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, WeakEntity, Window, div, px,
};
use store::{Navigator, Route, SourceStatus, SourcesModel};

const GLYPH: Pixels = px(16.);
const CHEVRON: Pixels = px(10.);
const CHECK: Pixels = px(11.);
const POPOVER_WIDTH: Pixels = px(218.);
const ROWS_HEIGHT: Pixels = px(320.);
const RAIL_TILE: Pixels = px(40.);
const SUBLINE_GAP: Pixels = px(2.);

// The switcher's own dimmed tones, which are off the named alpha scale.
const HEAD_COUNT: f32 = 0.38;
const ROW_NAME: f32 = 0.85;
/// A source the internet being down leaves out of reach still answers a click, with the daemon's refusal.
const CUT_OFF: f32 = 0.45;

const MANAGE_ICON: &str = "icons/sliders-horizontal.svg";

/// The active source, and a popover to pick another.
#[derive(IntoElement)]
pub struct SourceSwitcher {
    sources: Vec<SourceInfo>,
    status: SourceStatus,
    collapsed: bool,
    bare: bool,
}

impl SourceSwitcher {
    pub fn new(sources: Vec<SourceInfo>, status: SourceStatus) -> Self {
        Self {
            sources,
            status,
            collapsed: false,
            bare: false,
        }
    }

    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    /// Drop the rail's framing: in a bar the padding and the rule below it have nowhere to go.
    pub fn bare(mut self, bare: bool) -> Self {
        self.bare = bare;
        self
    }
}

impl RenderOnce for SourceSwitcher {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let active = self.sources.iter().find(|source| source.active);
        let label: SharedString = active
            .map(|source| SharedString::from(source.name.clone()))
            .unwrap_or_default();
        let mark = active.map_or(SourceMark::none(), |source| {
            SourceMark::new(&source.service)
        });
        let collapsed = self.collapsed;
        let bare = self.bare;
        let count = self.sources.len();
        let sources = self.sources;
        let status = self.status;

        div()
            .when(!bare, |frame| {
                frame
                    .border_b_1()
                    .border_color(style::white(style::BORDER_SOFT, cx))
            })
            .map(|frame| match (bare, collapsed) {
                (true, _) => frame,
                (false, true) => frame.flex().justify_center().py_3(),
                (false, false) => frame.px_3().pt_3().pb_2(),
            })
            .child(
                trigger(label, mark, status, collapsed, cx)
                    .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, cx| {
                        build(&sources, count, menu, cx)
                    }),
            )
    }
}

/// Every source in a list that scrolls on its own, then offline mode and the way to manage them, never scrolled away.
fn build(
    sources: &[SourceInfo],
    count: usize,
    menu: PopupMenu,
    cx: &Context<PopupMenu>,
) -> PopupMenu {
    let owner = cx.entity().downgrade();
    let sources: Rc<[SourceInfo]> = sources.into();
    let menu = menu
        .max_w(POPOVER_WIDTH)
        .min_w(POPOVER_WIDTH)
        .label(SharedString::from(format!("{}  {count}", t("sources"))))
        .item(
            PopupMenuItem::element(move |_, cx| {
                v_flex()
                    .id("source-rows")
                    .w_full()
                    .max_h(ROWS_HEIGHT)
                    .gap_0p5()
                    .overflow_y_scroll()
                    .children(sources.iter().map(|source| row(source, owner.clone(), cx)))
            })
            .disabled(true),
        );
    crate::offline::item(menu.separator(), cx).item(
        PopupMenuItem::new(t("manage_sources"))
            .icon(Icon::default().path(MANAGE_ICON).size(GLYPH))
            .on_click(|_, _, cx| {
                Navigator::navigate(
                    Route::Settings {
                        section: Some("library".into()),
                    },
                    cx,
                );
            }),
    )
}

/// One source; it answers its own click, since the list it sits in is a single inert menu item.
fn row(source: &SourceInfo, owner: WeakEntity<PopupMenu>, cx: &App) -> Stateful<Div> {
    let accent = source_mark::accent(Some(&source.service), cx);
    let active = source.active;
    let id = source.id.clone();
    let name = SharedString::from(source.name.clone());
    let subline = subline(source);
    let mark = SourceMark::new(&source.service).size(GLYPH);
    let ink = match active {
        true => cx.theme().foreground,
        false => style::white(ROW_NAME, cx),
    };
    let muted = style::white(style::TEXT_SUBTLE, cx);
    let cut_off = SourcesModel::global(cx).read(cx).cut_off(source);
    let hover = style::white(style::HOVER, cx);
    h_flex()
        .id(SharedString::from(format!("source-row-{}", source.id)))
        .w_full()
        .px_2()
        .py_1()
        .gap_2p5()
        .rounded(style::radius(cx))
        .cursor_pointer()
        .hover(move |row| row.bg(hover))
        .when(cut_off, |row| {
            row.opacity(CUT_OFF)
                .tooltip(|window, cx| Tooltip::new(t("error_network_down")).build(window, cx))
        })
        .child(mark)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(SUBLINE_GAP)
                .child(
                    div()
                        .truncate()
                        .text_sm()
                        .map(|line| match active {
                            true => line.font_semibold(),
                            false => line.font_medium(),
                        })
                        .text_color(ink)
                        .child(name),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(style::text_11())
                        .text_color(muted)
                        .child(subline),
                ),
        )
        .when(active, |row| {
            row.child(
                Icon::default()
                    .path(icons::CHECK)
                    .size(CHECK)
                    .text_color(accent),
            )
        })
        .when(cut_off, |row| {
            row.child(icons::icon(icons::WIFI_OFF).size(CHECK).text_color(muted))
        })
        .on_click(move |_, window, cx| {
            SourcesModel::global(cx)
                .update(cx, |sources, cx| sources.switch(id.clone(), window, cx));
            owner.update(cx, |_, cx| cx.emit(DismissEvent)).ok();
        })
}

/// The trigger tile: glyph, name, status dot and chevron, on the Button the menu hangs off.
fn trigger(
    label: SharedString,
    mark: SourceMark,
    status: SourceStatus,
    collapsed: bool,
    cx: &App,
) -> Button {
    let tooltip = label.clone();
    let tile = h_flex()
        .w_full()
        .gap_2p5()
        .child(mark)
        .when(!collapsed, |trigger| {
            trigger
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_medium()
                        .text_color(cx.theme().foreground)
                        .child(label),
                )
                .child(StatusDot::new(status))
                .child(
                    Icon::default()
                        .path(icons::CHEVRON_DOWN)
                        .size(CHEVRON)
                        .text_color(style::white(HEAD_COUNT, cx)),
                )
        });
    let button = style::dress(
        Button::new("source-trigger").tab_index(0),
        gpui_kit::transparent_black(),
        style::white(style::RAISED, cx),
        cx.theme().foreground,
        style::white(style::BORDER, cx),
        cx,
    )
    .rounded(px(8.))
    .map(|button| match collapsed {
        true => button.size(RAIL_TILE).tooltip(tooltip),
        false => button.w_full().h_auto().px_2p5().py_2(),
    });
    button.child(tile)
}

fn subline(source: &SourceInfo) -> SharedString {
    text(&source.service.name).to_uppercase().into()
}
