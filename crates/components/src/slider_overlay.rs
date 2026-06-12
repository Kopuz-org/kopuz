use dioxus::prelude::*;

/// Click-to-set fallback for sliders under the blitz renderer, where
/// `input type="range"` is inert and neither element-relative coordinates
/// nor element rects are available. A strip of invisible equal-width
/// segments over the track; each segment maps to its own fraction, so no
/// coordinate math is needed. Extends a few px past the (typically 4px)
/// track so it's actually clickable.
#[component]
pub fn BlitzSliderOverlay(segments: usize, on_set: EventHandler<f64>) -> Element {
    let n = segments.max(2);
    rsx! {
        div {
            class: "absolute inset-x-0 z-10 flex cursor-pointer",
            style: "top: -6px; bottom: -6px;",
            for i in 0..n {
                div {
                    class: "flex-1 h-full",
                    onclick: move |evt| {
                        evt.stop_propagation();
                        on_set.call(i as f64 / (n - 1) as f64);
                    },
                }
            }
        }
    }
}
