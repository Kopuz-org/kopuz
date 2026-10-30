//! The browse surface for a remote catalog.

pub mod browse;
pub mod discover;
pub mod search;
mod shelves;

use dioxus::prelude::*;

/// Calls `load_more` while the element with id `sentinel` is within 600px of
/// the viewport. `loaded` must change whenever a page of rows lands: the
/// observer only fires on intersection changes, so it is rebuilt after each
/// load to keep fetching when the new rows still leave the sentinel in view.
pub(crate) fn use_infinite_scroll(
    sentinel: &'static str,
    loaded: impl Fn() -> usize + 'static,
    mut load_more: impl FnMut() + 'static,
) {
    use_hook(move || {
        let mut eval = document::eval(&format!(
            r#"
            const scrolls = (window.__kopuzScrolls ??= {{}});
            scrolls['{sentinel}']?.dispose();
            let obs = null;
            const dispose = () => obs?.disconnect();
            const watch = () => {{
                dispose();
                const el = document.getElementById('{sentinel}');
                if (!el) return;
                obs = new IntersectionObserver((entries) => {{
                    if (entries.some((e) => e.isIntersecting)) dioxus.send('load-more');
                }}, {{ rootMargin: '600px' }});
                obs.observe(el);
            }};
            scrolls['{sentinel}'] = {{ watch, dispose }};
            watch();
            "#
        ));
        spawn(async move {
            while eval.recv::<serde_json::Value>().await.is_ok() {
                load_more();
            }
        });
    });

    use_effect(move || {
        loaded();
        let _ = document::eval(&format!("window.__kopuzScrolls?.['{sentinel}']?.watch()"));
    });

    use_drop(move || {
        let _ = document::eval(&format!(
            "window.__kopuzScrolls?.['{sentinel}']?.dispose(); delete window.__kopuzScrolls?.['{sentinel}'];"
        ));
    });
}
