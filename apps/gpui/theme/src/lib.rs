//! Kopuz themes over GPUI Component's: the 13 palette tokens a Kopuz theme defines, projected onto the component theme's roles.

mod accent;
mod contrast;
mod palettes;
mod system;

use std::collections::HashMap;
use std::rc::Rc;

use config::Ui;
use gpui_kit::component::{Theme, ThemeConfig, ThemeMode};
use gpui_kit::{App, Window};

pub use accent::{
    ACCENT_TWEEN, Surface, accent_for, accent_from, ambient, ground, swatches, wash_for,
};
pub use contrast::{AA, AA_LARGE, contrast, luminance, readable, readable_at, readable_on};
pub use palettes::{BUILT_IN, palette};
pub use system::{SYSTEM, system_colors};

/// A built-in Kopuz theme: the same thirteen colors the themes and the editor use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub name: &'static str,
    pub dark: bool,
    pub bg: &'static str,
    pub raised: &'static str,
    pub surface: &'static str,
    pub text: &'static str,
    pub text_muted: &'static str,
    pub accent: &'static str,
    pub accent_soft: &'static str,
    pub accent_alt: &'static str,
    pub accent_deep: &'static str,
    pub highlight: &'static str,
    pub highlight_dark: &'static str,
    pub progress: &'static str,
    pub danger: &'static str,
}

/// The same thirteen colors, owned, so a user's custom theme can carry them too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Colors {
    pub dark: bool,
    pub bg: String,
    pub raised: String,
    pub surface: String,
    pub text: String,
    pub text_muted: String,
    pub accent: String,
    pub accent_soft: String,
    pub accent_alt: String,
    pub accent_deep: String,
    pub highlight: String,
    pub highlight_dark: String,
    pub progress: String,
    pub danger: String,
}

impl From<&Palette> for Colors {
    fn from(palette: &Palette) -> Self {
        Self {
            dark: palette.dark,
            bg: palette.bg.into(),
            raised: palette.raised.into(),
            surface: palette.surface.into(),
            text: palette.text.into(),
            text_muted: palette.text_muted.into(),
            accent: palette.accent.into(),
            accent_soft: palette.accent_soft.into(),
            accent_alt: palette.accent_alt.into(),
            accent_deep: palette.accent_deep.into(),
            highlight: palette.highlight.into(),
            highlight_dark: palette.highlight_dark.into(),
            progress: palette.progress.into(),
            danger: palette.danger.into(),
        }
    }
}

impl Colors {
    /// The editor's variable names, as `Ui::custom_themes` stores them.
    pub fn from_vars(vars: &HashMap<String, String>) -> Option<Self> {
        let get = |key: &str| vars.get(key).cloned();
        let bg = get("bg")?;
        Some(Self {
            dark: is_dark(&bg),
            bg,
            raised: get("raised")?,
            surface: get("surface")?,
            text: get("text")?,
            text_muted: get("text-muted")?,
            accent: get("accent")?,
            accent_soft: get("accent-soft")?,
            accent_alt: get("accent-alt")?,
            accent_deep: get("accent-deep")?,
            highlight: get("highlight")?,
            highlight_dark: get("highlight-dark")?,
            progress: get("progress")?,
            danger: get("danger")?,
        })
    }
}

struct Active {
    colors: Colors,
    dim_text: gpui_kit::Hsla,
    // Where an ease is headed; the showing colors once it lands.
    settled: Colors,
}

impl gpui_kit::Global for Active {}

/// The palette now showing, for the tokens the component theme has no role for.
pub fn colors(cx: &App) -> Colors {
    cx.try_global::<Active>()
        .map(|active| active.colors.clone())
        .unwrap_or_else(|| palette("default").expect("default theme").into())
}

/// The palette the theme is easing to, or the one showing when it is still.
pub fn settled(cx: &App) -> Colors {
    cx.try_global::<Active>()
        .map(|active| active.settled.clone())
        .unwrap_or_else(|| colors(cx))
}

/// `text-slate-500`, the dimmest body text: `surface` is a panel colour too, so as text it is clamped to AA.
pub fn dim_text(cx: &App) -> gpui_kit::Hsla {
    match cx.try_global::<Active>() {
        Some(active) => active.dim_text,
        None => dim_text_of(&colors(cx)),
    }
}

fn dim_text_of(colors: &Colors) -> gpui_kit::Hsla {
    let background = parse(&colors.bg).unwrap_or_else(|| gpui_kit::hsla(0., 0., 0., 1.));
    let raised = parse(&colors.raised).unwrap_or(background);
    let surface = parse(&colors.surface)
        .or_else(|| parse(&colors.text_muted))
        .unwrap_or(background);
    // A card is `raised`, the page is `bg`: clamping against each in turn clears whichever is harder.
    contrast::readable_on(contrast::readable_on(surface, background), raised)
}

fn parse(color: &str) -> Option<gpui_kit::Hsla> {
    gpui_kit::Rgba::try_from(color).ok().map(Into::into)
}

/// Make these colors the active theme, including the scrollbars the Base layer paints.
pub fn apply(name: &str, colors: &Colors, window: Option<&mut Window>, cx: &mut App) {
    apply_easing(name, colors, colors, window, cx);
}

/// One frame of an ease to `settled`: `colors` show, and what is keyed on the theme renders for where it lands.
pub fn apply_easing(
    name: &str,
    colors: &Colors,
    settled: &Colors,
    window: Option<&mut Window>,
    cx: &mut App,
) {
    cx.set_global(Active {
        colors: colors.clone(),
        dim_text: dim_text_of(colors),
        settled: settled.clone(),
    });
    let mode = if colors.dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    let config: ThemeConfig = match serde_json::from_value(config_value(name, colors, mode)) {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(%error, theme = name, "could not build the theme");
            return;
        }
    };
    let config = Rc::new(config);
    let theme = Theme::global_mut(cx);
    if colors.dark {
        theme.dark_theme = config;
    } else {
        theme.light_theme = config;
    }
    Theme::change(mode, window, cx);
}

/// Upstream toasts rise bottom-centre, clear of whatever the window keeps along its bottom edge.
pub fn place_toasts(clearance: gpui_kit::Pixels, cx: &mut App) {
    let notification = &mut Theme::global_mut(cx).notification;
    notification.placement = gpui_kit::Anchor::BottomCenter;
    notification.margins.bottom = clearance;
}

/// Apply a built-in theme by name, falling back to the default.
pub fn apply_named(name: &str, window: Option<&mut Window>, cx: &mut App) {
    let palette = palette(name).unwrap_or_else(|| palette("default").expect("default theme"));
    apply(palette.name, &palette.into(), window, cx);
}

/// Apply whatever `Ui::theme` names: a built-in, or one of `custom_themes`.
pub fn apply_config(config: &Ui, window: Option<&mut Window>, cx: &mut App) {
    let name = config.theme.as_str();
    if let Some(palette) = palette(name) {
        apply(palette.name, &palette.into(), window, cx);
        return;
    }
    if let Some(colors) = config
        .custom_themes
        .get(name)
        .and_then(|custom| Colors::from_vars(&custom.vars))
    {
        apply(name, &colors, window, cx);
        return;
    }
    // Built from the playing cover, which is only readable mid-render: the shell's frame asks `system_target` and eases to it.
    if name == SYSTEM {
        reset_system(cx);
        if let Some(window) = window {
            window.refresh();
        }
        return;
    }
    // "album-art" and "live" recolor the default from a picture; until that lands they are it.
    apply_named("default", window, cx);
}

/// What System colors was last built from: the cover's swatches and the appearance.
#[derive(Default)]
struct SystemSeed(Option<(Option<store::Swatches>, bool)>);

impl gpui_kit::Global for SystemSeed {}

/// System colors' new scheme when the playing cover's colours or the window's appearance changed, else `None`; call it from a render.
pub fn system_target(window: &mut Window, cx: &mut App) -> Option<Colors> {
    let artwork = store::PlayerModel::global(cx)
        .read(cx)
        .track()
        .and_then(|track| track.artwork.clone());
    // A cover still being read keeps the scheme it has rather than flashing the default.
    let swatches = match &artwork {
        Some(artwork) => match store::palette::of(artwork, window, cx) {
            Some(swatches) => Some(swatches),
            None if cx
                .try_global::<SystemSeed>()
                .is_some_and(|seed| seed.0.is_some()) =>
            {
                return None;
            }
            None => None,
        },
        None => None,
    };
    let dark = matches!(
        window.appearance(),
        gpui_kit::WindowAppearance::Dark | gpui_kit::WindowAppearance::VibrantDark
    );
    let seed = Some((swatches, dark));
    if cx.default_global::<SystemSeed>().0 == seed {
        return None;
    }
    cx.global_mut::<SystemSeed>().0 = seed;
    Some(system_colors(swatches, dark))
}

/// `from` `amount` of the way to `to`, channel by channel; the appearance is `to`'s, as a flip is never eased.
pub fn mix(from: &Colors, to: &Colors, amount: f32) -> Colors {
    let at = |from: &str, to: &str| mix_hex(from, to, amount);
    Colors {
        dark: to.dark,
        bg: at(&from.bg, &to.bg),
        raised: at(&from.raised, &to.raised),
        surface: at(&from.surface, &to.surface),
        text: at(&from.text, &to.text),
        text_muted: at(&from.text_muted, &to.text_muted),
        accent: at(&from.accent, &to.accent),
        accent_soft: at(&from.accent_soft, &to.accent_soft),
        accent_alt: at(&from.accent_alt, &to.accent_alt),
        accent_deep: at(&from.accent_deep, &to.accent_deep),
        highlight: at(&from.highlight, &to.highlight),
        highlight_dark: at(&from.highlight_dark, &to.highlight_dark),
        progress: at(&from.progress, &to.progress),
        danger: at(&from.danger, &to.danger),
    }
}

fn mix_hex(from: &str, to: &str, amount: f32) -> String {
    let (Ok(a), Ok(b)) = (gpui_kit::Rgba::try_from(from), gpui_kit::Rgba::try_from(to)) else {
        return to.to_string();
    };
    let amount = amount.clamp(0., 1.);
    let channel = |a: f32, b: f32| ((a + (b - a) * amount).clamp(0., 1.) * 255.).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(a.r, b.r),
        channel(a.g, b.g),
        channel(a.b, b.b)
    )
}

/// Forget what System colors was built from, so the next follow rebuilds it.
pub fn reset_system(cx: &mut App) {
    cx.default_global::<SystemSeed>().0 = None;
}

/// `#rrggbb` at `a`, which the component theme parses as 8-digit hex.
fn alpha(color: &str, a: f32) -> String {
    let hex = color.trim_start_matches('#');
    let hex = if hex.len() >= 6 { &hex[..6] } else { "000000" };
    format!("#{hex}{:02x}", (a.clamp(0., 1.) * 255.) as u8)
}

/// A theme is dark when its background is; the editor never asks which it is.
fn is_dark(background: &str) -> bool {
    let hex = background.trim_start_matches('#');
    if hex.len() < 6 {
        return true;
    }
    let channel =
        |at: usize| u8::from_str_radix(hex.get(at..at + 2).unwrap_or("00"), 16).unwrap_or(0) as f32;
    (0.299 * channel(0) + 0.587 * channel(2) + 0.114 * channel(4)) < 128.
}

fn config_value(name: &str, colors: &Colors, mode: ThemeMode) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    let mut set = |key: &str, value: &str| {
        map.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    };
    // Kopuz's surfaces are its text color at low alpha (Tailwind `bg-white/N`),
    // so the component library's own components are given the same.
    let raised = alpha(&colors.text, 0.05);
    let hover = alpha(&colors.text, 0.10);
    let active = alpha(&colors.text, 0.20);
    set("background", &colors.bg);
    set("foreground", &colors.text);
    set("border", &hover);
    set("muted.background", &raised);
    set("muted.foreground", &colors.text_muted);
    set("secondary.background", &raised);
    set("secondary.foreground", &colors.text);
    set("secondary.hover.background", &hover);
    set("secondary.active.background", &active);
    set("popover.background", &colors.raised);
    set("popover.foreground", &colors.text);
    set("accent.background", &hover);
    set("accent.foreground", &colors.text);
    set("primary.background", &colors.accent);
    set("primary.foreground", &colors.bg);
    set("primary.hover.background", &colors.accent_soft);
    set("primary.active.background", &colors.accent_alt);
    set("ring", &colors.accent);
    set("link", &colors.accent);
    set("link.hover", &colors.accent_soft);
    set("link.active", &colors.accent_alt);
    set("progress.bar.background", &colors.progress);
    // A slider's empty track is this colour at 20%, so the progress green here
    // made every idle slider read as full; Kopuz's sliders are white/90.
    set("slider.background", &alpha(&colors.text, 0.90));
    set("slider.thumb.background", &colors.text);
    set("switch.background", &colors.surface);
    set("switch.thumb.background", &colors.text);
    set("selection.background", &colors.accent_deep);
    set("danger.background", &colors.danger);
    set("danger.foreground", &colors.text);
    set("sidebar.background", &alpha(&colors.bg, 0.40));
    set("sidebar.foreground", &colors.text);
    set("sidebar.border", &alpha(&colors.text, 0.05));
    set("sidebar.accent.background", &hover);
    set("sidebar.accent.foreground", &colors.text);
    set("sidebar.primary.background", &colors.accent);
    set("sidebar.primary.foreground", &colors.bg);
    set("title_bar.background", &colors.bg);
    set("title_bar.border", &colors.surface);
    set("status_bar.background", &colors.raised);
    set("status_bar.border", &colors.surface);
    set("list.background", &colors.bg);
    set("list.hover.background", &hover);
    set("list.active.background", &active);
    set("list.active.border", &colors.accent);
    set("table.background", &colors.bg);
    set("table.hover.background", &hover);
    set("table.active.background", &active);
    set("input.border", &hover);
    set("tab.background", &colors.raised);
    set("tab.foreground", &colors.text_muted);
    set("tab.active.background", &colors.surface);
    set("tab.active.foreground", &colors.text);
    set("tab_bar.background", &colors.raised);
    set("scrollbar.background", &colors.bg);
    set("scrollbar.thumb.background", &colors.surface);
    set("scrollbar.thumb.hover.background", &colors.text_muted);
    set("skeleton.background", &raised);
    set("drag.border", &colors.accent);
    set("drop_target.background", &colors.accent_deep);
    set("caret", &colors.accent);
    // The shell paints the modal scrim itself so it can fade; the library's own would cut in.
    set("overlay", &alpha(&colors.bg, 0.));
    set("button.background", &raised);
    set("button.foreground", &colors.text);
    set("button.hover.background", &hover);
    set("button.active.background", &active);
    set("button.primary.background", &colors.accent);
    set("button.primary.foreground", &colors.bg);
    set("button.primary.hover.background", &colors.accent_soft);
    set("button.primary.active.background", &colors.accent_alt);
    set("button.danger.background", &colors.danger);
    set("button.danger.foreground", &colors.text);
    set("info.background", &colors.highlight);
    set("info.foreground", &colors.text);
    set("success.background", &colors.progress);
    set("success.foreground", &colors.bg);

    let mut root = serde_json::Map::new();
    root.insert("name".into(), serde_json::Value::String(name.to_string()));
    let mode = if mode.is_dark() { "dark" } else { "light" };
    root.insert("mode".into(), serde_json::Value::String(mode.to_string()));
    root.insert("colors".into(), serde_json::Value::Object(map));
    serde_json::Value::Object(root)
}

/// The default theme's colours, for a screen drawn before the user's own theme is known.
#[derive(Debug, Clone, Copy)]
pub struct Startup {
    pub background: gpui_kit::Hsla,
    pub text: gpui_kit::Hsla,
    pub muted: gpui_kit::Hsla,
    pub accent: gpui_kit::Hsla,
}

pub fn startup() -> Startup {
    let fallback = gpui_kit::Hsla::default();
    let default = palette("default");
    let pick = |color: Option<&str>| color.and_then(parse).unwrap_or(fallback);
    Startup {
        background: pick(default.map(|palette| palette.bg)),
        text: pick(default.map(|palette| palette.text)),
        muted: pick(default.map(|palette| palette.text_muted)),
        accent: pick(default.map(|palette| palette.accent)),
    }
}
