use std::rc::Rc;

use gpui_kit::{App, ClickEvent, IntoElement, Pixels, RenderOnce, SharedString, Window, px};
use store::{DownloadsModel, Offline};

use crate::icon_button::{IconButton, IconTone};
use crate::{format::t, icons};

/// `w-12 h-12`: the outlined download beside a hero's play.
const HERO: Pixels = px(48.);
/// `w-8 h-8`: the download riding on a card's cover.
const CORNER: Pixels = px(32.);

pub(crate) type ToggleAct = Rc<dyn Fn(&[String], bool, &mut App)>;

/// Where the toggle sits, which sets its size and its colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ToggleSize {
    #[default]
    Hero,
    Corner,
}

/// Download a set of tracks, or give their offline copies back; busy while any of them is on its way.
#[derive(IntoElement)]
pub struct DownloadToggle {
    id: SharedString,
    keys: Rc<[String]>,
    diameter: Pixels,
    tone: IconTone,
    ring: Option<(f32, f32)>,
    offline: Option<Offline>,
    on_toggle: Option<ToggleAct>,
}

impl DownloadToggle {
    pub fn new(id: impl Into<SharedString>, keys: impl Into<Rc<[String]>>) -> Self {
        Self {
            id: id.into(),
            keys: keys.into(),
            diameter: HERO,
            tone: IconTone::Outline,
            ring: None,
            offline: None,
            on_toggle: None,
        }
    }

    pub fn size(self, size: ToggleSize) -> Self {
        match size {
            ToggleSize::Hero => self.diameter(HERO).tone(IconTone::Outline),
            ToggleSize::Corner => self.diameter(CORNER).tone(IconTone::Scrim),
        }
    }

    pub fn diameter(mut self, diameter: Pixels) -> Self {
        self.diameter = diameter;
        self
    }

    pub fn tone(mut self, tone: IconTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn ring(mut self, rest: f32, hot: f32) -> Self {
        self.ring = Some((rest, hot));
        self
    }

    /// Asked to download the keys (true) or give their copies back (false).
    pub fn on_toggle(mut self, toggle: impl Fn(&[String], bool, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(toggle));
        self
    }

    /// A page that already holds the answer for these keys hands it over instead of a rescan.
    pub fn offline(mut self, offline: Offline) -> Self {
        self.offline = Some(offline);
        self
    }
}

impl RenderOnce for DownloadToggle {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let offline = self
            .offline
            .unwrap_or_else(|| DownloadsModel::global(cx).read(cx).offline(&self.keys));
        let (icon, label) = match offline.stored {
            true => (icons::TRASH, t("remove_download")),
            false => (icons::DOWNLOAD, t("download_offline")),
        };
        let (keys, stored, on_toggle) = (self.keys.clone(), offline.stored, self.on_toggle);
        let on_click = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            if let Some(toggle) = &on_toggle {
                toggle(&keys, !stored, cx);
            }
        };
        let button = IconButton::new(self.id, icon)
            .size(self.diameter)
            .tone(self.tone)
            .tooltip(label)
            .loading(offline.busy)
            .disabled(offline.busy || self.keys.is_empty())
            .on_click(on_click);
        match self.ring {
            Some((rest, hot)) => button.ring(rest, hot),
            None => button,
        }
    }
}
