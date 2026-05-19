use dioxus::prelude::*;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

#[component]
pub fn SearchBar(search_query: Signal<String>) -> Element {
    let debounce_gen = use_hook(|| Arc::new(AtomicU64::new(0))).clone();

    rsx! {
        div {
            class: "relative max-w-2xl mb-8",
            i { class: "fa-solid fa-magnifying-glass absolute left-4 top-1/2 -translate-y-1/2 text-slate-500" }
            input {
                r#type: "text",
                placeholder: "{i18n::t(\"search_placeholder\")}",
                class: "w-full bg-white/5 border border-white/10 rounded-full py-3 pl-12 pr-4 text-white focus:outline-none focus:border-white/20 transition-colors",
                oninput: move |evt| {
                    let value = evt.value();
                    let tick = debounce_gen.fetch_add(1, Ordering::Relaxed) + 1;
                    let debounce_gen = debounce_gen.clone();
                    spawn(async move {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        if debounce_gen.load(Ordering::Relaxed) == tick {
                            search_query.set(value);
                        }
                    });
                },
                onkeydown: move |e| e.stop_propagation()
            }
        }
    }
}
