//! "Don't recommend this" as one button, shared by every player bar.

use dioxus::prelude::*;
use hooks::PlayerController;

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
            // Inline, not a Tailwind class: purged from the stylesheet it silently falls back to 1em.
            span {
                class: "relative inline-flex",
                style: "font-size: 0.875em",
                "aria-hidden": "true",
                // Stacked glyphs, not a border: a border reads lighter than the heart beside it.
                i { class: "fa-regular fa-circle" }
                i {
                    class: "fa-solid fa-minus absolute inset-0 flex items-center justify-center",
                    style: "transform: scale(0.5, 0.76)",
                }
            }
        }
    }
}
