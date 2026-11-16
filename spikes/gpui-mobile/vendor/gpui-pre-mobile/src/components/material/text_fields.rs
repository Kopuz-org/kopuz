//! Material text field components — outlined, filled, and error-state inputs.
//!
//! Text fields allow users to enter text into a UI. Material Design 3
//! defines two main styles:
//!
//! - **Outlined** — a text field with a visible border and floating label
//! - **Filled** — a text field with a tinted background and bottom indicator
//!
//! Both styles support an error state with a red border/indicator and an
//! error helper message below the field.
//!
//! The composite [`text_fields`] function renders a showcase of all three
//! states (outlined, filled, error) for demonstration purposes.

use gpui::{div, prelude::*, px, rgb};

/// Renders a complete Material Design text fields showcase with three variants:
///
/// 1. **Outlined** — "Email" field with a primary-coloured border and label
/// 2. **Filled** — "Username" field with a tinted background and bottom bar
/// 3. **Error** — "Password" field in error state with red border and helper text
///
/// Colours adapt to dark / light mode using the MD3 colour system.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let fields = material::text_fields(true); // dark mode
/// ```
pub fn text_fields(dark: bool) -> impl IntoElement {
    let on_surface = if dark { 0xe6e1e5_u32 } else { 0x1c1b1f };
    let placeholder = if dark { 0x938f99_u32 } else { 0x79747e };
    let filled_bg = if dark { 0x36343b_u32 } else { 0xe7e0ec };
    let primary = if dark { 0xd0bcff_u32 } else { 0x6750a4 };
    let error = if dark { 0xf2b8b5_u32 } else { 0xb3261e };

    div()
        .flex()
        .flex_col()
        .gap_3()
        // Outlined text field
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(rgb(primary)).child("Email"))
                .child(
                    div()
                        .px_4()
                        .py_3()
                        .rounded(px(4.0))
                        .border_1()
                        .border_color(rgb(primary))
                        .text_base()
                        .text_color(rgb(on_surface))
                        .child("user@example.com"),
                ),
        )
        // Filled text field
        .child(
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_4()
                        .pt_2()
                        .pb_3()
                        .rounded_tl(px(4.0))
                        .rounded_tr(px(4.0))
                        .bg(rgb(filled_bg))
                        .child(div().text_xs().text_color(rgb(primary)).child("Username"))
                        .child(
                            div()
                                .text_base()
                                .text_color(rgb(on_surface))
                                .child("john_doe"),
                        ),
                )
                .child(div().w_full().h(px(2.0)).bg(rgb(primary))),
        )
        // Error state
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(rgb(error)).child("Password"))
                .child(
                    div()
                        .px_4()
                        .py_3()
                        .rounded(px(4.0))
                        .border_1()
                        .border_color(rgb(error))
                        .text_base()
                        .text_color(rgb(placeholder))
                        .child("••••••"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(error))
                        .child("⚠ Password must be at least 8 characters"),
                ),
        )
}
