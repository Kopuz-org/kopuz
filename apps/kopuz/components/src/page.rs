//! The frame every page is set in.

use gpui_kit::component::v_flex;
use gpui_kit::{Div, Styled as _};

/// `size-full px-8 pt-8`: a page's column, its padding the same on every page; the page adds what follows.
pub fn page() -> Div {
    v_flex().size_full().px_8().pt_8()
}
