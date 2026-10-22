use std::collections::BTreeMap;

use material_colors::{color::Argb, hct::Hct, palette::TonalPalette};
use serde::Deserialize;

#[derive(Deserialize)]
struct SystemPalette {
    dark: bool,
    seed: u32,
    neutral: u32,
    colors: BTreeMap<String, u32>,
}

impl SystemPalette {
    fn roles(&self) -> BTreeMap<String, Argb> {
        let mut roles = super::tonal_roles(Argb::from_u32(self.seed), self.dark);
        let neutral = Hct::new(Argb::from_u32(self.neutral));
        let palette = TonalPalette::of(neutral.get_hue(), neutral.get_chroma());
        for (role, light, dark) in [
            ("surface", 98, 6),
            ("background", 98, 6),
            ("surface_dim", 87, 6),
            ("surface_bright", 98, 24),
            ("surface_container_lowest", 100, 4),
            ("surface_container_low", 96, 10),
            ("surface_container", 94, 12),
            ("surface_container_high", 92, 17),
            ("surface_container_highest", 90, 22),
        ] {
            roles.insert(
                role.into(),
                palette.tone(if self.dark { dark } else { light }),
            );
        }
        for (role, color) in &self.colors {
            if let Some(value) = roles.get_mut(role) {
                *value = Argb::from_u32(*color);
            }
        }
        roles
    }
}

#[cfg(target_os = "android")]
pub(super) fn set_enabled(enabled: bool) {
    let _ = dioxus::document::eval(&format!("window.KopuzSystemColors?.setEnabled({enabled})"));
}

#[cfg(target_os = "android")]
pub(super) async fn read() -> Option<String> {
    let mut eval = dioxus::document::eval(
        "dioxus.send(JSON.parse(window.KopuzSystemColors?.palette() ?? 'null'))",
    );
    let palette: Option<SystemPalette> = eval.recv().await.ok()?;
    let palette = palette?;
    Some(super::color_css(&palette.roles(), palette.dark))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_roles_override_generated_colors_and_complete_older_palettes() {
        for dark in [false, true] {
            let palette: SystemPalette = serde_json::from_value(serde_json::json!({
                "dark": dark,
                "seed": 0xffaabbccu32,
                "neutral": 0xff777777u32,
                "colors": { "primary": 0xff123456u32, "unknown_role": 0u32 }
            }))
            .unwrap();
            let roles = palette.roles();
            assert_eq!(roles["primary"], Argb::from_u32(0xff123456));
            assert!(!roles.contains_key("unknown_role"));
            assert_eq!(roles["surface"], roles["background"]);
            let tone = Hct::new(roles["surface_container"]).get_tone();
            assert!((tone - if dark { 12.0 } else { 94.0 }).abs() < 0.5);
        }
    }
}
