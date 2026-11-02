//! System colors' recolour on a track change, eased from the scheme on screen to the new cover's.

use components::motion;
use gpui_kit::{App, Window};

/// The scheme being eased to, where it started from, and how far along it is.
#[derive(Default)]
pub struct SystemEase {
    from: Option<theme::Colors>,
    to: Option<theme::Colors>,
    fade: motion::Fade,
    landed: bool,
}

impl SystemEase {
    /// Take up a new scheme when there is one, then apply where the ease stands; call it from a render.
    pub fn follow(&mut self, window: &mut Window, cx: &mut App) {
        if let Some(to) = theme::system_target(window, cx) {
            let shown = theme::colors(cx);
            // The first scheme has nothing of its own on screen to come from, and an appearance flip is not a recolour.
            self.from = (self.to.is_some() && shown.dark == to.dark).then_some(shown);
            self.fade = match self.from {
                Some(_) => motion::Fade::hidden(),
                None => motion::Fade::shown(),
            };
            self.to = Some(to);
            self.landed = false;
        }
        let Some(to) = self.to.as_ref().filter(|_| !self.landed) else {
            return;
        };
        let amount = self.fade.sample(true, theme::ACCENT_TWEEN, window, cx);
        let colors = match &self.from {
            Some(from) if amount < 1. => theme::mix(from, to, amount),
            _ => to.clone(),
        };
        theme::apply_easing(theme::SYSTEM, &colors, to, Some(window), cx);
        self.landed = amount >= 1.;
    }
}
