//! The source switcher: Local + every configured server as a uniform list, pick
//! one to make it active. Replaces the old binary Local⇄Server toggle — no
//! local-vs-server branching, and it reaches any number of servers.
//!
//! A compact trigger (the active source's icon + name) opens a glassy popover
//! that scrolls past ~8 sources, so the control stays the same neat size whether
//! you have one source or a dozen — no cramped chip row.

use config::{AppConfig, MusicService, Source};
use dioxus::prelude::*;

/// One selectable source: its key, display label, and icon class.
fn entries(config: &AppConfig) -> Vec<(Source, String, &'static str)> {
    let mut v = vec![(
        Source::Local,
        i18n::t("local").to_string(),
        "fa-solid fa-hard-drive",
    )];
    for s in &config.servers {
        v.push((
            Source::Server(s.id.clone()),
            s.name.clone(),
            service_icon(s.service),
        ));
    }
    v
}

fn service_icon(service: MusicService) -> &'static str {
    match service {
        MusicService::YtMusic => "fa-brands fa-youtube",
        MusicService::SoundCloud => "fa-brands fa-soundcloud",
        _ => "fa-solid fa-server",
    }
}

#[component]
pub fn SourceSwitcher(
    config: Signal<AppConfig>,
    #[props(default = false)] collapsed: bool,
) -> Element {
    let mut open = use_signal(|| false);
    let sources = entries(&config.read());
    let active = config.read().active_source.clone();
    let (active_label, active_icon) = sources
        .iter()
        .find(|(s, _, _)| *s == active)
        .map(|(_, l, i)| (l.clone(), *i))
        .unwrap_or_else(|| (i18n::t("local").to_string(), "fa-solid fa-hard-drive"));

    // The popover: an invisible full-viewport catcher closes it on an outside
    // click; the panel floats above the nav (below the trigger, or beside it when
    // the sidebar is collapsed) and scrolls once there are many sources.
    let panel_pos = if collapsed {
        "absolute left-full top-0 ml-2 w-52"
    } else {
        "absolute left-3 right-3 top-full mt-1.5"
    };
    let menu = rsx! {
        div { class: "fixed inset-0 z-40", onclick: move |_| open.set(false) }
        div {
            class: "{panel_pos} z-50 max-h-[60vh] overflow-y-auto rounded-xl border border-white/10 bg-[#17171d]/95 backdrop-blur-xl shadow-2xl shadow-black/60 p-1 origin-top",
            for (src , label , icon) in sources {
                {
                    let is_active = src == active;
                    rsx! {
                        button {
                            key: "{src.as_str()}",
                            class: if is_active { "w-full flex items-center gap-2.5 px-2.5 py-2 rounded-lg bg-indigo-500/15 text-left" } else { "w-full flex items-center gap-2.5 px-2.5 py-2 rounded-lg text-left hover:bg-white/[0.06] transition-colors" },
                            onclick: move |_| {
                                crate::shared::set_active_source(config, src.clone());
                                open.set(false);
                            },
                            i { class: if is_active { "{icon} text-[11px] w-4 text-center text-indigo-400" } else { "{icon} text-[11px] w-4 text-center text-white/45" } }
                            span { class: if is_active { "flex-1 text-xs font-semibold text-indigo-200 truncate" } else { "flex-1 text-xs font-medium text-white/80 truncate" }, "{label}" }
                            if is_active {
                                i { class: "fa-solid fa-check text-[9px] text-indigo-400" }
                            }
                        }
                    }
                }
            }
        }
    };

    if collapsed {
        rsx! {
            div { class: "relative flex justify-center py-3 border-b border-white/5",
                button {
                    class: "w-9 h-9 flex items-center justify-center rounded-lg bg-white/[0.04] hover:bg-white/[0.08] border border-white/10 transition-colors",
                    title: "{active_label}",
                    onclick: move |_| open.set(!open()),
                    i { class: "{active_icon} text-xs text-indigo-400" }
                }
                if open() {
                    {menu}
                }
            }
        }
    } else {
        rsx! {
            div { class: "relative px-3 pt-3 pb-2 border-b border-white/5",
                button {
                    class: "w-full flex items-center gap-2.5 px-3 py-2 rounded-lg bg-white/[0.04] hover:bg-white/[0.08] border border-white/10 transition-colors",
                    onclick: move |_| open.set(!open()),
                    i { class: "{active_icon} text-[11px] w-4 text-center text-indigo-400" }
                    span { class: "flex-1 text-left text-xs font-semibold text-white/85 truncate", "{active_label}" }
                    i { class: if open() { "fa-solid fa-chevron-down text-[9px] text-white/40 rotate-180 transition-transform" } else { "fa-solid fa-chevron-down text-[9px] text-white/40 transition-transform" } }
                }
                if open() {
                    {menu}
                }
            }
        }
    }
}
