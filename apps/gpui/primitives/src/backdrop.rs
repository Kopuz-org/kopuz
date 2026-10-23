//! A picture behind a whole surface: blurred, darkened, and read over rather than looked at.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    AnyElement, App, ImageSource, IntoElement, ObjectFit, ParentElement as _, RenderImage, Styled,
    StyledImage as _, Window, div, img, relative,
};
use store::ambient::Cast;

use crate::motion;

/// `source` filling its parent, under a scrim of the theme's own background at `darkening` percent.
pub fn wallpaper(source: ImageSource, darkening: u8, blur: u8, cx: &App) -> AnyElement {
    let scrim = f32::from(darkening.min(95)) / 100.;
    // Overscanned by the blur radius, so the softened edge never shows as a seam.
    let scale = 1. + f32::from(blur.min(100)) * 0.004;
    let inset = (1. - scale) / 2.;
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .overflow_hidden()
        .bg(cx.theme().background)
        .child(
            img(store::blurred_backdrop(source, blur))
                .absolute()
                .top(relative(inset))
                .left(relative(inset))
                .w(relative(scale))
                .h(relative(scale))
                .object_fit(ObjectFit::Cover),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(cx.theme().background.opacity(scrim)),
        )
        .into_any_element()
}

/// A chosen picture outranks `artwork`, and either outranks its light: the order the settings promise.
pub fn layer(
    artwork: Option<&api::ArtworkRef>,
    min_darkening: u8,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let (custom, cover, darkening, blur) = {
        let config = store::ConfigModel::global(cx);
        let config = config.read(cx).config();
        (
            config.background.custom_background_path.clone(),
            config.background.cover_art_background,
            config.background.cover_art_darkening.max(min_darkening),
            config.background.cover_art_blur,
        )
    };
    if !custom.is_empty() {
        let source: ImageSource = std::path::PathBuf::from(custom).into();
        return Some(wallpaper(source, darkening, blur, cx));
    }
    if cover {
        let source = store::artwork_source(artwork?, true);
        return Some(wallpaper(source, darkening, blur, cx));
    }
    ambient(window, cx)
}

/// The accent's own ease, so the light and the tint it casts land together.
const CROSSFADE: Duration = theme::ACCENT_TWEEN;

/// The field that is up, the one it is fading from, and a count that keys each change's own fade.
#[derive(Default)]
struct Shown {
    light: Option<Arc<RenderImage>>,
    from: Option<Arc<RenderImage>>,
    turn: u64,
}

/// The light the playing cover casts, crossfaded between tracks; `None` while nothing is lit.
pub fn ambient(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    // `None` is still rendering, so whatever is up stays up rather than flashing the bare theme.
    let wanted = match theme::ambient(cx) {
        None => Some(None),
        Some((artwork, spread)) => {
            match store::ambient::cast(&artwork, theme::ground(cx), spread, window, cx) {
                Cast::Pending => None,
                Cast::Nothing => Some(None),
                Cast::Light(field) => Some(Some(field)),
            }
        }
    };
    let shown = window.use_keyed_state("window-ambient", cx, |_, _| Shown::default());
    if let Some(wanted) = wanted {
        let changed = match (&shown.read(cx).light, &wanted) {
            (Some(up), Some(wanted)) => !Arc::ptr_eq(up, wanted),
            (None, None) => false,
            _ => true,
        };
        if changed {
            shown.update(cx, |shown, cx| {
                shown.from = shown.light.take();
                shown.light = wanted;
                shown.turn += 1;
                cx.notify();
            });
        }
    }

    let (light, from, turn) = {
        let shown = shown.read(cx);
        (shown.light.clone(), shown.from.clone(), shown.turn)
    };
    let arrived = motion::enter(motion::id("window-ambient", turn), CROSSFADE, window, cx);
    if arrived >= 1. && from.is_some() {
        shown.update(cx, |shown, _| shown.from = None);
    }
    if light.is_none() && from.is_none() {
        return None;
    }
    // An opaque field fades in over the old one; with nothing arriving, the old one fades out.
    let leaving = match light.is_some() {
        true => 1.,
        false => 1. - arrived,
    };
    Some(
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .children(from.map(|field| stretched(field, leaving)))
            .children(light.map(|field| stretched(field, arrived)))
            .into_any_element(),
    )
}

/// The field is a colour field with no shape of its own, so it is stretched rather than fitted.
fn stretched(field: Arc<RenderImage>, opacity: f32) -> impl IntoElement {
    img(ImageSource::Render(field))
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .object_fit(ObjectFit::Fill)
        .opacity(opacity)
}
