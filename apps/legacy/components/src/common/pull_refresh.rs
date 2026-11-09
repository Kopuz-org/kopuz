//! Pull down at the top of a list to refresh it, for the touch layouts.

use dioxus::prelude::*;

/// Pull (px) that commits a refresh on release.
const REFRESH_AT: f64 = 110.0;
/// Tallest the indicator grows; past this the pull keeps going but the
/// indicator stays put, like the platform's own.
const INDICATOR_MAX: f64 = 64.0;

/// Wraps a list whose scroller is `scroller_id`. A pull only arms when that
/// scroller is already at its top, so scrolling back up through a long list
/// never refreshes it. Off Android it renders its children and nothing else.
#[component]
pub fn PullToRefresh(
    scroller_id: String,
    refreshing: bool,
    on_refresh: EventHandler<()>,
    #[props(default)] class: String,
    children: Element,
) -> Element {
    let mut swipe = crate::gestures::use_swipe();
    let mut armed = use_signal(|| false);
    let mut past_threshold = use_signal(|| false);

    if !cfg!(target_os = "android") {
        return rsx! {
            div { class: "{class}", {children} }
        };
    }

    let pull = if *armed.read() {
        swipe.pull_down()
    } else {
        0.0
    };
    let indicator = (pull * 0.5).min(INDICATOR_MAX);
    let showing = refreshing || indicator > 0.0;
    let height = if refreshing { 40.0 } else { indicator };
    let ready = pull >= REFRESH_AT;
    let id_for_start = scroller_id.clone();

    rsx! {
        div {
            class: "{class}",
            ontouchstart: move |evt| {
                swipe.start(&evt);
                armed.set(false);
                let id = id_for_start.replace('\\', "\\\\").replace('\'', "\\'");
                spawn(async move {
                    let mut at_top = dioxus::document::eval(&format!(
                        "const el = document.getElementById('{id}'); dioxus.send(el ? el.scrollTop : 0);"
                    ));
                    if let Ok(scroll_top) = at_top.recv::<f64>().await
                        && scroll_top <= 1.0
                    {
                        armed.set(true);
                    }
                });
            },
            ontouchmove: move |evt| {
                swipe.update(&evt);
                let past = *armed.peek() && swipe.pull_down() >= REFRESH_AT;
                if past != *past_threshold.peek() {
                    past_threshold.set(past);
                    if past {
                        crate::haptics::perform(crate::haptics::Haptic::Tick);
                    }
                }
            },
            ontouchend: move |evt| {
                let commit = *armed.peek() && swipe.pull_down() >= REFRESH_AT;
                swipe.finish(&evt);
                armed.set(false);
                past_threshold.set(false);
                if commit && !refreshing {
                    on_refresh.call(());
                }
            },
            ontouchcancel: move |_| {
                swipe.reset();
                armed.set(false);
                past_threshold.set(false);
            },
            if showing {
                div {
                    class: "shrink-0 flex items-center justify-center overflow-hidden",
                    style: if refreshing || indicator == 0.0 {
                        format!("height: {height}px; transition: height 180ms ease-out;")
                    } else {
                        format!("height: {height}px;")
                    },
                    "aria-hidden": "true",
                    i {
                        class: if refreshing {
                            "fa-solid fa-rotate fa-spin text-white/60"
                        } else if ready {
                            "fa-solid fa-rotate text-white"
                        } else {
                            "fa-solid fa-arrow-down text-white/40"
                        },
                        style: if !refreshing { format!("transform: rotate({}deg);", (pull / REFRESH_AT * 180.0).min(180.0)) },
                    }
                }
            }
            {children}
        }
    }
}
