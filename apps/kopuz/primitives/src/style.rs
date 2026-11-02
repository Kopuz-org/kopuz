//! Kopuz's Tailwind look in GPUI terms; `docs/STYLING.md` holds the full class-to-call table.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::ButtonCustomVariant;
use gpui_kit::{App, Hsla, Pixels, Rems, px, rems};

use crate::skin;

/// `bg-white/5`: the raised surface a row, tile or thumb sits on.
pub const RAISED: f32 = 0.05;
/// `hover:bg-white/10`, and the resting surface of a pressed-looking control.
pub const HOVER: f32 = 0.10;
/// `hover:bg-white/20`, and `bg-white/20` on an active control.
pub const ACTIVE: f32 = 0.20;
/// `border-white/10`.
pub const BORDER: f32 = 0.10;
/// `border-white/5`, the quieter divider.
pub const BORDER_SOFT: f32 = 0.05;

/// `text-white/90`, a heading that is not quite full white.
pub const TEXT_STRONG: f32 = 0.90;
/// `text-white/60`, secondary text.
pub const TEXT_SECONDARY: f32 = 0.60;
/// `text-white/50`, a form's help line and a section heading.
pub const TEXT_MUTED: f32 = 0.50;
/// `text-white/45`, a row's subtitle.
pub const TEXT_SUBTLE: f32 = 0.45;
/// `text-white/30`, a row's album and duration columns.
pub const TEXT_DIM: f32 = 0.30;
/// `text-white/20`, a placeholder glyph.
pub const TEXT_FAINT: f32 = 0.20;
/// The `inset 0 0 0 9999px rgba(255,255,255,0.07)` a selected row is washed with.
pub const SELECTED: f32 = 0.07;
/// `color-mix(… indigo-500 12%, transparent)`: the now-playing row's tint.
pub const PLAYING: f32 = 0.12;

/// `bg-white/N`: the theme's text color at `alpha`, which `--color-white` resolves to in every theme.
pub fn white(alpha: f32, cx: &App) -> Hsla {
    cx.theme().foreground.opacity(alpha)
}

/// `bg-black/N`: the theme's background color at `alpha`, for panels that darken what is behind.
pub fn black(alpha: f32, cx: &App) -> Hsla {
    cx.theme().background.opacity(alpha)
}

/// `color-mix(in oklab, var(--color-indigo-500) N%, transparent)`: the accent as a wash.
pub fn accent(alpha: f32, cx: &App) -> Hsla {
    cx.theme().primary.opacity(alpha)
}

/// `text-slate-400`: the muted text every secondary line uses.
pub fn slate_400(cx: &App) -> Hsla {
    cx.theme().muted_foreground
}

/// `text-slate-500`: the solid mid-grey the artist and album columns use, clamped to AA.
pub fn slate_500(cx: &App) -> Hsla {
    theme::dim_text(cx)
}

/// `rounded` — 4px under Normal.
pub fn radius(_: &App) -> Rems {
    skin::metrics().radius
}

/// `rounded-lg` — 8px, the row and small-control radius.
pub fn radius_lg() -> Rems {
    skin::metrics().radius_lg
}

/// `rounded-xl` — 12px, the card and panel radius.
pub fn radius_xl() -> Rems {
    skin::metrics().radius_xl
}

/// `rounded-2xl` — 16px.
pub fn radius_2xl() -> Rems {
    skin::metrics().radius_2xl
}

/// The corner of a small piece of art or a pill sized in pixels: a queue thumb, a tab, a segment.
pub fn thumb_radius() -> Pixels {
    skin::metrics().thumb_radius
}

/// The corner of a cover big enough to read as a picture: the mini player's, a dialog's.
pub fn cover_radius() -> Pixels {
    skin::metrics().cover_radius
}

/// How tall a row stands at `density`: the user picks the step, the skin says what it measures.
pub fn density_row(density: store::layout::Density) -> Pixels {
    skin::metrics().density_row(density)
}

/// A row carrying a cover and two lines: the queue, an album's tracks, a shelf.
pub fn media_row() -> Pixels {
    skin::metrics().media_row
}

/// A bare one-line row, as the playlists panel draws it.
pub fn list_row() -> Pixels {
    skin::metrics().list_row
}

/// The corner of a square chrome button `size` across.
pub fn icon_corner(size: Pixels) -> Pixels {
    match skin::material() {
        // Material's standard icon button is a circle.
        true => size / 2.,
        false => size / 4.,
    }
}

/// The chrome's proportions, so a bar never states its own height.
pub fn chrome() -> skin::Chrome {
    skin::chrome()
}

/// `text-[10px]`, which Tailwind cannot express on its scale.
pub fn text_10() -> Pixels {
    px(10.)
}

/// `text-[11px]`.
pub fn text_11() -> Pixels {
    px(11.)
}

/// `h-14`: the track row, and so the virtualized row height.
pub fn row_height() -> Rems {
    skin::metrics().row_height
}

/// `w-10 h-10`: a desktop row's cover thumb.
pub fn row_cover() -> Rems {
    skin::metrics().row_cover
}

/// `w-9 h-9`: the square icon buttons in the chrome.
pub fn icon_button() -> Rems {
    skin::metrics().icon_button
}

/// A rem measure in pixels, for the setters that take `Pixels` (`Cover::size`, `Icon::size`).
pub fn to_px(measure: Rems, window: &gpui_kit::Window) -> Pixels {
    measure.to_pixels(window.rem_size())
}

/// `text-5xl` — GPUI's own scale stops at `text_3xl`.
pub fn text_5xl() -> Rems {
    rems(3.)
}

/// `text-7xl`, the album detail's title.
pub fn text_7xl() -> Rems {
    rems(4.5)
}

/// `rounded-md` — 6px, the cover thumbs and tab pills.
pub fn radius_md() -> Rems {
    skin::metrics().radius_md
}

/// `text-[9px]`, the repeat-one badge.
pub fn text_9() -> Pixels {
    px(9.)
}

/// A borderless Button's colours in every state; a fill above 0.2 alpha, or a ring, needs [`dress`].
pub fn button(fill: Hsla, hover: Hsla, ink: Hsla, cx: &App) -> ButtonCustomVariant {
    ButtonCustomVariant::new(cx)
        .color(rest(fill))
        .hover(hover)
        .active(hover)
        .foreground(ink)
}

/// No fill until the pointer washes it, keeping `ink`: `.ghost()` without its foreground thrown away.
pub fn quiet_button(ink: Hsla, cx: &App) -> ButtonCustomVariant {
    button(gpui_kit::transparent_black(), white(HOVER, cx), ink, cx)
}

/// A button that paints nothing at all, for a frame around it that owns the wash.
pub fn bare_button(ink: Hsla, cx: &App) -> ButtonCustomVariant {
    let clear = gpui_kit::transparent_black();
    button(clear, clear, ink, cx)
}

/// `bg-white/10 hover:bg-white/20`: the soft button settings cards and toolbars are built from.
pub fn soft_button(ink: Hsla, cx: &App) -> ButtonCustomVariant {
    button(white(HOVER, cx), white(ACTIVE, cx), ink, cx)
}

/// `bg-white/5 hover:bg-white/10`: a toolbar's dropdown triggers, raised a step off the page.
pub fn raised_button(ink: Hsla, cx: &App) -> ButtonCustomVariant {
    button(white(RAISED, cx), white(HOVER, cx), ink, cx)
}

/// `ink` as a wash and as the text: a delete or a connected state, `bg-red-500/20 hover:bg-red-500/30`.
pub fn tint_button(ink: Hsla, cx: &App) -> ButtonCustomVariant {
    button(ink.opacity(ACTIVE), ink.opacity(0.3), ink, cx)
}

/// `ink` as text over nothing, washing faintly in it: a destructive action written as a link.
pub fn tint_text_button(ink: Hsla, cx: &App) -> ButtonCustomVariant {
    button(gpui_kit::transparent_black(), ink.opacity(HOVER), ink, cx)
}

/// `bg-white/15`, the entry a picker list is on.
pub const PICKED: f32 = 0.15;

/// One entry of a picker list: quiet and slate until picked, then washed and white.
pub fn pick_button(picked: bool, cx: &App) -> ButtonCustomVariant {
    let (fill, ink) = match picked {
        true => (white(PICKED, cx), cx.theme().foreground),
        false => (gpui_kit::transparent_black(), slate_400(cx)),
    };
    button(fill, white(RAISED, cx), ink, cx)
}

/// `bg-white text-black`: the one inverted control on a surface, opaque so it goes through [`dress`].
pub fn inverted(
    button: gpui_kit::component::button::Button,
    cx: &App,
) -> gpui_kit::component::button::Button {
    let ink = cx.theme().foreground;
    dress(
        button,
        ink,
        white(TEXT_STRONG, cx),
        cx.theme().background,
        ink,
        cx,
    )
}

/// gpui-component rests a custom variant's `color` at a fifth of its alpha; hover and active paint whole.
const VARIANT_REST: f32 = 0.2;

/// What to hand `.color()` so a borderless `fill` of at most 0.2 alpha rests as given; the border reads it whole.
pub fn rest(fill: Hsla) -> Hsla {
    let mut compensated = fill;
    compensated.a = (fill.a / VARIANT_REST).min(1.);
    compensated
}

/// `fill` rests on the instance at any alpha; the variant's `color` is the border in every state, so it takes `edge` whole.
pub fn dress(
    button: gpui_kit::component::button::Button,
    fill: Hsla,
    hover: Hsla,
    ink: Hsla,
    edge: Hsla,
    cx: &App,
) -> gpui_kit::component::button::Button {
    use gpui_kit::Styled as _;
    use gpui_kit::component::button::ButtonVariants as _;
    button
        .custom(
            ButtonCustomVariant::new(cx)
                .color(edge)
                .hover(hover)
                .active(hover)
                .foreground(ink),
        )
        .bg(fill)
        .border_color(edge)
}
