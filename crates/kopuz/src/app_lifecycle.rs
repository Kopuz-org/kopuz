use dioxus::prelude::*;
use tracing::Instrument;

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

pub fn use_connectivity_probe(mut network_banner: Signal<Option<bool>>) -> Signal<bool> {
    let mut is_offline = use_signal(|| false);
    use_context_provider(|| is_offline);
    // Only a remote source makes reachability a thing worth watching, and
    // which is active is the daemon's answer.
    let active = hooks::sources::use_active_source_info();
    use_future(move || async move {
        let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
        else {
            return;
        };
        let mut misses: u8 = 0;
        loop {
            if !active
                .peek()
                .as_ref()
                .is_some_and(|source| source.kind == api::SourceKind::Server)
            {
                if *is_offline.peek() {
                    is_offline.set(false);
                }
                misses = 0;
                utils::sleep(std::time::Duration::from_secs(30)).await;
                continue;
            }
            let online = client
                .get("https://1.1.1.1")
                .send()
                .instrument(tracing::info_span!("net.connectivity"))
                .await
                .is_ok();
            if online {
                misses = 0;
                if *is_offline.peek() {
                    is_offline.set(false);
                }
            } else {
                misses = misses.saturating_add(1);
                if misses >= 2 && !*is_offline.peek() {
                    is_offline.set(true);
                }
            }
            let secs = if *is_offline.peek() { 10 } else { 30 };
            utils::sleep(std::time::Duration::from_secs(secs)).await;
        }
    });

    use_effect(move || {
        if *is_offline.read() {
            network_banner.set(Some(true));
        } else if network_banner.peek().as_ref() == Some(&true) {
            network_banner.set(Some(false));
            spawn(async move {
                utils::sleep(std::time::Duration::from_secs(4)).await;
                if network_banner.read().as_ref() == Some(&false) {
                    network_banner.set(None);
                }
            });
        }
    });

    is_offline
}
