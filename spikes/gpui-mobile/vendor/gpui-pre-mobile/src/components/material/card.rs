//! Material Design 3 Card components.
#![allow(unused_imports)]
//!
//! Cards contain content and actions about a single subject. MD3 defines
//! three card variants:
//!
//! - **Filled** — a card with a tinted surface background and no border.
//!   Used for grouping content with subtle emphasis.
//! - **Elevated** — a card with the base surface background and a subtle
//!   shadow/border to lift it off the page. The default card style.
//! - **Outlined** — a card with a transparent background and a visible
//!   outline border. Best for grouping related content without heavy emphasis.
//!
//! All card types follow a builder pattern and implement `IntoElement`.
//! Cards are container components — you add children to them using the
//! `.child(...)` method after building.
//!
//! # Examples
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::card::*;
//! use gpui_mobile::components::material::theme::MaterialTheme;
//!
//! let theme = MaterialTheme::dark();
//!
//! // Filled card (default)
//! let card = Card::filled(theme)
//!     .child(title_row)
//!     .child(body_text)
//!     .child(action_buttons);
//!
//! // Elevated card
//! let card = Card::elevated(theme)
//!     .child(content);
//!
//! // Outlined card
//! let card = Card::outlined(theme)
//!     .child(content);
//!
//! // Card with click handler (makes the card tappable)
//! let card = Card::filled(theme)
//!     .on_click(cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .child(content);
//!
//! // Convenience: pre-built card with header, body, actions
//! let card = CardBuilder::new(theme)
//!     .variant(CardVariant::Elevated)
//!     .avatar("AB", 0x6750A4)
//!     .title("Card Title")
//!     .subtitle("Supporting text")
//!     .body("This is the card body content with a longer description.")
//!     .action_text("Cancel", |_, _, _| {})
//!     .action_filled("Accept", |_, _, _| {})
//!     .build();
//! ```

use gpui::{div, prelude::*, px, rgb, AnyElement, MouseButton, MouseDownEvent, Stateful, Window};

use super::theme::{MaterialTheme, ShapeScale};

// ── Constants ────────────────────────────────────────────────────────────────

/// Default card corner radius (MD3 medium shape = 12dp).
const CARD_RADIUS: f32 = ShapeScale::MEDIUM;

/// Card internal padding.
const CARD_PADDING: f32 = 16.0;

/// Avatar size in the card header.
const AVATAR_SIZE: f32 = 40.0;

/// Gap between avatar and title column.
const HEADER_GAP: f32 = 16.0;

/// Gap between card sections (header, media, body, actions).
const SECTION_GAP: f32 = 16.0;

/// Gap between action buttons.
const ACTION_GAP: f32 = 8.0;

/// Minimum card width.
const _MIN_CARD_WIDTH: f32 = 0.0;

// ── Click handler alias ──────────────────────────────────────────────────────

type ClickHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static>;

// ── Card variant ─────────────────────────────────────────────────────────────

/// The visual variant of a Material Design 3 card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardVariant {
    /// Filled card — tinted surface-container-highest background, no border.
    /// Provides the most visual weight of the three variants.
    #[default]
    Filled,
    /// Elevated card — base surface background with a subtle shadow/border.
    /// The default choice for most card use cases.
    Elevated,
    /// Outlined card — transparent surface background with an outline border.
    /// Lowest visual weight, best for dense layouts or lists.
    Outlined,
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Card (container)
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Card** container.
///
/// This is the low-level card component — a styled container that you
/// populate with children. For a higher-level API with built-in header,
/// body, and action slots, see [`CardBuilder`].
///
/// The card supports three visual variants via [`CardVariant`]:
///
/// | Variant   | Background                    | Border           |
/// |-----------|-------------------------------|------------------|
/// | Filled    | `surface_container_highest`   | None             |
/// | Elevated  | `surface_container_low`       | Subtle shadow    |
/// | Outlined  | `surface`                     | `outline_variant`|
///
/// Cards are rounded containers with `overflow_hidden`, so children that
/// extend to the edges (like images) will be clipped to the card's shape.
pub struct Card {
    theme: MaterialTheme,
    variant: CardVariant,
    children: Vec<AnyElement>,
    on_click: Option<ClickHandler>,
    id: Option<gpui::ElementId>,
    /// Custom padding override (None = use default CARD_PADDING).
    padding: Option<f32>,
    /// Custom corner radius override (None = use CARD_RADIUS).
    radius: Option<f32>,
    /// Whether the card should take full width of its container.
    full_width: bool,
    /// Whether the card content is arranged as a column (default) or row.
    horizontal: bool,
    /// Custom gap between children (None = no explicit gap).
    gap: Option<f32>,
}

impl Card {
    // ── Variant constructors ─────────────────────────────────────────

    /// Create a **filled** card.
    ///
    /// Filled cards use `surface_container_highest` as their background
    /// and have no border. They provide the most visual weight.
    pub fn filled(theme: MaterialTheme) -> Self {
        Self::new(theme, CardVariant::Filled)
    }

    /// Create an **elevated** card.
    ///
    /// Elevated cards use `surface_container_low` as their background
    /// with a subtle shadow simulation (thin border). This is the most
    /// common card style for standalone content.
    pub fn elevated(theme: MaterialTheme) -> Self {
        Self::new(theme, CardVariant::Elevated)
    }

    /// Create an **outlined** card.
    ///
    /// Outlined cards use the base `surface` background with a visible
    /// `outline_variant` border. Best for dense layouts, lists, or when
    /// you want minimal visual emphasis.
    pub fn outlined(theme: MaterialTheme) -> Self {
        Self::new(theme, CardVariant::Outlined)
    }

    /// Create a card with a specific variant.
    pub fn new(theme: MaterialTheme, variant: CardVariant) -> Self {
        Self {
            theme,
            variant,
            children: Vec::new(),
            on_click: None,
            id: None,
            padding: None,
            radius: None,
            full_width: false,
            horizontal: false,
            gap: None,
        }
    }

    // ── Children ─────────────────────────────────────────────────────

    /// Add a child element to the card.
    ///
    /// Children are rendered in order, arranged vertically (column) by
    /// default. Use [`horizontal`](Self::horizontal) to switch to a row layout.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    /// Add multiple children at once.
    pub fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        for child in children {
            self.children.push(child.into_any_element());
        }
        self
    }

    // ── Configuration ────────────────────────────────────────────────

    /// Set a click handler, making the card tappable.
    ///
    /// When a card has a click handler, it becomes interactive and shows
    /// a pointer cursor on hover.
    pub fn on_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Set an explicit element ID.
    ///
    /// Required when the card has a click handler or when multiple cards
    /// need distinct identities for GPUI's element diffing.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Override the default padding (16dp).
    ///
    /// Pass `0.0` for no padding (useful when the card contains a
    /// full-bleed image or custom layout).
    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = Some(padding);
        self
    }

    /// Override the default corner radius (12dp).
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = Some(radius);
        self
    }

    /// Make the card take the full width of its container.
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    /// Arrange children horizontally (row) instead of vertically (column).
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }

    /// Set a custom gap between children (in logical pixels).
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = Some(gap);
        self
    }
}

impl IntoElement for Card {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let pad = self.padding.unwrap_or(CARD_PADDING);
        let rad = self.radius.unwrap_or(CARD_RADIUS);

        // Determine colors based on variant
        let (bg_color, border_color, has_border, has_shadow) = match self.variant {
            CardVariant::Filled => (t.surface_container_highest, 0u32, false, false),
            CardVariant::Elevated => (t.surface_container_low, t.shadow, false, true),
            CardVariant::Outlined => (t.surface, t.outline_variant, true, false),
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("card".into()));

        // Start building the card div — always call .id() upfront so the
        // type is consistently Stateful<Div>.
        let mut card = div()
            .id(elem_id)
            .flex()
            .rounded(px(rad))
            .bg(rgb(bg_color))
            .overflow_hidden();

        // Layout direction
        if self.horizontal {
            card = card.flex_row();
        } else {
            card = card.flex_col();
        }

        // Gap between children
        if let Some(gap_val) = self.gap {
            card = card.gap(px(gap_val));
        }

        // Padding (applied uniformly)
        if pad > 0.0 {
            card = card.p(px(pad));
        }

        // Width
        if self.full_width {
            card = card.w_full();
        }

        // Border (outlined variant)
        if has_border {
            card = card.border_1().border_color(rgb(border_color));
        }

        // Shadow simulation for elevated variant (subtle bottom/right border)
        if has_shadow {
            card = card
                .border_1()
                .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.08));
        }

        // Interactive state
        if self.on_click.is_some() {
            card = card.cursor_pointer();
        }

        // Add children
        for child in self.children {
            card = card.child(child);
        }

        // Click handler
        if let Some(handler) = self.on_click {
            card = card.on_mouse_down(MouseButton::Left, handler);
        }

        card.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  CardBuilder (high-level convenience API)
// ═══════════════════════════════════════════════════════════════════════════════

/// Internal descriptor for a card action button.
struct CardAction {
    label: String,
    style: CardActionStyle,
    on_click: ClickHandler,
}

/// The visual style of a card action button.
enum CardActionStyle {
    /// Text button (low emphasis) — just coloured text.
    Text,
    /// Filled button (high emphasis) — solid primary background.
    Filled,
    /// Outlined button (medium emphasis) — outline border.
    Outlined,
    /// Tonal button (medium emphasis) — secondary container background.
    Tonal,
}

/// A high-level builder for Material Design 3 cards with pre-defined
/// slots for common card layouts.
///
/// The [`CardBuilder`] provides a convenient API for assembling cards
/// with a standard structure:
///
/// 1. **Header** — optional avatar, title, and subtitle
/// 2. **Media** — optional full-bleed media element (image, etc.)
/// 3. **Body** — optional body text
/// 4. **Actions** — optional row of action buttons
///
/// ```text
/// ╭────────────────────────────────────╮
/// │  [AV]  Title                       │  ← header
/// │        Subtitle                    │
/// │────────────────────────────────────│
/// │        [Media Area]                │  ← optional media
/// │────────────────────────────────────│
/// │  Body text content goes here and   │  ← body
/// │  can span multiple lines.          │
/// │────────────────────────────────────│
/// │                  [Cancel] [Accept] │  ← actions
/// ╰────────────────────────────────────╯
/// ```
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material::card::*;
/// use gpui_mobile::components::material::theme::MaterialTheme;
///
/// let theme = MaterialTheme::dark();
///
/// let card = CardBuilder::new(theme)
///     .variant(CardVariant::Elevated)
///     .avatar("JD", 0x6750A4)
///     .title("John Doe")
///     .subtitle("2 hours ago")
///     .body("Shared a new photo from the weekend trip to the mountains.")
///     .action_text("Comment", |_, _, _| {})
///     .action_filled("Like", |_, _, _| {})
///     .build();
/// ```
pub struct CardBuilder {
    theme: MaterialTheme,
    variant: CardVariant,
    // Header
    avatar_text: Option<String>,
    avatar_bg: Option<u32>,
    avatar_element: Option<AnyElement>,
    title: Option<String>,
    subtitle: Option<String>,
    trailing_element: Option<AnyElement>,
    // Media
    media: Option<AnyElement>,
    // Body
    body: Option<String>,
    body_element: Option<AnyElement>,
    // Actions
    actions: Vec<CardAction>,
    // Custom content (appended after body, before actions)
    custom_content: Vec<AnyElement>,
    // Configuration
    on_click: Option<ClickHandler>,
    id: Option<gpui::ElementId>,
    full_width: bool,
}

impl CardBuilder {
    /// Create a new card builder with the given theme.
    ///
    /// Default variant is [`CardVariant::Filled`].
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            variant: CardVariant::Filled,
            avatar_text: None,
            avatar_bg: None,
            avatar_element: None,
            title: None,
            subtitle: None,
            trailing_element: None,
            media: None,
            body: None,
            body_element: None,
            actions: Vec::new(),
            custom_content: Vec::new(),
            on_click: None,
            id: None,
            full_width: false,
        }
    }

    // ── Variant ──────────────────────────────────────────────────────

    /// Set the card variant.
    pub fn variant(mut self, variant: CardVariant) -> Self {
        self.variant = variant;
        self
    }

    // ── Header ───────────────────────────────────────────────────────

    /// Set a text avatar (e.g. initials like "AB").
    ///
    /// The avatar is rendered as a circular badge with the given text
    /// on a coloured background.
    ///
    /// # Parameters
    ///
    /// - `text` — avatar text (typically 1-2 characters)
    /// - `bg` — background colour (RGB u32)
    pub fn avatar(mut self, text: impl Into<String>, bg: u32) -> Self {
        self.avatar_text = Some(text.into());
        self.avatar_bg = Some(bg);
        self
    }

    /// Set a custom avatar element (e.g. an image or icon).
    ///
    /// This replaces the text avatar if both are set.
    pub fn avatar_element(mut self, element: impl IntoElement) -> Self {
        self.avatar_element = Some(element.into_any_element());
        self
    }

    /// Set the card title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the card subtitle (displayed below the title in the header).
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Set a trailing element in the header row (e.g. a timestamp, icon button).
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing_element = Some(element.into_any_element());
        self
    }

    // ── Media ────────────────────────────────────────────────────────

    /// Set a media element (e.g. an image) displayed between the header and body.
    ///
    /// The media element is rendered full-width with no padding (full-bleed).
    pub fn media(mut self, element: impl IntoElement) -> Self {
        self.media = Some(element.into_any_element());
        self
    }

    // ── Body ─────────────────────────────────────────────────────────

    /// Set the body text content.
    pub fn body(mut self, text: impl Into<String>) -> Self {
        self.body = Some(text.into());
        self
    }

    /// Set a custom body element (replaces body text if both are set).
    pub fn body_element(mut self, element: impl IntoElement) -> Self {
        self.body_element = Some(element.into_any_element());
        self
    }

    // ── Custom content ───────────────────────────────────────────────

    /// Add a custom content element between the body and actions.
    ///
    /// Useful for adding chips, tags, ratings, or other non-standard
    /// card content.
    pub fn content(mut self, element: impl IntoElement) -> Self {
        self.custom_content.push(element.into_any_element());
        self
    }

    // ── Actions ──────────────────────────────────────────────────────

    /// Add a **text** action button (low emphasis).
    pub fn action_text(
        mut self,
        label: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.actions.push(CardAction {
            label: label.into(),
            style: CardActionStyle::Text,
            on_click: Box::new(on_click),
        });
        self
    }

    /// Add a **filled** action button (high emphasis).
    pub fn action_filled(
        mut self,
        label: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.actions.push(CardAction {
            label: label.into(),
            style: CardActionStyle::Filled,
            on_click: Box::new(on_click),
        });
        self
    }

    /// Add an **outlined** action button (medium emphasis).
    pub fn action_outlined(
        mut self,
        label: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.actions.push(CardAction {
            label: label.into(),
            style: CardActionStyle::Outlined,
            on_click: Box::new(on_click),
        });
        self
    }

    /// Add a **tonal** action button (medium emphasis).
    pub fn action_tonal(
        mut self,
        label: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.actions.push(CardAction {
            label: label.into(),
            style: CardActionStyle::Tonal,
            on_click: Box::new(on_click),
        });
        self
    }

    // ── Configuration ────────────────────────────────────────────────

    /// Make the entire card clickable.
    pub fn on_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Make the card take the full width of its container.
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    // ── Build ────────────────────────────────────────────────────────

    /// Build and return the card element.
    ///
    /// Consumes the builder.
    pub fn build(self) -> impl IntoElement {
        let t = self.theme;
        let has_header =
            self.avatar_text.is_some() || self.avatar_element.is_some() || self.title.is_some();
        let has_body = self.body.is_some() || self.body_element.is_some();
        let has_actions = !self.actions.is_empty();
        let has_media = self.media.is_some();
        let has_custom = !self.custom_content.is_empty();

        // Determine card colors based on variant
        let (bg_color, border_color, has_border, has_shadow) = match self.variant {
            CardVariant::Filled => (t.surface_container_highest, 0u32, false, false),
            CardVariant::Elevated => (t.surface_container_low, t.shadow, false, true),
            CardVariant::Outlined => (t.surface, t.outline_variant, true, false),
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("card-builder".into()));

        // Build the outer card container — always call .id() upfront
        let mut card = div()
            .id(elem_id)
            .flex()
            .flex_col()
            .rounded(px(CARD_RADIUS))
            .bg(rgb(bg_color))
            .overflow_hidden();

        if self.full_width {
            card = card.w_full();
        }

        if has_border {
            card = card.border_1().border_color(rgb(border_color));
        }

        if has_shadow {
            card = card
                .border_1()
                .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.08));
        }

        if self.on_click.is_some() {
            card = card.cursor_pointer();
        }

        // ── Header section ───────────────────────────────────────────
        if has_header {
            let mut header = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(HEADER_GAP))
                .px(px(CARD_PADDING))
                .pt(px(CARD_PADDING));

            // Avatar
            if let Some(avatar_el) = self.avatar_element {
                header = header.child(avatar_el);
            } else if let Some(avatar_text) = &self.avatar_text {
                let avatar_bg = self.avatar_bg.unwrap_or(t.primary);
                let avatar_fg = if self.avatar_bg.is_some() {
                    // Use a contrasting color — for custom bg, use white or on_primary
                    t.on_primary
                } else {
                    t.on_primary
                };

                header = header.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(AVATAR_SIZE))
                        .rounded_full()
                        .bg(rgb(avatar_bg))
                        .text_color(rgb(avatar_fg))
                        .text_sm()
                        .child(avatar_text.clone()),
                );
            }

            // Title column (title + subtitle)
            if self.title.is_some() || self.subtitle.is_some() {
                let mut title_col = div().flex().flex_col().flex_1();

                if let Some(title_text) = &self.title {
                    title_col = title_col.child(
                        div()
                            .text_base()
                            .line_height(px(24.0))
                            .text_color(rgb(t.on_surface))
                            .child(title_text.clone()),
                    );
                }

                if let Some(subtitle_text) = &self.subtitle {
                    title_col = title_col.child(
                        div()
                            .text_sm()
                            .line_height(px(20.0))
                            .text_color(rgb(t.on_surface_variant))
                            .child(subtitle_text.clone()),
                    );
                }

                header = header.child(title_col);
            }

            // Trailing element
            if let Some(trailing) = self.trailing_element {
                header = header.child(trailing);
            }

            card = card.child(header);
        }

        // ── Media section (full-bleed, no padding) ───────────────────
        if let Some(media_el) = self.media {
            let mut media_container = div().w_full();
            if has_header {
                media_container = media_container.mt(px(SECTION_GAP));
            }
            media_container = media_container.child(media_el);
            card = card.child(media_container);
        }

        // ── Body section ─────────────────────────────────────────────
        if has_body {
            let mut body_container = div().px(px(CARD_PADDING));

            if has_header || has_media {
                body_container = body_container.pt(px(SECTION_GAP));
            } else {
                body_container = body_container.pt(px(CARD_PADDING));
            }

            if let Some(body_el) = self.body_element {
                body_container = body_container.child(body_el);
            } else if let Some(body_text) = &self.body {
                body_container = body_container.child(
                    div()
                        .text_sm()
                        .line_height(px(20.0))
                        .text_color(rgb(t.on_surface_variant))
                        .child(body_text.clone()),
                );
            }

            card = card.child(body_container);
        }

        // ── Custom content ───────────────────────────────────────────
        if has_custom {
            for content_el in self.custom_content {
                card = card.child(
                    div()
                        .px(px(CARD_PADDING))
                        .pt(px(SECTION_GAP / 2.0))
                        .child(content_el),
                );
            }
        }

        // ── Actions section ──────────────────────────────────────────
        if has_actions {
            let mut actions_row = div()
                .flex()
                .flex_row()
                .justify_end()
                .items_center()
                .gap(px(ACTION_GAP))
                .px(px(CARD_PADDING))
                .pt(px(SECTION_GAP))
                .pb(px(CARD_PADDING));

            for (i, action) in self.actions.into_iter().enumerate() {
                let action_el = Self::build_action_button(&t, i, action);
                actions_row = actions_row.child(action_el);
            }

            card = card.child(actions_row);
        } else {
            // Bottom padding when there are no actions
            if has_header || has_body || has_media || has_custom {
                card = card.child(div().h(px(CARD_PADDING)));
            }
        }

        // ── Card click handler ───────────────────────────────────────
        if let Some(handler) = self.on_click {
            card = card.on_mouse_down(MouseButton::Left, handler);
        }

        card
    }

    // ── Internal helpers ─────────────────────────────────────────────

    /// Build an action button element.
    fn build_action_button(theme: &MaterialTheme, index: usize, action: CardAction) -> AnyElement {
        let btn_id = gpui::ElementId::Name(format!("card-action-{index}").into());

        match action.style {
            CardActionStyle::Text => div()
                .id(btn_id)
                .px(px(12.0))
                .py(px(10.0))
                .rounded(px(20.0))
                .text_sm()
                .text_color(rgb(theme.primary))
                .cursor_pointer()
                .child(action.label)
                .on_mouse_down(MouseButton::Left, action.on_click)
                .into_any_element(),

            CardActionStyle::Filled => div()
                .id(btn_id)
                .px(px(24.0))
                .py(px(10.0))
                .rounded(px(20.0))
                .bg(rgb(theme.primary))
                .text_sm()
                .text_color(rgb(theme.on_primary))
                .cursor_pointer()
                .child(action.label)
                .on_mouse_down(MouseButton::Left, action.on_click)
                .into_any_element(),

            CardActionStyle::Outlined => div()
                .id(btn_id)
                .px(px(24.0))
                .py(px(10.0))
                .rounded(px(20.0))
                .border_1()
                .border_color(rgb(theme.outline))
                .text_sm()
                .text_color(rgb(theme.primary))
                .cursor_pointer()
                .child(action.label)
                .on_mouse_down(MouseButton::Left, action.on_click)
                .into_any_element(),

            CardActionStyle::Tonal => div()
                .id(btn_id)
                .px(px(24.0))
                .py(px(10.0))
                .rounded(px(20.0))
                .bg(rgb(theme.secondary_container))
                .text_sm()
                .text_color(rgb(theme.on_secondary_container))
                .cursor_pointer()
                .child(action.label)
                .on_mouse_down(MouseButton::Left, action.on_click)
                .into_any_element(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Legacy compatibility
// ═══════════════════════════════════════════════════════════════════════════════

/// Legacy composite showcase — elevated and outlined cards.
///
/// Kept for backward compatibility with the existing component gallery.
/// New code should use [`Card`] or [`CardBuilder`] directly.
pub fn cards(dark: bool) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);
    let _on_surface = theme.on_surface;
    let _on_surface_variant = theme.on_surface_variant;
    let primary = theme.primary;
    let _outline = theme.outline_variant;

    div()
        .flex()
        .flex_col()
        .gap_3()
        // Elevated card
        .child(
            CardBuilder::new(theme)
                .variant(CardVariant::Elevated)
                .avatar("AB", primary)
                .title("Elevated Card")
                .subtitle("Material Design 3")
                .body(
                    "Cards contain content and actions about a single subject. \
                     Elevated cards have a drop shadow for visual hierarchy.",
                )
                .action_text("Cancel", |_, _, _| {})
                .action_filled("Accept", |_, _, _| {})
                .id("legacy-elevated-card")
                .full_width()
                .build(),
        )
        // Outlined card
        .child(
            CardBuilder::new(theme)
                .variant(CardVariant::Outlined)
                .title("Outlined Card")
                .body(
                    "Outlined cards use a border instead of shadow. \
                     Best for grouping related content without heavy emphasis.",
                )
                .id("legacy-outlined-card")
                .full_width()
                .build(),
        )
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Demo / showcase
// ═══════════════════════════════════════════════════════════════════════════════

/// Renders a comprehensive showcase of all MD3 card variants and the
/// CardBuilder API.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material::card;
///
/// let demo = card::card_demo(true); // dark mode
/// ```
pub fn card_demo(dark: bool) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);

    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        // Filled card
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("FILLED"),
                )
                .child(
                    CardBuilder::new(theme)
                        .variant(CardVariant::Filled)
                        .title("Filled Card")
                        .subtitle("Default variant")
                        .body("Filled cards have the highest visual emphasis with a tinted background.")
                        .action_text("Learn More", |_, _, _| {})
                        .id("demo-filled-card")
                        .full_width()
                        .build(),
                ),
        )
        // Elevated card
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("ELEVATED"),
                )
                .child(
                    CardBuilder::new(theme)
                        .variant(CardVariant::Elevated)
                        .avatar("JD", theme.primary)
                        .title("Jane Doe")
                        .subtitle("2 hours ago")
                        .body("Just finished a great hike in the mountains! The views were absolutely breathtaking.")
                        .action_text("Comment", |_, _, _| {})
                        .action_tonal("Like", |_, _, _| {})
                        .id("demo-elevated-card")
                        .full_width()
                        .build(),
                ),
        )
        // Outlined card
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("OUTLINED"),
                )
                .child(
                    CardBuilder::new(theme)
                        .variant(CardVariant::Outlined)
                        .title("Outlined Card")
                        .body("Outlined cards use a border for visual separation with minimal emphasis.")
                        .action_outlined("Details", |_, _, _| {})
                        .id("demo-outlined-card")
                        .full_width()
                        .build(),
                ),
        )
        // Card with all sections
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("ALL SECTIONS"),
                )
                .child(
                    CardBuilder::new(theme)
                        .variant(CardVariant::Elevated)
                        .avatar("🎵", theme.tertiary_container)
                        .title("Now Playing")
                        .subtitle("Music Player")
                        .body("Bohemian Rhapsody — Queen")
                        .content(
                            // Custom content: a simple progress bar
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.0))
                                .child(
                                    div()
                                        .w_full()
                                        .h(px(4.0))
                                        .rounded(px(2.0))
                                        .bg(rgb(theme.surface_container_highest))
                                        .child(
                                            div()
                                                .w(gpui::relative(0.6))
                                                .h(px(4.0))
                                                .rounded(px(2.0))
                                                .bg(rgb(theme.primary)),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .justify_between()
                                        .text_xs()
                                        .text_color(rgb(theme.on_surface_variant))
                                        .child("3:42")
                                        .child("5:55"),
                                ),
                        )
                        .action_text("⏮ Prev", |_, _, _| {})
                        .action_filled("⏸ Pause", |_, _, _| {})
                        .action_text("Next ⏭", |_, _, _| {})
                        .id("demo-full-card")
                        .full_width()
                        .build(),
                ),
        )
        // Low-level Card container
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("LOW-LEVEL CARD CONTAINER"),
                )
                .child(
                    Card::filled(theme)
                        .full_width()
                        .gap(8.0)
                        .id("demo-low-level-card")
                        .child(
                            div()
                                .text_base()
                                .text_color(rgb(theme.on_surface))
                                .child("Using Card::filled()"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(theme.on_surface_variant))
                                .child(
                                    "This card is built using the low-level Card container. \
                                     You add children directly — no header, body, or action slots.",
                                ),
                        ),
                ),
        )
        // Outlined card with minimal content
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("HORIZONTAL CARD"),
                )
                .child(
                    Card::outlined(theme)
                        .full_width()
                        .horizontal()
                        .gap(16.0)
                        .id("demo-horizontal-card")
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(80.0))
                                .rounded(px(8.0))
                                .bg(rgb(theme.primary_container))
                                .text_3xl()
                                .text_color(rgb(theme.on_primary_container))
                                .child("📚"),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .justify_center()
                                .flex_1()
                                .gap(px(4.0))
                                .child(
                                    div()
                                        .text_base()
                                        .text_color(rgb(theme.on_surface))
                                        .child("Horizontal Layout"),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(theme.on_surface_variant))
                                        .child("Cards can also use a horizontal (row) layout."),
                                ),
                        ),
                ),
        )
        // Clickable card
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("CLICKABLE CARD"),
                )
                .child(
                    CardBuilder::new(theme)
                        .variant(CardVariant::Elevated)
                        .title("Tap me!")
                        .body("Cards with an on_click handler show a pointer cursor and respond to taps.")
                        .on_click(|_, _, _| {
                            // In a real app, this would navigate or trigger an action
                        })
                        .id("demo-clickable-card")
                        .full_width()
                        .build(),
                ),
        )
}
