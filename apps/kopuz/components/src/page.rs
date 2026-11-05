//! The frame every page is set in.

use gpui_kit::component::v_flex;
use gpui_kit::{Div, Styled as _};

/// `size-full px-8 pt-8` at the skin's padding: a page's column, the same on every page; the page adds what follows.
pub fn page() -> Div {
    let padding = crate::skin::arrangement().pages.padding;
    v_flex().size_full().px(padding).pt(padding)
}
