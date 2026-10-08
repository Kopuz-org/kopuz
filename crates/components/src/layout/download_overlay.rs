//! What the daemon is fetching for offline playback.
//!
//! The overlay used to read a queue the UI process owned, down to bytes
//! transferred per item. The download is a daemon job now, and a job reports
//! which track it is on out of how many -- so this shows that, plus the
//! titles, and no longer a byte-level speed the daemon does not measure.

use dioxus::prelude::*;

/// One row: what to call it, and where it has got to.
struct Row {
    key: String,
    title: String,
    artist: String,
    state: api::DownloadItemState,
}

#[component]
pub fn DownloadOverlay() -> Element {
    let api = hooks::use_api();
    let downloads = hooks::downloads::use_downloads();
    let mut collapsed = use_signal(|| false);

    // The batch reports keys; the titles come from the library, so a row says
    // what is downloading rather than an opaque id.
    let statuses = use_resource(move || {
        let api = api.clone();
        let running = downloads.read().running;
        async move {
            if !running {
                return Vec::new();
            }
            let items = api.download_statuses().await.unwrap_or_default();
            let keys: Vec<String> = items.iter().map(|item| item.key.clone()).collect();
            let known = api.tracks_by_keys(keys).await.unwrap_or_default();
            items
                .into_iter()
                .map(|item| {
                    let track = known.iter().find(|track| track.key == item.key);
                    Row {
                        title: track
                            .map(|track| track.title.clone())
                            .filter(|title| !title.is_empty())
                            .unwrap_or_else(|| item.key.clone()),
                        artist: track.map(|track| track.artist.clone()).unwrap_or_default(),
                        key: item.key,
                        state: item.state,
                    }
                })
                .collect()
        }
    });

    let view = downloads.read().clone();
    if !view.running {
        return rsx! {};
    }
    let rows = statuses.read();
    let rows = rows.as_deref().unwrap_or_default();
    let failed = rows
        .iter()
        .filter(|row| row.state == api::DownloadItemState::Failed)
        .count();
    let title_text = format!("Downloading {} / {}", view.done, view.total.max(view.done));

    rsx! {
        div {
            class: "fixed top-16 right-4 z-50 w-72 rounded-xl bg-neutral-900/95 border border-white/10 shadow-2xl backdrop-blur-md overflow-hidden",

            div {
                class: "flex items-center justify-between px-4 py-3 border-b border-white/5 cursor-pointer select-none",
                onclick: move |_| { let value = *collapsed.read(); collapsed.set(!value); },

                div { class: "flex items-center gap-2",
                    i { class: "fa-solid fa-arrow-down-to-bracket text-indigo-400 text-sm animate-pulse" }
                    span { class: "text-sm font-semibold text-white", "{title_text}" }
                }

                div { class: "flex items-center gap-2",
                    button {
                        class: "text-red-400/70 hover:text-red-400 transition-colors text-xs px-2 py-0.5 rounded bg-red-500/10 hover:bg-red-500/20",
                        onclick: move |evt| {
                            evt.stop_propagation();
                            hooks::downloads::cancel();
                        },
                        "Stop"
                    }
                    i {
                        class: format!(
                            "fa-solid {} text-white/40 text-xs transition-transform",
                            if *collapsed.read() { "fa-chevron-down" } else { "fa-chevron-up" }
                        )
                    }
                }
            }

            if !*collapsed.read() {
                div { class: "px-4 py-3 space-y-3",
                    div { class: "w-full h-1.5 bg-white/10 rounded-full overflow-hidden",
                        div {
                            class: "h-full bg-indigo-500 rounded-full transition-all duration-300",
                            style: format!(
                                "width: {:.1}%",
                                if view.total == 0 {
                                    0.0
                                } else {
                                    view.done as f64 / view.total as f64 * 100.0
                                }
                            ),
                        }
                    }

                    if failed > 0 {
                        div { class: "flex items-center gap-1.5 text-xs text-red-400",
                            i { class: "fa-solid fa-triangle-exclamation" }
                            span { "{failed} failed" }
                        }
                    }

                    if !rows.is_empty() {
                        div {
                            class: "space-y-1 border-t border-white/5 pt-2 max-h-48 overflow-y-auto",
                            for row in rows.iter() {
                                div {
                                    key: "{row.key}",
                                    class: "flex items-center gap-2 text-xs",
                                    match row.state {
                                        api::DownloadItemState::Downloading => rsx! {
                                            i { class: "fa-solid fa-arrow-down text-indigo-400 w-3 shrink-0" }
                                        },
                                        api::DownloadItemState::Failed => rsx! {
                                            i { class: "fa-solid fa-xmark text-red-400 w-3 shrink-0" }
                                        },
                                        api::DownloadItemState::Queued => rsx! {
                                            i { class: "fa-regular fa-clock text-slate-500 w-3 shrink-0" }
                                        },
                                    }
                                    div { class: "min-w-0 flex-1",
                                        p {
                                            class: format!(
                                                "truncate {}",
                                                match row.state {
                                                    api::DownloadItemState::Downloading => "text-white",
                                                    api::DownloadItemState::Failed => "text-red-400/70",
                                                    api::DownloadItemState::Queued => "text-slate-500",
                                                }
                                            ),
                                            "{row.title}"
                                        }
                                        if !row.artist.is_empty() {
                                            p { class: "text-slate-500 truncate", "{row.artist}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
