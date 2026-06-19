//! The source switcher: Local + every configured server as a uniform list, pick
//! one to make it active. Replaces the old binary Local⇄Server toggle — no
//! local-vs-server branching, and it reaches any number of servers.
//!
//! A compact trigger (the active source's brand-tinted "jack" + name) opens a
//! glassy popover that springs in with a staggered reveal and scrolls past ~8
//! sources, so the control stays neat with one source or a dozen. Each source
//! carries its service's accent colour (a CSS `--accent` var the styles read);
//! the active row glows with it. Mono micro-labels use the app's JetBrains Mono.

use config::{AppConfig, MusicService, Source, UiStyle};
use dioxus::prelude::*;

/// Static styles for the switcher (keyframes + classes that read a per-element
/// `--accent`/`--active` CSS variable). Injected once; rendered with the trigger
/// so the closed control is styled too.
// Colours route through two indirection vars set per UI style on the wrapper:
// `--ss-surface` (the popover/tile base) and `--ss-fg` (the foreground). The
// dimmed tones are `--ss-fg`-based `color-mix` (foreground at reduced alpha), so
// they track whichever polarity the wrapper picked. The switcher must match the
// chrome it sits in, and that differs by UI style: the Modern ("Vaxry") sidebar
// is a hardcoded dark surface on every colour theme, so it gets fixed dark
// values; the Normal sidebar follows the theme, so it gets the theme tokens
// (`--color-neutral-900` / `--color-white`) and tracks light/dark themes.
// Per-service brand accents stay fixed regardless.
const SWITCHER_CSS: &str = r#"
.ss-tr{width:100%;display:flex;align-items:center;gap:11px;padding:9px 11px;border-radius:12px;background:color-mix(in oklab,var(--ss-fg) 4%,transparent);border:1px solid color-mix(in oklab,var(--ss-fg) 12%,transparent);cursor:pointer;color:inherit;transition:background .15s,border-color .15s}
.ss-tr:hover{background:color-mix(in oklab,var(--ss-fg) 8%,transparent)}
.ss-tr.ss-open{border-color:color-mix(in oklab,var(--accent) 45%,transparent)}
.ss-mini{width:40px;height:40px;padding:0;justify-content:center;border-radius:11px}
.ss-tile{width:28px;height:28px;border-radius:8px;display:grid;place-items:center;flex-shrink:0;background:color-mix(in oklab,var(--accent) 16%,var(--ss-surface));border:1px solid color-mix(in oklab,var(--accent) 32%,transparent)}
.ss-tile i{font-size:12px;color:var(--accent)}
.ss-stk{flex:1;text-align:left;min-width:0}
.ss-kick{display:block;font-family:'JetBrains Mono',ui-monospace,monospace;font-size:8.5px;letter-spacing:.2em;text-transform:uppercase;color:color-mix(in oklab,var(--ss-fg) 42%,transparent);margin-bottom:2px}
.ss-tname{display:block;font-size:13px;font-weight:600;letter-spacing:-.01em;color:var(--ss-fg);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.ss-chev{font-size:9px;color:color-mix(in oklab,var(--ss-fg) 42%,transparent);transition:transform .18s}
.ss-tr.ss-open .ss-chev{transform:rotate(180deg);color:var(--accent)}
.ss-pop{position:absolute;top:calc(100% + 7px);left:0;right:0;z-index:50;border-radius:12px;border:1px solid color-mix(in oklab,var(--ss-fg) 12%,transparent);overflow:hidden auto;max-height:60vh;background:var(--ss-surface);box-shadow:0 16px 40px -20px rgba(0,0,0,.7)}
.ss-pop-mini{left:calc(100% + 12px);right:auto;top:-4px;width:218px}
.ss-head{display:flex;align-items:center;justify-content:space-between;padding:11px 13px 9px;border-bottom:1px solid color-mix(in oklab,var(--ss-fg) 10%,transparent)}
.ss-head .t{font-family:'JetBrains Mono',ui-monospace,monospace;font-size:9px;letter-spacing:.2em;text-transform:uppercase;color:color-mix(in oklab,var(--ss-fg) 42%,transparent)}
.ss-head .c{font-family:'JetBrains Mono',ui-monospace,monospace;font-size:9px;color:color-mix(in oklab,var(--ss-fg) 32%,transparent)}
.ss-list{padding:5px}
.ss-row{position:relative;width:100%;display:flex;align-items:center;gap:11px;padding:8px 9px;border-radius:9px;cursor:pointer;color:inherit;background:none;border:0;text-align:left;transition:background .12s}
.ss-row:hover{background:color-mix(in oklab,var(--ss-fg) 7%,transparent)}
.ss-meta{flex:1;min-width:0}
.ss-rname{display:block;font-size:13px;font-weight:550;letter-spacing:-.01em;color:color-mix(in oklab,var(--ss-fg) 82%,transparent);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.ss-rsub{display:block;font-family:'JetBrains Mono',ui-monospace,monospace;font-size:9px;letter-spacing:.04em;text-transform:uppercase;color:color-mix(in oklab,var(--ss-fg) 42%,transparent);margin-top:3px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.ss-row.ss-act{background:color-mix(in oklab,var(--ss-fg) 7%,transparent)}
.ss-row.ss-act .ss-rname{color:var(--ss-fg);font-weight:650}
.ss-check{font-size:10px;color:var(--accent)}
.ss-foot{padding:5px;border-top:1px solid color-mix(in oklab,var(--ss-fg) 10%,transparent)}
.ss-foot button{width:100%;display:flex;align-items:center;gap:10px;padding:8px 9px;border-radius:9px;color:color-mix(in oklab,var(--ss-fg) 60%,transparent);font-size:12px;font-weight:550;background:none;border:0;cursor:pointer;text-align:left;transition:background .12s,color .12s}
.ss-foot button:hover{background:color-mix(in oklab,var(--ss-fg) 7%,transparent);color:var(--ss-fg)}
.ss-foot button .ar{margin-left:auto;font-size:9px}
"#;

/// Local uses the active theme's accent so it reads as native (servers keep
/// their fixed brand colours).
const LOCAL_ACCENT: &str = "var(--color-indigo-500)";

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
    // Match the chrome: the Modern ("Vaxry") sidebar is a fixed dark surface on
    // every colour theme; the Normal sidebar follows the theme tokens.
    let surface_vars = match config.read().ui_style {
        UiStyle::Modern => "--ss-surface:#16161d;--ss-fg:#ffffff;",
        UiStyle::Normal => "--ss-surface:var(--color-neutral-900);--ss-fg:var(--color-white);",
    };
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
            style: "--accent:{active_accent};{surface_vars}",
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
                    div { class: "ss-head",
                        span { class: "t", "{i18n::t(\"sources\")}" }
                        span { class: "c", "{count}" }
                    }
                    div { class: "ss-list",
                        for (src , label , icon , accent , sub) in sources.into_iter() {
                            {
                                let is_active = src == active;
                                rsx! {
                                    button {
                                        key: "{src.as_str()}",
                                        class: if is_active { "ss-row ss-act" } else { "ss-row" },
                                        style: "--accent:{accent};",
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
