//! The source switcher: Local + every configured server as a uniform list, pick
//! one to make it active. Replaces the old binary Local⇄Server toggle — no
//! local-vs-server branching, and it reaches any number of servers (the old
//! toggle could only flip between Local and "the" server).

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
    let sources = entries(&config.read());
    let active = config.read().active_source.clone();

    if collapsed {
        rsx! {
            div { class: "flex flex-col items-center gap-1 py-3 border-b border-white/5",
                for (src , _label , icon) in sources {
                    {
                        let is_active = src == active;
                        rsx! {
                            button {
                                key: "{src.as_str()}",
                                class: if is_active { "text-[10px] font-bold py-1" } else { "text-[10px] font-bold py-1 opacity-30" },
                                style: if is_active { "color: var(--color-indigo-500);" } else { "" },
                                onclick: move |_| crate::shared::set_active_source(config, src.clone()),
                                i { class: "{icon} text-xs" }
                            }
                        }
                    }
                }
            }
        }
    } else {
        rsx! {
            div { class: "px-3 pt-3 pb-2 border-b border-white/5",
                div { class: "flex flex-wrap gap-1 text-[11px] font-bold",
                    for (src , label , _icon) in sources {
                        {
                            let is_active = src == active;
                            rsx! {
                                button {
                                    key: "{src.as_str()}",
                                    class: "flex-1 min-w-[64px] py-1.5 px-2 rounded-lg transition-colors truncate",
                                    style: if is_active { "background: color-mix(in oklab, var(--color-indigo-500) 20%, transparent); color: var(--color-indigo-500);" } else { "color: rgba(255,255,255,0.3);" },
                                    onclick: move |_| crate::shared::set_active_source(config, src.clone()),
                                    "{label.to_uppercase()}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
