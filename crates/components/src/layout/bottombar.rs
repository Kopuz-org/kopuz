use config::UiStyle;
use dioxus::prelude::*;

use crate::normal::bottombar::BottombarNormal;
use crate::queue_drag::{install_native_artwork_drag_prevention, set_queue_drag_enabled};
use crate::vaxry::bottombar::BottombarVaxry;

#[component]
pub fn Bottombar(
    is_fullscreen: Signal<bool>,
    persisted_volume: Signal<f32>,
    is_rightbar_open: Signal<bool>,
    is_devices_open: Signal<bool>,
) -> Element {
    let config = use_context::<hooks::PlayerController>().config;
    use_effect(move || {
        install_native_artwork_drag_prevention();
    });

    let c = *is_rightbar_open.read();
    set_queue_drag_enabled(c);

    match config.read().ui_style {
        UiStyle::Normal | UiStyle::Material3 => rsx! {
            BottombarNormal {
                is_fullscreen,
                persisted_volume, is_rightbar_open, is_devices_open,
            }
        },
        UiStyle::Vaxry => rsx! {
            BottombarVaxry {
                is_fullscreen,
                persisted_volume, is_rightbar_open, is_devices_open,
            }
        },
    }
}
