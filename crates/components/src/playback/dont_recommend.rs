//! "Don't recommend this" as one button, shared by every player bar.
//!
//! It sits next to the heart and is its negative: tell the active source to
//! stop surfacing what is playing, then move on. The capability gate, the
//! label and the mark live here so the three bars cannot drift apart on what
//! the button means -- each passes only its own chrome, the way each already
//! styles its own heart.

use dioxus::prelude::*;
use hooks::PlayerController;

/// Renders nothing when the active source takes no such signal, so a bar can
/// place it unconditionally. `class` is the button's own chrome; the mark
/// takes its colour from that through `currentColor` and its size from the
/// font size, so a bar sets both for the mark exactly as it does for its
/// heart -- `text-xs` shrinks the two together.
#[component]
pub fn DontRecommendButton(class: String) -> Element {
    let ctrl = use_context::<PlayerController>();
    let caps = hooks::sources::use_capabilities();
    // Every hook runs before the capability gate below: the gate flips when the
    // active source changes, and a hook after it would change this component's
    // hook count mid-life.
    let track = use_memo(move || ctrl.current_track_snapshot.read().clone());
    let is_favorite = hooks::use_db_queries::use_track_is_favorite(track)();
    if !caps.read().dont_recommend {
        return rsx! {};
    }

    let label = i18n::t("dont_recommend").to_string();
    rsx! {
        button {
            class: "{class} disabled:opacity-40 disabled:cursor-not-allowed",
            // A liked song and a rejected one are the same setting on the
            // remote, so offering both at once would only ever mean undoing
            // the heart by surprise. Unheart it first.
            disabled: is_favorite,
            title: "{label}",
            "aria-label": "{label}",
            onclick: move |_| hooks::recommendations::dont_recommend(ctrl),
            // Two glyphs, not CSS. Font Awesome Free has no outline
            // circle-minus (its solid one is a filled disc with the minus
            // knocked out -- the inverse of this button), so the mark is
            // `circle` from the regular face with `minus` from the solid one
            // laid over it.
            //
            // Stacking glyphs rather than drawing a bordered box is what makes
            // this match the heart beside it. A CSS border goes through the
            // path rasteriser -- unhinted, its ink smeared across whatever
            // pixels the fractional width lands on -- while a glyph goes
            // through the text rasteriser, which snaps stems to the pixel grid.
            // At identical nominal widths the border still reads lighter and
            // softer than the glyph, which no amount of tuning the width fixes.
            //
            // The two glyphs need no centring maths: FA inks `circle` and
            // `minus` on one optical axis, both centred 0.3762em above the
            // baseline, so at a shared font size they land concentric. The
            // minus is scaled because at full size it spans 0.815em, all but
            // touching the ring's 0.81em interior -- x0.5 sets its length, and
            // y0.76 takes its 0.125em bar down to the ring's own 0.095em
            // stroke so the mark reads as one weight.
            // 0.92em because the heart inks 1em wide by 0.855em tall, and a
            // circle reads level with a shape that wide and that tall at the
            // geometric mean of the two (0.925). Sizing the ring to either
            // extent alone makes it the larger or the smaller of the pair.
            //
            // The circle is left in normal flow rather than absolutely placed
            // in a fixed box, so this wrapper is sized and positioned by the
            // glyph's own line box -- the same thing that positions the heart.
            // A fixed box is a different layout primitive: it ignores
            // line-height and font metrics, so it drifts against the heart by
            // however much the engine's idea of a line box differs from the
            // number we hard-coded. Only the minus is overlaid.
            //
            // `inline-flex`, not `inline-block`, because an inline-block's line
            // box carries a strut from the inherited line-height: measured at a
            // 96px font it made this wrapper 103px tall around an 88.3px glyph.
            // That rides the ring up against the heart AND, since the minus is
            // laid over the wrapper, centres the bar on the strut's midline
            // rather than the ring's. A flex container has no strut, so the
            // wrapper is exactly the glyph and both faults go together.
            span {
                class: "relative inline-flex text-[0.92em]",
                "aria-hidden": "true",
                i { class: "fa-regular fa-circle" }
                i {
                    class: "fa-solid fa-minus absolute inset-0 flex items-center justify-center",
                    style: "transform: scale(0.5, 0.76)",
                }
            }
        }
    }
}
