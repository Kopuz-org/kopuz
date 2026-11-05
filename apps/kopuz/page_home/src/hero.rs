//! The hero: the album the last session was in, across the page, with play, a heart and its menu.

use std::ops::RangeInclusive;

use api::QueueContext;
use components::format::{t, t_with};
use components::menus::WithContextMenu as _;
use components::{
    Cover, IconButton, IconTone, MenuButton, MenuVariant, Picture, Scrim, icons, menus, style,
};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, DragMoveEvent, ElementId, FontWeight,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, ParentElement as _, Pixels,
    Render, SharedString, StatefulInteractiveElement as _, Styled, Window, div, linear_color_stop,
    linear_gradient, px, relative, rems,
};
use store::{FavoritesModel, LayoutModel, actions};

use crate::data::{Play, Tile};

/// How tall the hero may be dragged, as the Dioxus edit mode clamps it.
const HEIGHT: RangeInclusive<u32> = 140..=800;

/// `w-11 h-11`: the heart and the dots beside Start listening.
const SIDE_BUTTON: Pixels = px(44.);
const SIDE_GLYPH: Pixels = px(18.);

/// `h-3`, the grip along the hero's foot in edit mode.
const GRIP: Pixels = px(12.);

/// What the hero needs beyond its tile: how tall it is, the album's tracks for the heart, and whether it is being arranged.
pub(crate) struct Hero<'a> {
    pub tile: Option<&'a Tile>,
    pub height: u32,
    pub keys: &'a [String],
    pub editing: bool,
}

pub(crate) fn hero(hero: Hero<'_>, window: &mut Window, cx: &mut App) -> AnyElement {
    let height = hero.height.clamp(*HEIGHT.start(), *HEIGHT.end());
    let radius = style::to_px(style::radius_xl(), window);
    let frame = div()
        .id("home-hero")
        .relative()
        .w_full()
        .flex_shrink_0()
        .h(px(height as f32))
        .rounded(radius)
        .overflow_hidden();
    let Some(tile) = hero.tile else {
        // With nothing played yet the hero is a blank block, there only to be sized.
        return frame
            .bg(style::white(style::RAISED, cx))
            .children(hero.editing.then(|| grip(height, window, cx)))
            .into_any_element();
    };
    let menu = menus::menu_for(tile.target.clone());
    let picture = Picture::new(tile.artwork.as_ref(), tile.pictured.clone());
    frame
        .child(
            Cover::new(tile.artwork.as_ref())
                .fill()
                .hq(true)
                .rounded(radius)
                .spot(picture.as_ref().map(Picture::spot))
                .scrim(Scrim {
                    foot: 0.5,
                    top: 0.,
                    reach: 1.,
                }),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .rounded(radius)
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(style::black(0.95, cx), 0.),
                    linear_color_stop(style::black(0.2, cx), 1.),
                )),
        )
        .child(body(tile, hero.keys, menu.clone(), cx))
        .children(hero.editing.then(|| grip(height, window, cx)))
        .with_context_menu(menus::pictured(Some(menu), picture))
}

fn body(tile: &Tile, keys: &[String], menu: components::Menu, cx: &mut App) -> AnyElement {
    let favorites = FavoritesModel::global(cx).read(cx);
    let favorite = !keys.is_empty() && keys.iter().all(|key| favorites.is_favorite(key));
    let heart: Vec<String> = keys
        .iter()
        .filter(|key| favorites.is_favorite(key) == favorite)
        .cloned()
        .collect();
    let album = match &tile.play {
        Play::Album(id) => Some(id.clone()),
        _ => None,
    };
    let muted = style::white(style::TEXT_MUTED, cx);
    v_flex()
        .absolute()
        .inset_0()
        .justify_center()
        .p_12()
        .child(
            div()
                .mb_3()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(cx.theme().primary)
                .child(t("featured_album")),
        )
        .child(
            div()
                .flex_shrink_0()
                .mb_4()
                .max_w_full()
                .truncate()
                .text_size(style::text_5xl())
                .line_height(relative(1.25))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().foreground)
                .child(tile.title.clone()),
        )
        .when(!tile.subtitle.is_empty(), |body| {
            body.child(
                div()
                    .flex_shrink_0()
                    .mb_8()
                    .max_w(rems(28.))
                    .truncate()
                    .text_lg()
                    .line_height(rems(1.75))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(muted)
                    .child(t_with(
                        "by_artist_full",
                        &[("artist", tile.subtitle.to_string().into())],
                    )),
            )
        })
        .child(
            h_flex()
                .gap_4()
                .items_center()
                .child(
                    style::inverted(Button::new("home-hero-play"), cx)
                        .h_11()
                        .px_8()
                        .rounded_full()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .icon(icons::icon(icons::PLAY))
                        .label(t("start_listening"))
                        .on_click(move |_, _, cx| {
                            if let Some(id) = album.clone() {
                                actions::play_context(QueueContext::Album { id }, Some(0), cx);
                            }
                        }),
                )
                .child(
                    IconButton::new("home-hero-favorite", icons::HEART)
                        .size(SIDE_BUTTON)
                        .glyph(SIDE_GLYPH)
                        .tone(IconTone::Soft)
                        .lit(favorite)
                        .accent(cx.theme().danger)
                        .disabled(keys.is_empty())
                        .tooltip(t(if favorite {
                            "remove_from_favorites"
                        } else {
                            "add_to_favorites"
                        }))
                        // Only the tracks not already where the heart is going are asked about.
                        .on_click(move |_, _, cx| {
                            actions::set_favorites(heart.clone(), !favorite, cx)
                        }),
                )
                .child(
                    MenuButton::new("home-hero-menu", menu).variant(MenuVariant::Icon {
                        size: SIDE_BUTTON,
                        glyph: SIDE_GLYPH,
                        tone: IconTone::Soft,
                    }),
                ),
        )
        .into_any_element()
}

/// The payload of a hero resize; the handle is the only thing listening for it.
#[derive(Clone)]
struct HeroDrag;

impl Render for HeroDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::Empty
    }
}

/// The strip along the hero's foot that drags its height, saved as `hero_height`.
fn grip(height: u32, window: &mut Window, cx: &mut App) -> AnyElement {
    let start = window.use_keyed_state(
        ElementId::from(SharedString::new_static("home-hero-grip")),
        cx,
        |_, _| None::<(Pixels, u32)>,
    );
    let pressed = start.clone();
    let lit = cx.theme().primary.opacity(0.4);
    div()
        .id("home-hero-grip")
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(GRIP)
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .cursor_row_resize()
        .bg(style::black(0.4, cx))
        .hover(move |grip| grip.bg(lit))
        .child(div().w_10().h_1().rounded_full().bg(style::white(0.6, cx)))
        .on_drag(HeroDrag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
            pressed.update(cx, |held, _| *held = Some((event.position.y, height)));
            cx.stop_propagation();
        })
        .on_drag_move(move |event: &DragMoveEvent<HeroDrag>, _, cx| {
            let Some((from, base)) = *start.read(cx) else {
                return;
            };
            let moved = (event.event.position.y - from).as_f32().round() as i64;
            let height = (i64::from(base) + moved)
                .clamp(i64::from(*HEIGHT.start()), i64::from(*HEIGHT.end()))
                as u32;
            LayoutModel::edit(cx, |layout| layout.ui.hero_height = height);
        })
        .into_any_element()
}
