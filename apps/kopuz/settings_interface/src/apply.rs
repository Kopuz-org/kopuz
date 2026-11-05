//! Making `layout.interface` and `ui_style` real: the rem size every `rems(…)` resolves against, the face and the skin.

use std::cell::{Cell, RefCell};

use config::{SettingsEdit, UiEdit, UiStyle};
use gpui_kit::component::Theme;
use gpui_kit::{App, Global, Pixels, SharedString, px};
use store::layout::SCALE_RANGE;
use store::{ConfigModel, LayoutModel};

/// What this module last wrote, and the theme's own values under it.
#[derive(Default)]
struct Applied {
    wrote: RefCell<Option<(Pixels, SharedString)>>,
    base: RefCell<Option<(Pixels, SharedString)>>,
    /// The old skin was sent as `ui_style` this run; a refused or failed write is retried next start.
    migration_sent: Cell<bool>,
}

impl Global for Applied {}

/// Follow `layout.interface` from here on, re-applying it over whatever a theme change installs.
pub fn watch(cx: &mut App) {
    cx.set_global(Applied::default());
    super::font::watch(cx);
    resolve(cx);
    cx.observe(&LayoutModel::global(cx), |_, cx| resolve(cx))
        .detach();
    cx.observe(&ConfigModel::global(cx), |_, cx| resolve(cx))
        .detach();
    // `Root::render` pushes the theme's font size into the window every frame, so this is the lever.
    cx.observe_global::<Theme>(resolve).detach();
}

pub(super) fn resolve(cx: &mut App) {
    migrate(cx);
    skin(cx);
    apply(cx);
}

/// Carry a skin picked under the old `skin` key into `ui_style`, once both stores have been read.
fn migrate(cx: &mut App) {
    let config = ConfigModel::global(cx).read(cx);
    if !config.is_loaded() || config.is_locked("ui_style") {
        return;
    }
    let current = config.config().ui.ui_style;
    // The copy is optimistic, so the key goes only once the daemon took the write, or none is left.
    let settled = match cx.try_global::<Applied>() {
        Some(applied) if applied.migration_sent.get() => config.just_saved("ui_style"),
        _ => !config.is_saving(),
    };
    let layout = LayoutModel::global(cx);
    let Some(legacy) = layout.read(cx).legacy_skin().map(str::to_owned) else {
        return;
    };
    match migrated(Some(&legacy), current) {
        Some(style) => {
            let Some(applied) = cx.try_global::<Applied>() else {
                return;
            };
            if !applied.migration_sent.replace(true) {
                ConfigModel::edit(cx, SettingsEdit::Ui(UiEdit::UiStyle(style)));
            }
        }
        None if settled => layout.update(cx, |layout, cx| layout.forget_legacy_skin(cx)),
        None => {}
    }
}

/// The `ui_style` an old skin choice stands for, unless one has been picked since.
fn migrated(legacy: Option<&str>, current: UiStyle) -> Option<UiStyle> {
    if current != UiStyle::default() {
        return None;
    }
    [UiStyle::Vaxry, UiStyle::Material3]
        .into_iter()
        .find(|style| Some(style.as_str()) == legacy)
}

/// Resolve `ui_style` once per change, and hand the component library the radii it draws with.
fn skin(cx: &mut App) {
    let was = components::skin::active();
    let style = ConfigModel::global(cx).read(cx).config().ui.ui_style;
    components::skin::select(style.as_str());
    // The radii below may not move, but every row, bar and page measures itself by the new skin.
    if components::skin::active() != was {
        cx.refresh_windows();
    }
    let metrics = components::skin::metrics();
    // The kit rounds buttons, fields and the switch by `radius` and dialogs by `radius_lg`.
    let (radius, radius_lg) = match components::skin::material() {
        // Half a 40px button is a full pill; dialogs take Material's extra-large 28.
        true => (px(20.), px(28.)),
        false => (metrics.thumb_radius, metrics.cover_radius),
    };
    let theme = Theme::global(cx);
    if theme.radius == radius && theme.radius_lg == radius_lg {
        return;
    }
    let theme = Theme::global_mut(cx);
    theme.radius = radius;
    theme.radius_lg = radius_lg;
    cx.refresh_windows();
}

fn apply(cx: &mut App) {
    let current = {
        let theme = Theme::global(cx);
        (theme.font_size, theme.font_family.clone())
    };
    let layout = store::layout(cx);
    let scale = layout
        .interface
        .scale
        .clamp(*SCALE_RANGE.start(), *SCALE_RANGE.end());
    let font = layout.interface.font.clone();
    let Some(applied) = cx.try_global::<Applied>() else {
        return;
    };
    // Anything this module did not write is the theme's own answer, and so the new baseline.
    if applied.wrote.borrow().as_ref() != Some(&current) {
        *applied.base.borrow_mut() = Some(current.clone());
    }
    let (base_size, base_family) = applied
        .base
        .borrow()
        .clone()
        .unwrap_or_else(|| current.clone());
    let wanted = (
        base_size * scale,
        font.map(SharedString::from)
            .or_else(|| super::font::family(cx))
            .or_else(|| skin_font(cx))
            .unwrap_or(base_family),
    );
    *applied.wrote.borrow_mut() = Some(wanted.clone());
    if wanted == current {
        return;
    }
    let theme = Theme::global_mut(cx);
    theme.font_size = wanted.0;
    theme.font_family = wanted.1;
    cx.refresh_windows();
}

/// The skin's face, where the system has it; a missing one would draw in GPUI's fallback rather than the theme's.
fn skin_font(cx: &App) -> Option<SharedString> {
    let wanted = components::skin::font()?;
    cx.text_system()
        .all_font_names()
        .iter()
        .any(|name| name == wanted)
        .then(|| SharedString::new_static(wanted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_material_skin_becomes_the_ui_style_once() {
        assert_eq!(
            migrated(Some("material3"), UiStyle::Normal),
            Some(UiStyle::Material3)
        );
        // The second start sees the style it wrote, and the key is gone by then anyway.
        assert_eq!(migrated(Some("material3"), UiStyle::Material3), None);
        assert_eq!(migrated(None, UiStyle::Material3), None);
    }

    #[test]
    fn a_ui_style_already_picked_wins_over_the_old_skin() {
        assert_eq!(migrated(Some("material3"), UiStyle::Vaxry), None);
    }

    #[test]
    fn a_fresh_install_or_the_old_default_migrates_nothing() {
        assert_eq!(migrated(None, UiStyle::Normal), None);
        assert_eq!(migrated(Some("normal"), UiStyle::Normal), None);
        assert_eq!(migrated(Some("neon"), UiStyle::Normal), None);
    }
}
