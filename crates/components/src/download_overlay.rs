use ::server::{DownloadItem, DownloadProgress, DownloadQueue, DownloadStatus};
use dioxus::prelude::*;

fn compute_eta(
    in_flight: &[DownloadItem],
    bytes_done_session: u64,
    session_elapsed_secs: f64,
) -> Option<u64> {
    if session_elapsed_secs < 0.5 || bytes_done_session == 0 {
        return None;
    }
    let bps = bytes_done_session as f64 / session_elapsed_secs;
    if bps <= 0.0 {
        return None;
    }
    let remaining: u64 = in_flight
        .iter()
        .map(|i| {
            if i.bytes_total > 0 {
                i.bytes_total.saturating_sub(i.bytes_done)
            } else {
                8_000_000
            }
        })
        .sum();
    Some((remaining as f64 / bps) as u64)
}

fn fmt_eta(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else {
        format!("{}m {}s", secs / 60, secs % 60)
    }
}

fn fmt_bytes(b: u64) -> String {
    if b < 1024 {
        format!("{} B", b)
    } else if b < 1024 * 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{:.1} MB", b as f64 / (1024.0 * 1024.0))
    }
}

#[component]
pub fn DownloadOverlay(mut queue: Signal<DownloadQueue>) -> Element {
    let progress = use_context::<Signal<DownloadProgress>>();
    let mut collapsed = use_signal(|| false);

    let q = queue.read();

    let has_items = !q.items.is_empty();
    if !has_items {
        return rsx! {};
    }

    let is_active = q.is_active();
    let done = q.done_count();
    let total = q.total_non_failed();
    let in_flight_skeletons: Vec<DownloadItem> = q
        .items
        .iter()
        .filter(|i| matches!(i.status, DownloadStatus::Downloading))
        .cloned()
        .collect();
    let failed_count = q
        .items
        .iter()
        .filter(|i| matches!(i.status, DownloadStatus::Failed))
        .count();
    drop(q);

    let prog = progress.read();
    let in_flight: Vec<DownloadItem> = in_flight_skeletons
        .into_iter()
        .map(|mut item| {
            if let Some(&bytes) = prog.per_item.get(&item.id) {
                item.bytes_done = bytes;
            }
            item
        })
        .collect();
    let bytes_done_session = prog.bytes_done_session;
    let session_elapsed_secs = prog.session_elapsed_secs;
    drop(prog);

    let eta = compute_eta(&in_flight, bytes_done_session, session_elapsed_secs);

    let title_text = if is_active {
        format!("Downloading {} / {}", done, total)
    } else {
        format!("Done — {} downloaded", done)
    };

    rsx! {
        div {
            class: "fixed top-16 right-4 z-50 w-72 rounded-xl bg-neutral-900/95 border border-white/10 shadow-2xl backdrop-blur-md overflow-hidden",

            div {
                class: "flex items-center justify-between px-4 py-3 border-b border-white/5 cursor-pointer select-none",
                onclick: move |_| { let v = *collapsed.read(); collapsed.set(!v); },

                div { class: "flex items-center gap-2",
                    if is_active {
                        i { class: "fa-solid fa-arrow-down-to-bracket text-indigo-400 text-sm animate-pulse" }
                    } else {
                        i { class: "fa-solid fa-circle-check text-emerald-400 text-sm" }
                    }
                    span { class: "text-sm font-semibold text-white", "{title_text}" }
                }

                div { class: "flex items-center gap-2",
                    if is_active {
                        button {
                            class: "text-red-400/70 hover:text-red-400 transition-colors text-xs px-2 py-0.5 rounded bg-red-500/10 hover:bg-red-500/20",
                            onclick: move |evt| {
                                evt.stop_propagation();
                                queue.write().cancel_all();
                            },
                            "Stop"
                        }
                    } else {
                        button {
                            class: "text-white/40 hover:text-white/80 transition-colors text-xs px-2 py-0.5 rounded bg-white/5 hover:bg-white/10",
                            onclick: move |evt| {
                                evt.stop_propagation();
                                queue.write().dismiss();
                            },
                            "Clear"
                        }
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

                    if !in_flight.is_empty() {
                        div { class: "space-y-2.5",
                            for item in in_flight.iter().cloned() {
                                div {
                                    key: "{item.id}",
                                    class: "space-y-1.5",
                                    div { class: "flex items-center justify-between",
                                        div { class: "min-w-0 flex-1",
                                            p { class: "text-sm font-medium text-white truncate", "{item.title}" }
                                            p { class: "text-xs text-slate-400 truncate", "{item.artist}" }
                                        }
                                        if item.bytes_total > 0 {
                                            span { class: "text-xs text-slate-500 ml-2 shrink-0",
                                                "{fmt_bytes(item.bytes_done)} / {fmt_bytes(item.bytes_total)}"
                                            }
                                        } else {
                                            span { class: "text-xs text-slate-500 ml-2 shrink-0",
                                                "{fmt_bytes(item.bytes_done)}"
                                            }
                                        }
                                    }
                                    div { class: "w-full h-1.5 bg-white/10 rounded-full overflow-hidden",
                                        div {
                                            class: "h-full bg-indigo-500 rounded-full transition-all duration-300",
                                            style: if item.bytes_total > 0 {
                                                format!("width: {:.1}%", item.bytes_done as f64 / item.bytes_total as f64 * 100.0)
                                            } else {
                                                format!("width: {:.1}%", (item.bytes_done as f64 / 8_000_000.0 * 100.0).max(5.0).min(95.0))
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if !is_active {
                        div { class: "flex items-center gap-2 text-emerald-400",
                            i { class: "fa-solid fa-check text-sm" }
                            span { class: "text-sm", "All downloads complete" }
                        }
                    }

                    if is_active {
                        div { class: "flex items-center justify-between text-xs text-slate-500",
                            if let Some(secs) = eta {
                                span { "~{fmt_eta(secs)} remaining" }
                            } else {
                                span { "Calculating..." }
                            }
                            {
                                let q = queue.read();
                                let queued = q.items.iter().filter(|i| matches!(i.status, DownloadStatus::Queued)).count();
                                drop(q);
                                if queued > 0 {
                                    rsx! { span { "{queued} queued" } }
                                } else {
                                    rsx! {}
                                }
                            }
                        }
                    }

                    if failed_count > 0 {
                        div { class: "flex items-center gap-1.5 text-xs text-red-400",
                            i { class: "fa-solid fa-triangle-exclamation" }
                            span { "{failed_count} failed" }
                        }
                    }

                    {
                        let q = queue.read();
                        let visible: Vec<_> = q.items.iter()
                            .filter(|i| !matches!(i.status, DownloadStatus::Downloading))
                            .cloned()
                            .collect();
                        drop(q);

                        if !visible.is_empty() {
                            rsx! {
                                div {
                                    class: "space-y-1 border-t border-white/5 pt-2 max-h-48 overflow-y-auto",
                                    for item in visible {
                                        div {
                                            key: "{item.id}",
                                            class: "flex items-center gap-2 text-xs",
                                            match item.status {
                                                DownloadStatus::Done => rsx! {
                                                    i { class: "fa-solid fa-check text-emerald-400 w-3 shrink-0" }
                                                },
                                                DownloadStatus::Failed => rsx! {
                                                    i { class: "fa-solid fa-xmark text-red-400 w-3 shrink-0" }
                                                },
                                                DownloadStatus::Queued => rsx! {
                                                    i { class: "fa-regular fa-clock text-slate-500 w-3 shrink-0" }
                                                },
                                                _ => rsx! {}
                                            }
                                            span {
                                                class: format!(
                                                    "truncate {}",
                                                    match item.status {
                                                        DownloadStatus::Done => "text-slate-400",
                                                        DownloadStatus::Failed => "text-red-400/70",
                                                        _ => "text-slate-500",
                                                    }
                                                ),
                                                if item.title.is_empty() {
                                                    "Track {&item.id[..item.id.len().min(8)]}"
                                                } else {
                                                    "{item.title}"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            rsx! {}
                        }
                    }
                }
            }
        }
    }
}
