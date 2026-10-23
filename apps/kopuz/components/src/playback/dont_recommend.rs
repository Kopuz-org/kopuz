//! "Don't recommend this" as one button, shared by every player bar.

use dioxus::prelude::*;
use hooks::PlayerController;

/// A circle with a bar through its middle, drawn rather than stacked from two
/// font glyphs: each glyph sits in its own advance width, so the bar never
/// lined up with the circle's center. The stroke matches the weight of Font
/// Awesome's regular circle, so it reads as heavy as the heart beside it.
#[component]
fn NoEntryIcon() -> Element {
    rsx! {
        svg {
            width: "0.875em",
            height: "0.875em",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2.25",
            stroke_linecap: "round",
            "aria-hidden": "true",
            circle { cx: "12", cy: "12", r: "10.5" }
            line {
                x1: "7",
                y1: "12",
                x2: "17",
                y2: "12",
            }
        }
    }
}

/// Renders nothing when the active source takes no such signal.
#[component]
pub fn DontRecommendButton(class: String) -> Element {
    let ctrl = use_context::<PlayerController>();
    let caps = hooks::sources::use_capabilities();
    // Hooks run before the gate; one after it would change the hook count mid-life.
    let track = use_memo(move || ctrl.current_track_snapshot.read().clone());
    let is_favorite = hooks::use_db_queries::use_track_is_favorite(track)();
    if !caps.read().dont_recommend {
        return rsx! {};
    }

    let label = i18n::t("dont_recommend").to_string();
    rsx! {
        button {
            class: "{class} disabled:opacity-40 disabled:cursor-not-allowed",
            // Like and dislike are one setting on the remote, so both at once would undo the heart.
            disabled: is_favorite,
            title: "{label}",
            "aria-label": "{label}",
            onclick: move |_| hooks::recommendations::dont_recommend(ctrl),
            NoEntryIcon {}
        }
    }
}
