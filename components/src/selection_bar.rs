use dioxus::prelude::*;

#[component]
pub fn SelectionBar(
    count: usize,
    on_add_to_playlist: EventHandler<()>,
    on_delete: EventHandler<()>,
    on_cancel: EventHandler<()>,
    #[props(default = true)] show_delete: bool,
) -> Element {
    if count == 0 {
        return rsx! { "" };
    }

    let delete_text = rust_i18n::t!("delete").to_string();
    let add_to_playlist_text = rust_i18n::t!("add_to_playlist").to_string();

    rsx! {
        div {
            class: "fixed bottom-24 left-1/2 -translate-x-1/2 bg-indigo-500 text-white px-6 py-2.5 rounded-full shadow-2xl flex items-center gap-4 z-50 animate-in fade-in zoom-in duration-200 font-mono",

            span { class: "font-bold text-lg whitespace-nowrap pl-2", "{count} selected" }

            div { class: "w-px h-5 bg-white/20" }

            div { class: "flex items-center gap-4",
                button {
                    class: "hover:opacity-80 transition-opacity flex items-center gap-2 font-medium whitespace-nowrap",
                    onclick: move |_| on_add_to_playlist.call(()),
                    i { class: "fa-solid fa-plus text-sm" }
                    span { class: "hidden sm:inline", "{add_to_playlist_text}" }
                }

                if show_delete {
                    button {
                        class: "hover:opacity-80 transition-opacity flex items-center gap-2 font-medium whitespace-nowrap",
                        onclick: move |_| on_delete.call(()),
                        i { class: "fa-solid fa-trash text-sm" }
                        span { class: "hidden sm:inline", "{delete_text}" }
                    }
                }
            }

            div { class: "w-px h-5 bg-white/20" }

            button {
                class: "hover:opacity-80 transition-opacity flex items-center justify-center pr-2",
                onclick: move |_| on_cancel.call(()),
                i { class: "fa-solid fa-xmark text-lg" }
            }
        }
    }
}
