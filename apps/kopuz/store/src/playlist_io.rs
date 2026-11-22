use std::path::{Path, PathBuf};

use api::{ApiError, ExternalPlaylist, JobKind, PlaylistImport, TrackInfo};
use gpui_kit::{App, PathPromptOptions};

use crate::{JobsModel, Link, Notices, SourcesModel};

pub fn import_file(cx: &mut App) {
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
        let Some(path) = paths.into_iter().next() else {
            return;
        };
        let text = match std::fs::read(&path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(error) => {
                cx.update(|cx| Notices::error(error.to_string(), cx));
                return;
            }
        };
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| i18n::t("import_playlist"));
        let csv = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"));
        let base = path.parent().map(|dir| dir.to_string_lossy().into_owned());
        let call = cx.update(|cx| {
            Link::global(cx).call(move |api| async move {
                match csv {
                    true => {
                        api.import_external_playlist(ExternalPlaylist::Csv { name, text })
                            .await
                    }
                    false => api.import_playlist(name, text, base).await,
                }
            })
        });
        let result = call.await;
        cx.update(|cx| announce(result, cx));
    })
    .detach();
}

pub fn import_link(link: String, cx: &mut App) {
    let call = Link::global(cx).call(move |api| async move {
        api.import_external_playlist(ExternalPlaylist::Link(link))
            .await
    });
    cx.spawn(async move |cx| {
        let result = call.await;
        cx.update(|cx| announce(result, cx));
    })
    .detach();
}

fn announce(result: Result<PlaylistImport, ApiError>, cx: &mut App) {
    match result {
        Ok(import) => {
            if SourcesModel::global(cx).read(cx).capabilities().sync {
                JobsModel::global(cx).update(cx, |jobs, cx| jobs.start(JobKind::PlaylistSync, cx));
            }
            let key = match import.missing.is_empty() {
                true => "playlist_imported",
                false => "playlist_imported_partly",
            };
            Notices::info(
                plain(i18n::t_with(
                    key,
                    &[
                        ("matched", import.matched.into()),
                        ("missing", import.missing.len().into()),
                    ],
                )),
                cx,
            );
        }
        Err(error) => Notices::failed(&error, cx),
    }
}

pub fn export_m3u(name: String, keys: Vec<String>, cx: &mut App) {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let file = format!("{}.m3u8", sanitize(&name));
    let chosen = cx.prompt_for_new_path(&home, Some(&file));
    let call = Link::global(cx).call(move |api| async move { api.tracks_by_keys(keys).await });
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(path))) = chosen.await else {
            return;
        };
        let tracks = match call.await {
            Ok(tracks) => tracks,
            Err(error) => {
                cx.update(|cx| Notices::failed(&error, cx));
                return;
            }
        };
        let written = std::fs::write(&path, render(&tracks));
        cx.update(|cx| match written {
            Ok(()) => Notices::info(
                plain(i18n::t_with(
                    "playlist_exported",
                    &[("count", tracks.len().into())],
                )),
                cx,
            ),
            Err(error) => Notices::error(error.to_string(), cx),
        });
    })
    .detach();
}

fn plain(text: String) -> String {
    text.replace(['\u{2068}', '\u{2069}'], "")
}

fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    match cleaned.trim() {
        "" => "playlist".to_string(),
        trimmed => trimmed.to_string(),
    }
}

pub(crate) fn render(tracks: &[TrackInfo]) -> String {
    let mut out = String::from("#EXTM3U\n");
    for track in tracks {
        let seconds = track
            .duration_ms
            .map_or(-1, |ms| i64::try_from(ms / 1000).unwrap_or(-1));
        out.push_str(&format!(
            "#EXTINF:{seconds},{} - {}\n",
            track.artist, track.title
        ));
        if Path::new(&track.key).is_absolute() {
            out.push_str(&track.key);
        } else {
            out.push_str(&format!("# {}", track.key));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(key: &str, title: &str, ms: Option<u64>) -> TrackInfo {
        TrackInfo {
            key: key.into(),
            title: title.into(),
            artist: "Artist".into(),
            duration_ms: ms,
            ..TrackInfo::default()
        }
    }

    #[test]
    fn local_tracks_are_written_by_path_and_remote_ones_as_notes() {
        let text = render(&[
            track("/music/a.flac", "A", Some(61_500)),
            track("yt123", "B", None),
        ]);
        assert_eq!(
            text,
            "#EXTM3U\n#EXTINF:61,Artist - A\n/music/a.flac\n#EXTINF:-1,Artist - B\n# yt123\n"
        );
    }

    #[test]
    fn a_name_becomes_a_safe_file_name() {
        assert_eq!(sanitize("AC/DC: Live?"), "AC_DC_ Live_");
        assert_eq!(sanitize("  "), "playlist");
    }
}
