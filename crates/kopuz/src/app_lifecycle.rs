use dioxus::prelude::*;

/// The JS runtime the daemon borrows from this process. Desktop runs its own;
/// on Android the only capable runtime is the WebView already drawing the UI,
/// so the daemon's solve requests are drained through `document::eval`. Which
/// source needed a script, and why, stays behind the seam.
#[cfg(target_os = "android")]
pub fn use_webview_script_engine() {
    use_hook(|| {
        let (engine, mut rx) = daemon::script_engine::webview_channel();
        if daemon::script_engine::set_engine(engine).is_err() {
            tracing::warn!("a script engine is already registered; the webview one is not active");
        }
        spawn(async move {
            while let Some(req) = rx.recv().await {
                let wrapped = format!(
                    "globalThis.print=function(s){{dioxus.send(s);}};\
                     try{{{}}}catch(e){{dioxus.send('\\u0000ERR'+(e&&e.stack?e.stack:e));}}",
                    req.program
                );
                let mut eval = dioxus::document::eval(&wrapped);
                let result = match tokio::time::timeout(
                    std::time::Duration::from_secs(20),
                    eval.recv::<String>(),
                )
                .await
                {
                    Ok(Ok(s)) => match s.strip_prefix('\u{0}') {
                        Some(err) => Err(format!("webview JS: {}", err.trim_start_matches("ERR"))),
                        None => Ok(s),
                    },
                    Ok(Err(error)) => Err(format!("webview eval recv: {error}")),
                    Err(_) => Err("webview decipher timed out".to_string()),
                };
                let _ = req.reply.send(result);
            }
        });
    });
}

/// What the dismissable network banner says; offline mode itself is the indicator's to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkBanner {
    /// The internet went down and the daemon moved to `name`, a source that works without it.
    MovedOffline { name: String },
    /// The internet is back after a drop, and the daemon had moved off `source`.
    SwitchBack { source: String, name: String },
    /// The internet is back after a drop that turned offline mode on, which stays until someone turns it off.
    BackOnlineStillOffline,
    /// The internet is back after a drop that needed nothing else.
    BackOnline,
}

/// The banner `status` calls for over these sources; `None` when nothing needs saying. `dropped` says the internet went down earlier in this session.
fn banner_for(
    status: &api::NetworkStatus,
    sources: &[api::SourceInfo],
    dropped: bool,
) -> Option<NetworkBanner> {
    let active = sources.iter().find(|source| source.active);
    let moved_off = status
        .left_source
        .as_ref()
        .filter(|left| active.is_none_or(|active| &active.id != *left));
    if status.internet_down {
        return match (moved_off, active) {
            (Some(_), Some(active)) if !status.offline_mode.is_on() => {
                Some(NetworkBanner::MovedOffline {
                    name: active.name.clone(),
                })
            }
            _ => None,
        };
    }
    if let Some(left) = moved_off.and_then(|left| sources.iter().find(|source| &source.id == left))
    {
        return Some(NetworkBanner::SwitchBack {
            source: left.id.clone(),
            name: left.name.clone(),
        });
    }
    match status.offline_mode {
        api::OfflineMode::Automatic => Some(NetworkBanner::BackOnlineStillOffline),
        api::OfflineMode::Requested => None,
        api::OfflineMode::Off => dropped.then_some(NetworkBanner::BackOnline),
    }
}

/// The app's offline flag, which the pages' downloads-only views read, and its banner, both fed by the daemon rather than a probe of the app's own.
pub fn use_offline_mode(
    mut network_banner: Signal<Option<NetworkBanner>>,
) -> (Signal<bool>, Signal<api::NetworkStatus>) {
    let status = hooks::sources::provide_network_status();
    let sources = hooks::sources::use_sources();
    let mut is_offline = use_signal(|| false);
    let mut dropped = use_signal(|| false);
    // What was last called for, so a dismissed banner stays gone until something new happens.
    let mut called_for = use_signal(|| None::<NetworkBanner>);
    use_context_provider(|| is_offline);
    use_effect(move || {
        let status = status.read().clone();
        let offline = status.offline_mode.is_on();
        if offline != *is_offline.peek() {
            tracing::info!(offline, "offline mode changed");
            is_offline.set(offline);
        }
        if status.internet_down && !*dropped.peek() {
            dropped.set(true);
        }
        let next = banner_for(
            &status,
            sources.read().as_deref().unwrap_or_default(),
            *dropped.peek(),
        );
        if *called_for.peek() == next {
            return;
        }
        called_for.set(next.clone());
        network_banner.set(next.clone());
        match next {
            // The drop has been answered; what follows needs no word about it.
            Some(NetworkBanner::SwitchBack { .. } | NetworkBanner::BackOnlineStillOffline) => {
                dropped.set(false);
            }
            Some(NetworkBanner::BackOnline) => {
                spawn(async move {
                    utils::sleep(std::time::Duration::from_secs(4)).await;
                    dropped.set(false);
                    if network_banner.peek().as_ref() == Some(&NetworkBanner::BackOnline) {
                        network_banner.set(None);
                    }
                });
            }
            _ => {}
        }
    });
    (is_offline, status)
}

/// Leave offline mode, then go back to `source` when the daemon had moved off it.
pub fn switch_back(source: Option<String>) {
    spawn(async move {
        let api = hooks::api::consume_api();
        let left = match api.set_offline(false).await {
            Ok(status) => status.left_source,
            Err(error) => {
                hooks::toast::toast_api_error(&error);
                return;
            }
        };
        if let Some(source) = source.filter(|source| left.as_ref() == Some(source)) {
            hooks::source_switch::apply_source_switch(source).await;
        }
    });
}

pub fn banner_text(banner: &NetworkBanner) -> String {
    match banner {
        NetworkBanner::MovedOffline { name } => {
            i18n::t_with("network_moved_offline", &[("source", name.clone().into())])
        }
        NetworkBanner::SwitchBack { name, .. } => {
            i18n::t_with("back_online_switch_to", &[("source", name.clone().into())])
        }
        NetworkBanner::BackOnlineStillOffline => i18n::t("back_online_still_offline"),
        NetworkBanner::BackOnline => i18n::t("back_online"),
    }
}

/// The offline mode indicator: there whenever offline mode is on, with no way to dismiss it but leaving offline mode.
#[component]
pub fn OfflineModeBar(status: api::NetworkStatus) -> Element {
    let text = match (status.offline_mode, status.internet_down) {
        (api::OfflineMode::Automatic, true) => i18n::t("network_offline_banner"),
        _ => i18n::t("offline_mode_on"),
    };
    rsx! {
        div {
            class: "flex-shrink-0 flex items-center justify-between gap-3 px-4 py-2 bg-amber-500/15 border-b border-amber-500/20 text-amber-300 text-sm",
            role: "status",
            div { class: "flex items-center gap-2",
                i { class: "fa-solid fa-plane text-xs" }
                span { class: "font-semibold", {i18n::t("offline_mode")} }
                span { class: "opacity-80", {text} }
            }
            button {
                class: "text-xs underline opacity-80 hover:opacity-100 transition-opacity",
                onclick: move |_| hooks::sources::set_offline_mode(false, i18n::t("offline_mode_off")),
                {i18n::t("offline_mode_turn_off")}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: &str, active: bool) -> api::SourceInfo {
        api::SourceInfo {
            id: id.into(),
            name: format!("{id} name"),
            active,
            ..Default::default()
        }
    }

    fn status(
        internet_down: bool,
        left: Option<&str>,
        mode: api::OfflineMode,
    ) -> api::NetworkStatus {
        api::NetworkStatus {
            internet_down,
            left_source: left.map(str::to_string),
            offline_mode: mode,
        }
    }

    #[test]
    fn the_banner_follows_the_daemon() {
        use api::OfflineMode::*;
        let on_yt = [source("folder", false), source("yt", true)];
        let moved = [source("folder", true), source("yt", false)];

        assert_eq!(banner_for(&status(false, None, Off), &on_yt, false), None);
        assert_eq!(
            banner_for(&status(false, None, Requested), &on_yt, false),
            None,
            "offline mode someone asked for is the indicator's, never a back-online offer"
        );
        assert_eq!(
            banner_for(&status(true, None, Automatic), &on_yt, true),
            None,
            "the indicator says it"
        );
        assert_eq!(
            banner_for(&status(true, Some("yt"), Off), &moved, true),
            Some(NetworkBanner::MovedOffline {
                name: "folder name".into()
            }),
            "the move off a source is said"
        );
        assert_eq!(
            banner_for(&status(false, Some("yt"), Off), &moved, true),
            Some(NetworkBanner::SwitchBack {
                source: "yt".into(),
                name: "yt name".into()
            })
        );
        assert_eq!(
            banner_for(&status(false, Some("yt"), Off), &on_yt, false),
            None,
            "never an offer to switch to the source already active"
        );
        assert_eq!(
            banner_for(&status(false, None, Automatic), &on_yt, true),
            Some(NetworkBanner::BackOnlineStillOffline)
        );
        assert_eq!(
            banner_for(&status(false, None, Off), &on_yt, true),
            Some(NetworkBanner::BackOnline)
        );
    }
}
