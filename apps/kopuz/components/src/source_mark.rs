//! The one mark for a music source: the daemon's own SVG path, or a bundled glyph of the same shape.

use api::{Icon, ServiceInfo, ServiceRef};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    AnyElement, App, Hsla, IntoElement, Pixels, RenderOnce, Rgba, Styled as _, Window, px, svg,
};

use crate::icons;

/// A service as the daemon describes it, whether offered or already added.
pub trait Service {
    fn icon(&self) -> &Icon;
    fn accent(&self) -> &str;
}

impl Service for ServiceInfo {
    fn icon(&self) -> &Icon {
        &self.icon
    }

    fn accent(&self) -> &str {
        &self.accent
    }
}

impl Service for ServiceRef {
    fn icon(&self) -> &Icon {
        &self.icon
    }

    fn accent(&self) -> &str {
        &self.accent
    }
}

/// A service's brand colour; one without a colour, or no service at all, wears the app's accent.
pub fn accent(service: Option<&impl Service>, cx: &App) -> Hsla {
    brand(service.map(Service::accent), cx)
}

fn brand(accent: Option<&str>, cx: &App) -> Hsla {
    accent
        .and_then(|accent| Rgba::try_from(accent).ok())
        .map(Hsla::from)
        .unwrap_or(cx.theme().primary)
}

/// A service's glyph in its brand colour, or the generic server mark when there is none.
#[derive(IntoElement, Clone)]
pub struct SourceMark {
    icon: Option<Icon>,
    size: Pixels,
    color: Option<Hsla>,
    accent: Option<String>,
}

impl SourceMark {
    pub fn new(service: &impl Service) -> Self {
        Self {
            icon: Some(service.icon().clone()),
            size: px(16.),
            color: None,
            accent: Some(service.accent().to_string()),
        }
    }

    /// The mark for no source at all.
    pub fn none() -> Self {
        Self {
            icon: None,
            size: px(16.),
            color: None,
            accent: None,
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    /// Draw it in `color` instead of the brand's.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for SourceMark {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = self
            .color
            .unwrap_or_else(|| brand(self.accent.as_deref(), cx));
        let glyph: AnyElement = match self.icon {
            Some(Icon::Svg(path)) => svg()
                .size(self.size)
                .flex_shrink_0()
                .text_color(color)
                .data(document(&path).as_bytes())
                .into_any_element(),
            Some(Icon::Class(class)) => bundled_glyph(bundled(&class), self.size, color),
            None => bundled_glyph(icons::SERVER, self.size, color),
        };
        glyph
    }
}

fn bundled_glyph(path: &'static str, size: Pixels, color: Hsla) -> AnyElement {
    icons::icon(path)
        .size(size)
        .flex_shrink_0()
        .text_color(color)
        .into_any_element()
}

// The renderer keeps only the alpha of what it rasterizes, so the fill just has to be opaque.
fn document(path: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="black"><path d="{path}"/></svg>"#
    )
}

/// A FontAwesome class cannot be drawn here; every brand shares one mark, the rest map by shape.
fn bundled(class: &str) -> &'static str {
    if class.contains("fa-brands") {
        icons::MUSIC
    } else if class.contains("compact-disc") {
        icons::DISC
    } else if class.contains("cloud") {
        icons::CLOUD
    } else if class.contains("folder") {
        icons::FOLDER_TREE
    } else if class.contains("hard-drive") {
        icons::HARD_DRIVE
    } else {
        icons::SERVER
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_class_still_draws_something() {
        assert_eq!(bundled("fa-solid fa-server"), icons::SERVER);
        assert_eq!(bundled("fa-brands fa-soundcloud"), icons::MUSIC);
        assert_eq!(bundled("fa-solid fa-cloud"), icons::CLOUD);
    }

    #[test]
    fn a_path_becomes_a_standalone_document() {
        assert!(document("M0 0").contains(r#"<path d="M0 0"/>"#));
    }
}
