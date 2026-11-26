//! Links and local files from the command line, the system or a later launch.

use std::time::Instant;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use futures::channel::oneshot;
use gpui_kit::component::Root;
use gpui_kit::{App, AppContext as _, AsyncApp, WindowHandle};

use crate::{args, instance};

enum Action {
    Link(String),
    Files(Vec<String>),
}

/// Give `items` to the app whose window is open, from a launch that opens none.
pub fn hand_off(address: &link::Address, items: &[String]) {
    let sent = link::relay::path(address).is_some_and(|path| link::relay::send(&path, items));
    if !sent {
        tracing::warn!("could not hand items to the open window");
    }
}

/// Open `first` and then each batch as it arrives; window actions wait for `boot`.
pub fn follow(
    first: Vec<String>,
    batches: UnboundedReceiver<Vec<String>>,
    boot: oneshot::Receiver<()>,
    cx: &mut App,
) {
    // Apart from the listen links, so one of those never waits behind a page that is waiting for the window.
    let (actions, mut held) = futures::channel::mpsc::unbounded();
    cx.spawn(async move |cx| {
        let initial = (!first.is_empty()).then_some(first);
        let mut incoming = futures::stream::iter(initial).chain(batches);
        while let Some(batch) = incoming.next().await {
            let mut files = Vec::new();
            for item in batch {
                if store::deep_link::is_listen(&item) {
                    cx.update(|cx| instance::open(item, cx));
                } else if let Some(path) = args::file_url_path(&item) {
                    files.push(path);
                } else {
                    let _ = actions.unbounded_send(Action::Link(item));
                }
            }
            if !files.is_empty() {
                let _ = actions.unbounded_send(Action::Files(files));
            }
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        let _ = boot.await;
        while let Some(action) = held.next().await {
            attached(cx).await;
            cx.update(|cx| match action {
                Action::Link(link) => open(&link, cx),
                Action::Files(files) => open_files(files, cx),
            });
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

/// Bring the main window forward, reopening it if it was hidden to the tray.
fn focus_window(cx: &mut App) -> Option<WindowHandle<Root>> {
    if crate::main_window(cx).is_none() {
        crate::open_main_window(Instant::now(), None, cx);
    }
    cx.activate(true);
    crate::main_window(cx)
}

fn open_files(files: Vec<String>, cx: &mut App) {
    if let Some(window) = focus_window(cx) {
        let _ = cx.update_window(window.into(), |_, window, _| {
            window.activate_window();
        });
    }
    store::actions::play_tracks(files, Some(0), cx);
}

fn open(text: &str, cx: &mut App) {
    let window = focus_window(cx);
    let Some(link) = store::deep_link::parse(text) else {
        // Up to the query only: what follows it may be a password.
        let link = text.split('?').next().unwrap_or_default();
        tracing::warn!(link, "not a link this app opens");
        return store::Notices::error(i18n::t("link_unknown"), cx);
    };
    let Some(window) = window else {
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
