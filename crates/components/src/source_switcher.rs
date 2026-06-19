//! The source switcher: Local + every configured server as a uniform list, pick
//! one to make it active. Replaces the old binary Local⇄Server toggle — no
//! local-vs-server branching, and it reaches any number of servers.
//!
//! A compact trigger (the active source's brand-tinted "jack" + name) opens a
//! glassy popover that springs in with a staggered reveal and scrolls past ~8
//! sources, so the control stays neat with one source or a dozen. Each source
//! carries its service's accent colour (a CSS `--accent` var the styles read);
//! the active row glows with it. Mono micro-labels use the app's JetBrains Mono.

use config::{AppConfig, MusicService, Source};
use dioxus::prelude::*;

/// Static styles for the switcher (keyframes + classes that read a per-element
/// `--accent`/`--active` CSS variable). Injected once; rendered with the trigger
/// so the closed control is styled too.
const SWITCHER_CSS: &str = r#"
.ss-tr{width:100%;display:flex;align-items:center;gap:11px;padding:8px 11px 8px 9px;border-radius:13px;background:rgba(255,255,255,.04);border:1px solid rgba(255,255,255,.09);cursor:pointer;color:inherit;transition:background .18s,border-color .2s,box-shadow .2s}
.ss-tr:hover{background:rgba(255,255,255,.07)}
.ss-tr:hover .ss-tile{transform:translateY(-1px)}
.ss-tr.ss-open{border-color:color-mix(in oklab,var(--accent) 50%,transparent);box-shadow:0 0 0 3px color-mix(in oklab,var(--accent) 11%,transparent),0 10px 24px -14px color-mix(in oklab,var(--accent) 60%,transparent)}
.ss-mini{width:40px;height:40px;padding:0;justify-content:center;border-radius:12px}
.ss-tile{width:28px;height:28px;border-radius:9px;display:grid;place-items:center;flex-shrink:0;background:color-mix(in oklab,var(--accent) 18%,#0b0b10);box-shadow:inset 0 0 0 1px color-mix(in oklab,var(--accent) 28%,transparent),0 2px 8px -3px color-mix(in oklab,var(--accent) 50%,transparent);transition:transform .18s cubic-bezier(.2,.85,.25,1)}
.ss-tile i{font-size:12px;color:var(--accent);filter:drop-shadow(0 0 5px color-mix(in oklab,var(--accent) 55%,transparent))}
.ss-stk{flex:1;text-align:left;min-width:0}
.ss-kick{display:block;font-family:'JetBrains Mono',ui-monospace,monospace;font-size:8.5px;letter-spacing:.2em;text-transform:uppercase;color:rgba(255,255,255,.42);margin-bottom:1px}
.ss-tname{display:block;font-size:13px;font-weight:600;letter-spacing:-.01em;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.ss-chev{font-size:9px;color:rgba(255,255,255,.42);transition:transform .24s cubic-bezier(.2,.85,.25,1)}
.ss-tr.ss-open .ss-chev{transform:rotate(180deg);color:var(--accent)}
.ss-pop{position:absolute;top:calc(100% + 9px);left:0;right:0;z-index:50;border-radius:16px;border:1px solid rgba(255,255,255,.09);overflow:hidden auto;max-height:60vh;isolation:isolate;background:linear-gradient(180deg,color-mix(in oklab,var(--active) 13%,transparent),transparent 80px),rgba(18,18,25,.92);backdrop-filter:blur(22px) saturate(1.4);box-shadow:0 28px 64px -20px rgba(0,0,0,.85),0 0 0 1px rgba(0,0,0,.35),inset 0 1px 0 rgba(255,255,255,.06);transform-origin:top;animation:ss-pop .2s cubic-bezier(.2,.9,.25,1)}
.ss-pop-mini{left:calc(100% + 12px);right:auto;top:-4px;width:218px}
@keyframes ss-pop{from{opacity:0;transform:translateY(-9px) scale(.955)}to{opacity:1;transform:none}}
.ss-head{display:flex;align-items:center;justify-content:space-between;padding:12px 15px 8px}
.ss-head .t{font-family:'JetBrains Mono',ui-monospace,monospace;font-size:9px;letter-spacing:.22em;text-transform:uppercase;color:rgba(255,255,255,.42)}
.ss-head .c{font-family:'JetBrains Mono',ui-monospace,monospace;font-size:9px;color:rgba(255,255,255,.28)}
.ss-list{padding:0 7px}
.ss-row{position:relative;width:100%;display:flex;align-items:center;gap:12px;padding:8px 11px;border-radius:11px;cursor:pointer;color:inherit;background:none;border:0;text-align:left;transition:background .15s;animation:ss-row .3s cubic-bezier(.2,.85,.25,1) backwards}
@keyframes ss-row{from{opacity:0;transform:translateX(-9px)}to{opacity:1;transform:none}}
.ss-row:hover{background:rgba(255,255,255,.055)}
.ss-row:hover .ss-tile{transform:scale(1.06)}
.ss-meta{flex:1;min-width:0}
.ss-rname{display:block;font-size:13px;font-weight:550;letter-spacing:-.01em;color:rgba(255,255,255,.74);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.ss-rsub{display:block;font-family:'JetBrains Mono',ui-monospace,monospace;font-size:9px;letter-spacing:.04em;color:rgba(255,255,255,.42);margin-top:2px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.ss-row.ss-act{background:linear-gradient(90deg,color-mix(in oklab,var(--accent) 17%,transparent),transparent 72%)}
.ss-row.ss-act::before{content:"";position:absolute;left:0;top:8px;bottom:8px;width:3px;border-radius:0 4px 4px 0;background:var(--accent);box-shadow:0 0 12px 1px var(--accent);animation:ss-bar .3s cubic-bezier(.2,.85,.25,1)}
@keyframes ss-bar{from{transform:scaleY(0)}to{transform:scaleY(1)}}
.ss-row.ss-act .ss-rname{color:#fff;font-weight:680}
.ss-check{font-size:10px;color:var(--accent);filter:drop-shadow(0 0 5px color-mix(in oklab,var(--accent) 60%,transparent))}
.ss-foot{margin:7px;border-top:1px solid rgba(255,255,255,.09);padding-top:6px}
.ss-foot button{width:100%;display:flex;align-items:center;gap:10px;padding:8px 11px;border-radius:11px;color:rgba(255,255,255,.42);font-size:12px;font-weight:560;background:none;border:0;cursor:pointer;text-align:left;transition:background .15s,color .15s}
.ss-foot button:hover{background:rgba(255,255,255,.05);color:rgba(255,255,255,.74)}
.ss-foot button .ar{margin-left:auto;font-size:9px}
"#;

const LOCAL_ACCENT: &str = "#8b93ff";

/// One selectable source: key, label, icon class, accent colour, mono subline.
fn entries(config: &AppConfig) -> Vec<(Source, String, &'static str, &'static str, String)> {
    let mut v = vec![(
        Source::Local,
        i18n::t("local").to_string(),
        "fa-solid fa-hard-drive",
        LOCAL_ACCENT,
        i18n::t("source_on_this_device").to_string(),
    )];
    for s in &config.servers {
        let (icon, accent) = service_style(s.service);
        v.push((
            Source::Server(s.id.clone()),
            s.name.clone(),
            icon,
            accent,
            s.service.display_name().to_uppercase(),
        ));
    }
    v
}

/// Icon + accent colour per service, so each source reads at a glance.
fn service_style(service: MusicService) -> (&'static str, &'static str) {
    match service {
        MusicService::YtMusic => ("fa-brands fa-youtube", "#ff3355"),
        MusicService::SoundCloud => ("fa-brands fa-soundcloud", "#ff7a33"),
        MusicService::Jellyfin => ("fa-solid fa-server", "#b277ee"),
        MusicService::Subsonic | MusicService::Custom => ("fa-solid fa-compact-disc", "#f0a84b"),
    }
}

#[component]
pub fn SourceSwitcher(
    config: Signal<AppConfig>,
    #[props(default = false)] collapsed: bool,
    #[props(default)] on_manage: Option<EventHandler<()>>,
) -> Element {
    let mut open = use_signal(|| false);
    let sources = entries(&config.read());
    let count = sources.len();
    let active = config.read().active_source.clone();
    let (active_label, active_icon, active_accent) = sources
        .iter()
        .find(|(s, ..)| *s == active)
        .map(|(_, l, i, a, _)| (l.clone(), *i, *a))
        .unwrap_or_else(|| {
            (
                i18n::t("local").to_string(),
                "fa-solid fa-hard-drive",
                LOCAL_ACCENT,
            )
        });

    rsx! {
        div {
            class: if collapsed { "relative flex justify-center py-3 border-b border-white/5" } else { "relative px-3 pt-3 pb-2 border-b border-white/5" },
            style: "--accent:{active_accent};",
            style { dangerous_inner_html: SWITCHER_CSS }

            button {
                class: match (collapsed, open()) {
                    (true, true) => "ss-tr ss-mini ss-open",
                    (true, false) => "ss-tr ss-mini",
                    (false, true) => "ss-tr ss-open",
                    (false, false) => "ss-tr",
                },
                title: "{active_label}",
                onclick: move |_| open.set(!open()),
                span { class: "ss-tile", i { class: "{active_icon}" } }
                if !collapsed {
                    span { class: "ss-stk",
                        span { class: "ss-kick", "{i18n::t(\"source\")}" }
                        span { class: "ss-tname", "{active_label}" }
                    }
                    i { class: "fa-solid fa-chevron-down ss-chev" }
                }
            }

            if open() {
                div { class: "fixed inset-0 z-40", onclick: move |_| open.set(false) }
                div {
                    class: if collapsed { "ss-pop ss-pop-mini" } else { "ss-pop" },
                    style: "--active:{active_accent};",
                    div { class: "ss-head",
                        span { class: "t", "{i18n::t(\"sources\")}" }
                        span { class: "c", "{count}" }
                    }
                    div { class: "ss-list",
                        for (i , (src , label , icon , accent , sub)) in sources.into_iter().enumerate() {
                            {
                                let is_active = src == active;
                                let row_style = format!("--accent:{accent};animation-delay:{}ms;", i * 32);
                                rsx! {
                                    button {
                                        key: "{src.as_str()}",
                                        class: if is_active { "ss-row ss-act" } else { "ss-row" },
                                        style: "{row_style}",
                                        onclick: move |_| {
                                            crate::shared::set_active_source(config, src.clone());
                                            open.set(false);
                                        },
                                        span { class: "ss-tile", i { class: "{icon}" } }
                                        span { class: "ss-meta",
                                            span { class: "ss-rname", "{label}" }
                                            if !collapsed {
                                                span { class: "ss-rsub", "{sub}" }
                                            }
                                        }
                                        if is_active {
                                            i { class: "fa-solid fa-check ss-check" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if !collapsed && let Some(manage) = on_manage {
                        div { class: "ss-foot",
                            button {
                                onclick: move |_| {
                                    open.set(false);
                                    manage.call(());
                                },
                                i { class: "fa-solid fa-sliders" }
                                "{i18n::t(\"manage_sources\")}"
                                i { class: "fa-solid fa-arrow-right ar" }
                            }
                        }
                    }
                }
            }
        }
    }
}
