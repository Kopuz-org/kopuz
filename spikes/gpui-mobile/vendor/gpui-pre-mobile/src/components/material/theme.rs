//! Material Design 3 Color Theme System
//!
//! Provides a comprehensive MD3 color token system with light and dark
//! mode support. All components in the material module use these tokens
//! for consistent theming.
//!
//! The color values follow the Material Design 3 baseline color scheme
//! as defined at <https://m3.material.io/styles/color/the-color-system/tokens>.
//!
//! # Usage
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::theme::{MaterialTheme, color};
//!
//! let theme = MaterialTheme::dark();
//! let bg = color(theme.surface);
//! let text = color(theme.on_surface);
//! ```

use gpui::Hsla;

/// Convert a `u32` RGB color value (e.g. `0xFF00FF`) to a `gpui::Hsla`.
///
/// This is the canonical way to convert MD3 color tokens into a type that
/// is accepted uniformly by all GPUI style methods (`.bg()`, `.text_color()`,
/// `.border_color()`, etc.) **and** is compatible with `gpui::hsla()` values
/// in the same `if`/`else` expression branches.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material::theme::color;
///
/// let bg = color(0x6750A4); // primary purple as Hsla
/// ```
pub fn color(hex: u32) -> Hsla {
    let r = ((hex >> 16) & 0xFF) as f32 / 255.0;
    let g = ((hex >> 8) & 0xFF) as f32 / 255.0;
    let b = (hex & 0xFF) as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let l = (max + min) / 2.0;

    if delta < f32::EPSILON {
        return gpui::hsla(0.0, 0.0, l, 1.0);
    }

    let s = if l <= 0.5 {
        delta / (max + min)
    } else {
        delta / (2.0 - max - min)
    };

    let h = if (max - r).abs() < f32::EPSILON {
        let mut hue = (g - b) / delta;
        if hue < 0.0 {
            hue += 6.0;
        }
        hue / 6.0
    } else if (max - g).abs() < f32::EPSILON {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };

    gpui::hsla(h, s, l, 1.0)
}

/// Transparent color constant — `hsla(0, 0, 0, 0)`.
///
/// Useful as a default / no-op color in conditional expressions.
pub const TRANSPARENT: Hsla = Hsla {
    h: 0.0,
    s: 0.0,
    l: 0.0,
    a: 0.0,
};

/// A complete Material Design 3 color theme.
///
/// Contains all color tokens needed by MD3 components. Construct via
/// [`MaterialTheme::light`] or [`MaterialTheme::dark`], or create a
/// custom theme by filling in all fields.
#[derive(Debug, Clone, Copy)]
pub struct MaterialTheme {
    // ── Core colours ─────────────────────────────────────────────────
    /// Primary brand colour — used for key components like FABs, buttons.
    pub primary: u32,
    /// Text/icon colour on top of `primary`.
    pub on_primary: u32,
    /// Subtle primary-tinted container — used for tonal buttons, cards.
    pub primary_container: u32,
    /// Text/icon colour on top of `primary_container`.
    pub on_primary_container: u32,

    /// Secondary brand colour — used for less prominent components.
    pub secondary: u32,
    /// Text/icon colour on top of `secondary`.
    pub on_secondary: u32,
    /// Subtle secondary-tinted container.
    pub secondary_container: u32,
    /// Text/icon colour on top of `secondary_container`.
    pub on_secondary_container: u32,

    /// Tertiary accent colour — used for contrasting accents.
    pub tertiary: u32,
    /// Text/icon colour on top of `tertiary`.
    pub on_tertiary: u32,
    /// Subtle tertiary-tinted container.
    pub tertiary_container: u32,
    /// Text/icon colour on top of `tertiary_container`.
    pub on_tertiary_container: u32,

    // ── Error colours ────────────────────────────────────────────────
    /// Error colour — used for error states, destructive actions.
    pub error: u32,
    /// Text/icon colour on top of `error`.
    pub on_error: u32,
    /// Subtle error-tinted container for error banners.
    pub error_container: u32,
    /// Text/icon colour on top of `error_container`.
    pub on_error_container: u32,

    // ── Surface colours ──────────────────────────────────────────────
    /// Base surface colour (app background).
    pub surface: u32,
    /// Primary text/icon colour on `surface`.
    pub on_surface: u32,
    /// Secondary text/icon colour on `surface` (less emphasis).
    pub on_surface_variant: u32,

    /// Slightly tinted surface for elevated containers (elevation 1).
    pub surface_container_lowest: u32,
    /// Surface container at low elevation.
    pub surface_container_low: u32,
    /// Default surface container.
    pub surface_container: u32,
    /// Surface container at high elevation.
    pub surface_container_high: u32,
    /// Surface container at the highest elevation.
    pub surface_container_highest: u32,

    /// The dim variant of surface.
    pub surface_dim: u32,
    /// The bright variant of surface.
    pub surface_bright: u32,

    /// Inverse surface colour — used for snackbars, tooltips.
    pub inverse_surface: u32,
    /// Text/icon on `inverse_surface`.
    pub inverse_on_surface: u32,
    /// Primary colour on inverse surface.
    pub inverse_primary: u32,

    // ── Outline colours ──────────────────────────────────────────────
    /// Outline colour — used for borders, dividers.
    pub outline: u32,
    /// Subtle outline variant — used for decoration, less emphasis.
    pub outline_variant: u32,

    // ── Miscellaneous ────────────────────────────────────────────────
    /// Scrim colour (used for modal overlays) — typically black.
    pub scrim: u32,
    /// Shadow colour — used for elevation shadows.
    pub shadow: u32,

    /// Whether this theme is dark mode.
    pub is_dark: bool,
}

impl MaterialTheme {
    /// Returns the Material Design 3 **light** color scheme.
    ///
    /// Based on Google Blue (#4285F4) as the seed color.
    pub fn light() -> Self {
        Self {
            // Core — primary (Google Blue)
            primary: 0x005AC1,
            on_primary: 0xFFFFFF,
            primary_container: 0xD8E2FF,
            on_primary_container: 0x001A41,

            // Core — secondary
            secondary: 0x575E71,
            on_secondary: 0xFFFFFF,
            secondary_container: 0xDBE2F9,
            on_secondary_container: 0x141B2C,

            // Core — tertiary
            tertiary: 0x715573,
            on_tertiary: 0xFFFFFF,
            tertiary_container: 0xFCD7FB,
            on_tertiary_container: 0x2A132D,

            // Error (Google Red)
            error: 0xBA1A1A,
            on_error: 0xFFFFFF,
            error_container: 0xFFDAD6,
            on_error_container: 0x410002,

            // Surface
            surface: 0xF9F9FF,
            on_surface: 0x1A1B20,
            on_surface_variant: 0x44474F,

            surface_container_lowest: 0xFFFFFF,
            surface_container_low: 0xF3F3FA,
            surface_container: 0xEDEDF4,
            surface_container_high: 0xE8E7EE,
            surface_container_highest: 0xE2E2E9,

            surface_dim: 0xDAD9E0,
            surface_bright: 0xF9F9FF,

            inverse_surface: 0x2F3036,
            inverse_on_surface: 0xF1F0F7,
            inverse_primary: 0xADC6FF,

            // Outline
            outline: 0x74777F,
            outline_variant: 0xC4C6D0,

            // Misc
            scrim: 0x000000,
            shadow: 0x000000,

            is_dark: false,
        }
    }

    /// Returns the Material Design 3 **dark** color scheme.
    ///
    /// Based on Google Blue (#4285F4) as the seed color.
    pub fn dark() -> Self {
        Self {
            // Core — primary (Google Blue)
            primary: 0xADC6FF,
            on_primary: 0x002E69,
            primary_container: 0x004494,
            on_primary_container: 0xD8E2FF,

            // Core — secondary
            secondary: 0xBFC6DC,
            on_secondary: 0x293042,
            secondary_container: 0x3F4759,
            on_secondary_container: 0xDBE2F9,

            // Core — tertiary
            tertiary: 0xDFBBDF,
            on_tertiary: 0x402843,
            tertiary_container: 0x593D5B,
            on_tertiary_container: 0xFCD7FB,

            // Error (Google Red)
            error: 0xFFB4AB,
            on_error: 0x690005,
            error_container: 0x93000A,
            on_error_container: 0xFFDAD6,

            // Surface
            surface: 0x121318,
            on_surface: 0xE2E2E9,
            on_surface_variant: 0xC4C6D0,

            surface_container_lowest: 0x0D0E13,
            surface_container_low: 0x1A1B20,
            surface_container: 0x1E1F25,
            surface_container_high: 0x282A2F,
            surface_container_highest: 0x33353A,

            surface_dim: 0x121318,
            surface_bright: 0x38393F,

            inverse_surface: 0xE2E2E9,
            inverse_on_surface: 0x2F3036,
            inverse_primary: 0x005AC1,

            // Outline
            outline: 0x8E9099,
            outline_variant: 0x44474F,

            // Misc
            scrim: 0x000000,
            shadow: 0x000000,

            is_dark: true,
        }
    }

    /// Convenience constructor — returns [`dark`](Self::dark) or
    /// [`light`](Self::light) based on the boolean flag.
    pub fn from_appearance(dark: bool) -> Self {
        if dark {
            Self::dark()
        } else {
            Self::light()
        }
    }

    // ── Elevation helpers ────────────────────────────────────────────

    /// Returns the appropriate surface colour for a given MD3 elevation
    /// level (0–5).
    ///
    /// In dark mode, higher elevation maps to lighter surface containers.
    /// In light mode, higher elevation maps to slightly tinted containers.
    ///
    /// | Level | Token |
    /// |-------|-------|
    /// | 0     | `surface` |
    /// | 1     | `surface_container_low` |
    /// | 2     | `surface_container` |
    /// | 3     | `surface_container_high` |
    /// | 4–5   | `surface_container_highest` |
    pub fn surface_at_elevation(&self, level: u8) -> u32 {
        match level {
            0 => self.surface,
            1 => self.surface_container_low,
            2 => self.surface_container,
            3 => self.surface_container_high,
            _ => self.surface_container_highest,
        }
    }

    // ── State layer helpers ──────────────────────────────────────────

    /// Returns the opacity for a state layer overlay.
    ///
    /// State layers are semi-transparent overlays applied on top of
    /// component colours to communicate interactive states.
    ///
    /// | State    | Opacity |
    /// |----------|---------|
    /// | Hover    | 0.08    |
    /// | Focus    | 0.12    |
    /// | Pressed  | 0.12    |
    /// | Dragged  | 0.16    |
    /// | Disabled | 0.12 (content) / 0.38 (container) |
    pub fn state_layer_opacity(state: StateLayer) -> f32 {
        match state {
            StateLayer::Hover => 0.08,
            StateLayer::Focus => 0.12,
            StateLayer::Pressed => 0.12,
            StateLayer::Dragged => 0.16,
        }
    }

    /// Returns the disabled content opacity (0.38) per MD3 spec.
    pub fn disabled_content_opacity() -> f32 {
        0.38
    }

    /// Returns the disabled container opacity (0.12) per MD3 spec.
    pub fn disabled_container_opacity() -> f32 {
        0.12
    }
}

impl Default for MaterialTheme {
    /// Defaults to the dark theme.
    fn default() -> Self {
        Self::dark()
    }
}

// ── State Layer enum ─────────────────────────────────────────────────────────

/// Interactive state layers as defined by MD3.
///
/// Each state has a prescribed opacity that should be applied as an
/// overlay on top of the component's base colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateLayer {
    /// Mouse/pointer hover state.
    Hover,
    /// Keyboard/accessibility focus state.
    Focus,
    /// Active press / touch-down state.
    Pressed,
    /// Drag state.
    Dragged,
}

// ── Typography scale ─────────────────────────────────────────────────────────

/// Material Design 3 type scale sizes in logical pixels.
///
/// These follow the MD3 type scale specification:
/// <https://m3.material.io/styles/typography/type-scale-tokens>
///
/// Usage:
/// ```rust,ignore
/// use gpui_mobile::components::material::theme::TypeScale;
///
/// let size = TypeScale::BODY_LARGE; // 16.0
/// ```
pub struct TypeScale;

impl TypeScale {
    // ── Display ──────────────────────────────────────────────────────
    /// Display Large — 57px
    pub const DISPLAY_LARGE: f32 = 57.0;
    /// Display Medium — 45px
    pub const DISPLAY_MEDIUM: f32 = 45.0;
    /// Display Small — 36px
    pub const DISPLAY_SMALL: f32 = 36.0;

    // ── Headline ─────────────────────────────────────────────────────
    /// Headline Large — 32px
    pub const HEADLINE_LARGE: f32 = 32.0;
    /// Headline Medium — 28px
    pub const HEADLINE_MEDIUM: f32 = 28.0;
    /// Headline Small — 24px
    pub const HEADLINE_SMALL: f32 = 24.0;

    // ── Title ────────────────────────────────────────────────────────
    /// Title Large — 22px
    pub const TITLE_LARGE: f32 = 22.0;
    /// Title Medium — 16px
    pub const TITLE_MEDIUM: f32 = 16.0;
    /// Title Small — 14px
    pub const TITLE_SMALL: f32 = 14.0;

    // ── Body ─────────────────────────────────────────────────────────
    /// Body Large — 16px
    pub const BODY_LARGE: f32 = 16.0;
    /// Body Medium — 14px
    pub const BODY_MEDIUM: f32 = 14.0;
    /// Body Small — 12px
    pub const BODY_SMALL: f32 = 12.0;

    // ── Label ────────────────────────────────────────────────────────
    /// Label Large — 14px
    pub const LABEL_LARGE: f32 = 14.0;
    /// Label Medium — 12px
    pub const LABEL_MEDIUM: f32 = 12.0;
    /// Label Small — 11px
    pub const LABEL_SMALL: f32 = 11.0;
}

// ── Shape scale ──────────────────────────────────────────────────────────────

/// Material Design 3 shape (corner radius) tokens in logical pixels.
///
/// <https://m3.material.io/styles/shape/shape-scale-tokens>
pub struct ShapeScale;

impl ShapeScale {
    /// None — 0px (sharp corners)
    pub const NONE: f32 = 0.0;
    /// Extra Small — 4px
    pub const EXTRA_SMALL: f32 = 4.0;
    /// Small — 8px
    pub const SMALL: f32 = 8.0;
    /// Medium — 12px
    pub const MEDIUM: f32 = 12.0;
    /// Large — 16px
    pub const LARGE: f32 = 16.0;
    /// Extra Large — 28px
    pub const EXTRA_LARGE: f32 = 28.0;
    /// Full — very large value to create a pill/circle shape.
    pub const FULL: f32 = 1000.0;
}

// ── Elevation levels ─────────────────────────────────────────────────────────

/// Material Design 3 elevation levels.
///
/// In MD3, elevation is primarily expressed through tonal colour shifts
/// (in dark mode) or subtle shadow (in light mode), rather than heavy
/// drop shadows.
pub struct ElevationLevel;

impl ElevationLevel {
    /// Level 0 — no elevation (flat).
    pub const LEVEL0: u8 = 0;
    /// Level 1 — low elevation (e.g. cards, text fields).
    pub const LEVEL1: u8 = 1;
    /// Level 2 — medium elevation (e.g. elevated buttons, menus).
    pub const LEVEL2: u8 = 2;
    /// Level 3 — high elevation (e.g. navigation bars, FABs).
    pub const LEVEL3: u8 = 3;
    /// Level 4 — higher elevation (e.g. app bars when scrolled).
    pub const LEVEL4: u8 = 4;
    /// Level 5 — highest elevation (e.g. dialogs).
    pub const LEVEL5: u8 = 5;
}

// ── Motion / Duration tokens ─────────────────────────────────────────────────

/// Material Design 3 motion duration tokens in milliseconds.
///
/// <https://m3.material.io/styles/motion/easing-and-duration/tokens-specs>
pub struct MotionDuration;

impl MotionDuration {
    /// Short 1 — 50ms (micro interactions)
    pub const SHORT1: u64 = 50;
    /// Short 2 — 100ms (small transitions)
    pub const SHORT2: u64 = 100;
    /// Short 3 — 150ms (selection changes)
    pub const SHORT3: u64 = 150;
    /// Short 4 — 200ms (icon changes)
    pub const SHORT4: u64 = 200;
    /// Medium 1 — 250ms (expansion)
    pub const MEDIUM1: u64 = 250;
    /// Medium 2 — 300ms (page transitions)
    pub const MEDIUM2: u64 = 300;
    /// Medium 3 — 350ms (larger transitions)
    pub const MEDIUM3: u64 = 350;
    /// Medium 4 — 400ms (complex transitions)
    pub const MEDIUM4: u64 = 400;
    /// Long 1 — 450ms (enter / appear)
    pub const LONG1: u64 = 450;
    /// Long 2 — 500ms (detailed transitions)
    pub const LONG2: u64 = 500;
    /// Long 3 — 550ms (sheet transitions)
    pub const LONG3: u64 = 550;
    /// Long 4 — 600ms (full-screen transitions)
    pub const LONG4: u64 = 600;
    /// Extra Long 1 — 700ms
    pub const EXTRA_LONG1: u64 = 700;
    /// Extra Long 2 — 800ms
    pub const EXTRA_LONG2: u64 = 800;
    /// Extra Long 3 — 900ms
    pub const EXTRA_LONG3: u64 = 900;
    /// Extra Long 4 — 1000ms
    pub const EXTRA_LONG4: u64 = 1000;
}
