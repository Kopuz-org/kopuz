//! Picking a picture and handing it to the daemon: the one flow every cover and photo editor shares.

use std::path::{Path, PathBuf};

use api::{ArtworkTarget, ArtworkUpload};
use gpui_kit::{App, PathPromptOptions};

use crate::{Notices, actions};

/// The daemon stores the bytes, so it needs to be told what they are.
pub fn content_type_for(path: &Path) -> String {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    }
    .to_string()
}

/// Ask for one image file; `then` runs with it, and never runs when the picker is dismissed.
pub fn pick_image(cx: &mut App, then: impl FnOnce(PathBuf, &mut App) + 'static) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: None,
    });
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = picked.await else {
            return;
        };
        if let Some(path) = paths.into_iter().next() {
            cx.update(|cx| then(path, cx));
        }
    })
    .detach();
}

/// Pick a picture and store it as `target`'s artwork; the daemon decides what that means for it.
pub fn pick_and_upload(target: ArtworkTarget, cx: &mut App) {
    pick_image(cx, move |path, cx| {
        let content_type = content_type_for(&path);
        let read = cx
            .background_executor()
            .spawn(async move { std::fs::read(path) });
        cx.spawn(async move |cx| match read.await {
            Ok(bytes) => cx.update(|cx| {
                actions::run(cx, move |api| async move {
                    api.upload_artwork(ArtworkUpload {
                        target,
                        content_type,
                        bytes,
                    })
                    .await
                })
            }),
            // Reading the picked file, not a daemon call: its own message is the one to show.
            Err(error) => cx.update(|cx| Notices::error(error.to_string(), cx)),
        })
        .detach();
    });
}

pub fn remove_artwork(target: ArtworkTarget, cx: &mut App) {
    actions::run(
        cx,
        move |api| async move { api.remove_artwork(target).await },
    );
}
