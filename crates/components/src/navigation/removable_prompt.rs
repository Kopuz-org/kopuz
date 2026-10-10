use std::collections::HashSet;

use dioxus::prelude::*;

#[component]
pub fn RemovablePrompt() -> Element {
    let sources = hooks::sources::use_sources();
    let config = use_context::<Signal<config::AppConfig>>();
    let navigation = use_context::<crate::NavigationController>();
    let mut dismissed = use_signal(HashSet::<String>::new);
    let mut busy = use_signal(|| false);
    use_effect(move || {
        let present: HashSet<_> = sources
            .read()
            .iter()
            .flatten()
            .filter(|source| source.temporary)
            .map(|source| source.id.clone())
            .collect();
        if dismissed.peek().iter().any(|id| !present.contains(id)) {
            dismissed.write().retain(|id| present.contains(id));
        }
    });
    let pending = sources
        .read()
        .iter()
        .flatten()
        .find(|source| source.temporary && !source.active && !dismissed.read().contains(&source.id))
        .cloned();
    let Some(source) = pending else {
        return rsx! {};
    };
    let ignore = source.id.clone();
    rsx! {
        div { class: "fixed inset-0 z-50 bg-black/70 flex items-center justify-center p-6 text-white",
            div { class: "bg-stone-900 border border-white/15 rounded-xl p-6 w-full max-w-md flex flex-col gap-4",
                role: "dialog",
                "aria-modal": "true",
                "aria-label": i18n::t("removable_found"),
                onkeydown: move |event| {
                    if event.key() == Key::Escape && !busy() { dismissed.write().insert(ignore.clone()); }
                },
                h2 { class: "text-lg font-semibold", "{i18n::t(\"removable_found\")}" }
                p { class: "font-medium", "{source.name}" }
                if let Some(detail) = source.detail { p { class: "text-sm text-slate-400", "{detail}" } }
                p { class: "text-sm text-slate-300", "{i18n::t(\"removable_listen_help\")}" }
                div { class: "flex justify-end gap-3",
                    button {
                        class: "px-3 py-2 rounded hover:bg-white/10",
                        disabled: busy(),
                        onclick: {
                            let id = source.id.clone();
                            move |_| { dismissed.write().insert(id.clone()); }
                        },
                        "{i18n::t(\"removable_later\")}"
                    }
                    button {
                        class: "px-3 py-2 rounded bg-white text-black disabled:opacity-40",
                        autofocus: true,
                        disabled: busy(),
                        onclick: move |_| {
                            let id = source.id.clone();
                            busy.set(true);
                            spawn(async move {
                                if hooks::source_switch::apply_source_switch(config, id.clone()).await {
                                    dismissed.write().insert(id.clone());
                                    let api = hooks::api::consume_api();
                                    // Wait until the session has applied the switch before materializing a queue.
                                    let _ = api.queue_snapshot().await;
                                    let album = api.albums(api::Page { offset: 0, limit: 1 }).await.ok().and_then(|page| page.albums.into_iter().next());
                                    let still_active = api.sources().await.unwrap_or_default().iter().any(|source| source.id == id && source.active);
                                    if still_active && let Some(album) = album {
                                        navigation.navigate_to_album(album.id.clone());
                                        if let Err(error) = api.set_queue(api::SetQueueRequest {
                                            mode: api::QueueMode::Replace,
                                            context: api::QueueContext::Album { id: album.id },
                                            start_index: Some(0),
                                            shuffle: Some(false),
                                        }).await { hooks::toast::toast_error(&error.to_string()); }
                                    }
                                }
                                busy.set(false);
                            });
                        },
                        "{i18n::t(\"removable_listen\")}"
                    }
                }
            }
        }
    }
}
