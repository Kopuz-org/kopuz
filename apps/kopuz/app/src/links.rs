//! `kopuz://` links from the command line, the system or a later launch: a listen link goes to the daemon, any other opens in the window.

use std::time::Instant;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use futures::channel::oneshot;
use gpui_kit::{App, AppContext as _, AsyncApp};

use crate::instance;

/// Give `link` to the app whose window is open, from a launch that opens none.
pub fn hand_off(address: &link::Address, link: &str) {
    let sent = link::relay::path(address)
        .is_some_and(|path| link::relay::send(&path, &[link.to_string()]));
    if !sent {
        tracing::warn!("could not hand the link to the open window");
    }
}

/// Open `first` and then each link as it arrives; a page link waits for `boot`, when there is a window to show it.
pub fn follow(
    first: Option<String>,
    links: UnboundedReceiver<String>,
    boot: oneshot::Receiver<()>,
    cx: &mut App,
) {
    // Apart from the listen links, so one of those never waits behind a page that is waiting for the window.
    let (pages, mut held) = futures::channel::mpsc::unbounded();
    cx.spawn(async move |cx| {
        let mut links = futures::stream::iter(first).chain(links);
        while let Some(link) = links.next().await {
            match store::deep_link::is_listen(&link) {
                true => cx.update(|cx| instance::open(link, cx)),
                false => drop(pages.unbounded_send(link)),
            }
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        let _ = boot.await;
        while let Some(link) = held.next().await {
            attached(cx).await;
            cx.update(|cx| open(&link, cx));
        }
    })
    .detach();
}

/// Wait for the daemon: a play or queue link sent before it attaches is refused, not held.
async fn attached(cx: &mut AsyncApp) {
    loop {
        let (changed, change) = oneshot::channel();
        let watching = cx.update(|cx| {
            let connection = link::Link::connection(cx);
            if connection.read(cx).is_connected() {
                return None;
            }
            let mut changed = Some(changed);
            Some(cx.observe(&connection, move |_, _| {
                if let Some(changed) = changed.take() {
                    let _ = changed.send(());
                }
            }))
        });
        let Some(_watching) = watching else {
            return;
        };
        let _ = change.await;
    }
}

fn open(text: &str, cx: &mut App) {
    // A link is somebody asking for the app, so it comes forward even from the tray.
    if crate::main_window(cx).is_none() {
        crate::open_main_window(Instant::now(), None, cx);
    }
    cx.activate(true);
    let Some(link) = store::deep_link::parse(text) else {
        // Up to the query only: what follows it may be a password.
        let link = text.split('?').next().unwrap_or_default();
        tracing::warn!(link, "not a link this app opens");
        return store::Notices::error(i18n::t("link_unknown"), cx);
    };
    let Some(window) = crate::main_window(cx) else {
        return;
    };
    // Through the untyped handle: the typed one holds the root, which opening a dialog needs.
    let opened = cx.update_window(window.into(), |_, window, cx| {
        window.activate_window();
        store::deep_link::open(link, window, cx);
    });
    if let Err(error) = opened {
        tracing::warn!(%error, "could not open a link in the window");
    }
}
