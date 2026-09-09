//! Source-agnostic Playlists page (issue #35). The chrome (header, add-playlist,
//! folder/playlist detail) is shared; the grid renders folders + local management
//! when the source organises playlists in folders ([`Capabilities::folders`]), or
//! a flat remote list with per-card downloads + sync otherwise. No `is_server()`
//! dispatch — every divergence gates on the resolved source's capabilities.

use components::dots_menu::{DotsMenu, MenuAction};
use components::folder_picker::FolderPickerModal;
use components::playlist_detail::PlaylistDetail;
use components::playlist_popups::AddPlaylistPopup;
use config::{AppConfig, MusicService, UiStyle};
use dioxus::prelude::*;
use hooks::use_db_queries::use_playlists;

#[component]
#[tracing::instrument(name = "render.playlists_page", skip_all)]
pub fn PlaylistsPage(
    config: Signal<AppConfig>,
    mut selected_playlist_id: Signal<Option<String>>,
) -> Element {
    let nav_ctrl = use_context::<components::NavigationController>();
    let caps = hooks::sources::use_capabilities();

    let mut show_add_playlist = use_signal(|| false);
    let mut playlist_name = use_signal(String::new);
    let mut error = use_signal(|| Option::<String>::None);
    let mut saving = use_signal(|| false);
    let mut playlist_refresh_trigger = use_signal(|| 0u64);

    let playlists_res = use_playlists();

    let handle_add_playlist = move |_| {
        if saving() {
            return;
        }
        let name = playlist_name();
        // A source that can't mutate playlists (a creds-less/offline server, or a
        // read-only source) gets the friendly message instead of a raw error.
        if caps().playlists == api::PlaylistCapability::None {
            error.set(Some(i18n::t("error_server_not_configured").to_string()));
            return;
        }
        let api = hooks::consume_api();
        error.set(None);
        saving.set(true);
        spawn(async move {
            let result = api.create_playlist(name, Vec::new()).await;
            saving.set(false);
            match result {
                Ok(_) => {
                    // A server create mirrors into the DB but a re-sync still
                    // reconciles remote-side details, so the sync path re-fetches.
                    if caps().sync {
                        playlist_refresh_trigger.with_mut(|v| *v += 1);
                    }
                    show_add_playlist.set(false);
                    playlist_name.set(String::new());
                }
                Err(e) => {
                    error.set(Some(e.to_string()));
                }
            }
        });
    };

    let downloads = hooks::downloads::use_downloads();

    let mut last_source = use_signal(|| config.read().active_source.clone());
    if *last_source.read() != config.read().active_source {
        selected_playlist_id.set(None);
        last_source.set(config.read().active_source.clone());
    }

    let is_vaxry = config.read().ui_style == UiStyle::Vaxry;

    rsx! {
        div { class: if cfg!(target_os = "android") { "px-4 pt-2 absolute inset-0 flex flex-col" } else if is_vaxry { "px-6 pt-6 absolute inset-0 flex flex-col" } else { "px-8 pt-8 absolute inset-0 flex flex-col" },
            if let Some(pid) = selected_playlist_id.read().clone() {
                {
                    let pid_for_dl = pid.clone();
                    let is_downloading_all = {
                        let store = playlists_res.read().clone().unwrap_or_default();
                        store
                            .playlists
                            .iter()
                            .find(|playlist| playlist.id == pid)
                            .map(|playlist| playlist.tracks.clone())
                            .unwrap_or_default()
                            .iter()
                            .any(|key| downloads.read().is_active(key))
                    };
                    let pid_for_del = pid.clone();
                    let pid_for_dl_track = pid.clone();
                    rsx! {
                        PlaylistDetail {
                            playlist_id: pid,
                            config,
                            on_close: move |_| nav_ctrl.close_playlist(),
                            is_downloading_all,
                            on_download_all: move |_| {
                                // The playlist already holds its track refs, and
                                // refs are what a download takes.
                                let keys: Vec<String> = playlists_res
                                    .read()
                                    .clone()
                                    .unwrap_or_default()
                                    .playlists
                                    .iter()
                                    .find(|playlist| playlist.id == pid_for_dl)
                                    .map(|playlist| playlist.tracks.clone())
                                    .unwrap_or_default();
                                hooks::downloads::start(keys);
                            },
                            on_delete_all: move |_| {
                                let ids: Vec<String> = {
                                    let store = playlists_res.read().clone().unwrap_or_default();
                                    store
                                        .playlists
                                        .iter()
                                        .find(|p| p.id == pid_for_del)
                                        .map(|p| p.tracks.clone())
                                        .unwrap_or_default()
                                };
                                if !ids.is_empty() {
                                    hooks::downloads::remove(ids);
                                }
                            },
                            on_download_track: move |idx: usize| {
                                let store = playlists_res.read().clone().unwrap_or_default();
                                let Some(key) = store
                                    .playlists
                                    .iter()
                                    .find(|playlist| playlist.id == pid_for_dl_track)
                                    .and_then(|playlist| playlist.tracks.get(idx))
                                    .cloned()
                                else {
                                    return;
                                };
                                if downloads.read().is_stored(&key) {
                                    hooks::downloads::remove(vec![key]);
                                } else {
                                    hooks::downloads::start(vec![key]);
                                }
                            },
                        }
                    }
                }
            } else {
                div { class: if is_vaxry { "flex items-center justify-between mb-6" } else { "flex items-center justify-between mb-8" },
                    if is_vaxry {
                        div {
                            p {
                                class: "text-[10px] font-bold mb-0.5",
                                style: "color: rgba(255,255,255,0.35);",
                                "{i18n::t(\"library\")}"
                            }
                            h1 { class: "text-2xl font-semibold tracking-tight text-white", "{i18n::t(\"playlists\")}" }
                        }
                    } else {
                        h1 { class: "text-3xl font-semibold tracking-tight text-white", "{i18n::t(\"playlists\")}" }
                    }
                    div { class: "flex items-center gap-1",
                        if caps().folders {
                            button {
                                class: "w-10 h-10 flex items-center justify-center text-white/60 hover:text-white rounded-full hover:bg-white/10 transition-colors active:scale-95",
                                title: i18n::t("new_folder").to_string(),
                                onclick: move |_| {
                                    hooks::playlist_actions::create_folder(
                                        i18n::t("new_folder").to_string(),
                                    );
                                },
                                i { class: "fa-solid fa-folder-plus" }
                            }
                        }
                        button {
                            class: "w-10 h-10 flex items-center justify-center text-white/60 hover:text-white rounded-full hover:bg-white/10 transition-colors active:scale-95",
                            title: i18n::t("add_playlist").to_string(),
                            aria_label: i18n::t("add_playlist").to_string(),
                            onclick: move |_| {
                                error.set(None);
                                show_add_playlist.set(true);
                            },
                            i { class: "fa-solid fa-add" }
                        }
                    }
                }
                if show_add_playlist() {
                    AddPlaylistPopup {
                        playlist_name,
                        error,
                        on_close: move |_| {
                            error.set(None);
                            show_add_playlist.set(false);
                        },
                        on_save: handle_add_playlist,
                        show_add_folder: caps().folders,
                        on_add_folder: move |folder_path: String| {
                            let folder_path_buf = std::path::PathBuf::from(&folder_path);
                            let folder_name = folder_path_buf
                                .file_name()
                                .map(|name| name.to_string_lossy().to_string())
                                .unwrap_or_else(|| folder_path.clone());
                            let prefix = if folder_path.ends_with(std::path::MAIN_SEPARATOR) {
                                folder_path
                            } else {
                                format!("{folder_path}{}", std::path::MAIN_SEPARATOR)
                            };
                            let api = hooks::consume_api();
                            spawn(async move {
                                let refs: Vec<String> = api
                                    .folder_tracks(prefix, hooks::use_db_queries::all())
                                    .await
                                    .map(|page| {
                                        page.items.into_iter().map(|track| track.key).collect()
                                    })
                                    .unwrap_or_default();
                                if let Err(error) = api.create_playlist(folder_name, refs).await {
                                    hooks::toast::toast_error(&error.to_string());
                                }
                            });
                            error.set(None);
                            playlist_name.set(String::new());
                        },
                    }
                }

                PlaylistsGrid {
                    config,
                    selected_playlist_id,
                    refresh_trigger: playlist_refresh_trigger,
                }
            }
        }
    }
}

/// What a playlist card's overflow entry does. The menu is built conditionally —
/// the folder entries only appear inside a folder, radio only when the source
/// supports it — so entries carry their meaning instead of being matched by
/// position, where adding one silently rewires every entry after it.
#[derive(Clone, Copy, PartialEq)]
enum PlaylistCardAction {
    MoveToFolder,
    RemoveFromFolder,
    Rename,
    StartRadio,
    Delete,
}

/// The playlists grid: folders + local management when the source organises into
/// folders, else a flat remote list with downloads + remote sync. One component,
/// gated on [`Capabilities`].
#[component]
fn PlaylistsGrid(
    config: Signal<AppConfig>,
    mut selected_playlist_id: Signal<Option<String>>,
    refresh_trigger: Signal<u64>,
) -> Element {
    let caps = hooks::sources::use_capabilities();
    let is_offline = use_context::<Signal<bool>>();
    let downloads = hooks::downloads::use_downloads();

    let playlists_res = use_playlists();
    // First track of each playlist — the cover-of-last-resort for a playlist with
    // no explicit cover / image tag (resolved through the source cover seam).
    // Local folder-management state (mutated inside `folders_layout`'s handlers).
    let active_menu = use_signal(|| Option::<String>::None);
    let open_folder_id = use_signal(|| Option::<String>::None);
    let move_target_id = use_signal(|| Option::<String>::None);
    let rename_playlist_id = use_signal(|| Option::<String>::None);
    let rename_playlist_name = use_signal(String::new);
    let rename_folder_id = use_signal(|| Option::<String>::None);
    let rename_folder_name = use_signal(String::new);

    // The pull runs in the daemon, single-flight, so this only has to ask.
    // The stamp that stops an automatic re-sync lives there too, which is why
    // no dedup key or request id is kept here any more.
    let sync_job = hooks::jobs::use_job_progress(hooks::JobKind::PlaylistSync);
    let yt_is_syncing = use_memo(move || sync_job.read().running);
    let yt_synced_so_far = use_memo(move || sync_job.read().current.unwrap_or(0) as usize);

    use_effect(move || {
        if caps().sync {
            // Naming the trigger keeps the effect subscribed to it.
            let _ = *refresh_trigger.read();
            hooks::jobs::start(hooks::JobKind::PlaylistSync);
        }
    });

    let store = playlists_res.read().clone().unwrap_or_default();

    // A playlist's cover is the daemon's to resolve: it walks the explicit
    // cover, then the server's image tag, then the first track's art. The
    // middle one is signed with credentials that never leave the daemon, so
    // the chain cannot live here.
    let cover_for = |playlist: &reader::models::Playlist| -> Option<utils::CoverUrl> {
        ::server::cover::from_path(&config.read(), playlist.cover_path.as_deref(), 384)
    };

    if caps().folders {
        return folders_layout(FoldersCtx {
            selected_playlist_id,
            store,
            cover_for: &cover_for,
            active_menu,
            open_folder_id,
            move_target_id,
            rename_playlist_id,
            rename_playlist_name,
            rename_folder_id,
            rename_folder_name,
        });
    }

    // ---- Server (flat remote list) layout ----------------------------------
    let offline = caps().downloads && *is_offline.read();
    let conf = config.read();
    let playlists: Vec<reader::models::Playlist> = if offline {
        store
            .playlists
            .iter()
            .filter(|p| {
                !p.tracks.is_empty()
                    && p.tracks.iter().all(|tid| {
                        conf.offline_tracks
                            .get(tid)
                            .map(|path| std::path::Path::new(path).exists())
                            .unwrap_or(false)
                    })
            })
            .cloned()
            .collect()
    } else {
        store.playlists.clone()
    };
    drop(conf);
    let is_yt = caps().albums == api::AlbumPresentation::Remote;
    // The flat remote card has no overflow menu of its own, so radio is its one
    // entry — no kind-tagged action list needed here (unlike the folder card).
    let can_radio = caps().playlist_radio;
    let radio_text = components::radio_actions::radio_label();
    let radio_actions = vec![MenuAction::new(
        radio_text.as_str(),
        components::radio_actions::RADIO_ICON,
    )];
    let mut active_menu = active_menu;
    let yt_anon = config
        .read()
        .server
        .as_ref()
        .map(|s| s.service == MusicService::YtMusic && s.yt_anonymous)
        .unwrap_or(false);

    rsx! {
        div {
            if is_yt {
                {
                    let syncing = *yt_is_syncing.read();
                    let done = *yt_synced_so_far.read();
                    let total = playlists.len();
                    let remaining = total.saturating_sub(done);
                    rsx! {
                        div { class: "flex items-center justify-between gap-3 mb-3 px-2 text-xs text-slate-400",
                            div { class: "flex items-center gap-2",
                                if syncing {
                                    i { class: "fa-solid fa-arrows-rotate fa-spin text-indigo-300" }
                                    span { "Loading tracks — {done} / {total} playlists ({remaining} left)" }
                                } else if total > 0 {
                                    i { class: "fa-solid fa-check text-emerald-400" }
                                    span { "{total} playlists synced" }
                                }
                            }
                            button {
                                class: "px-3 py-1 rounded-lg bg-white/10 hover:bg-white/20 text-white/80 transition-colors disabled:opacity-50",
                                disabled: syncing,
                                onclick: move |_| {
                                    hooks::jobs::start(hooks::JobKind::PlaylistSync);
                                },
                                i { class: "fa-solid fa-arrows-rotate mr-1" }
                                "Refresh"
                            }
                        }
                    }
                }
            }

            if playlists.is_empty() {
                div { class: "flex flex-col items-center justify-center h-64 text-slate-500 text-center px-6",
                    if yt_anon {
                        i { class: "fa-solid fa-right-to-bracket text-4xl mb-4 opacity-50" }
                        p { "{i18n::t(\"yt_anon_playlists\")}" }
                    } else {
                        i { class: "fa-regular fa-folder-open text-4xl mb-4 opacity-50" }
                        p { "{i18n::t(\"no_playlists_found\")}" }
                    }
                }
            } else {
                div { class: "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-6",
                    {playlists.into_iter().map(|playlist| {
                        let cover_url = cover_for(&playlist);
                        let playlist_id_nav = playlist.id.clone();
                        let is_dl = playlist.tracks.iter().any(|key| downloads.read().is_active(key));
                        let all_downloaded = !playlist.tracks.is_empty()
                            && playlist.tracks.iter().all(|key| downloads.read().is_stored(key));
                        rsx! {
                            div {
                                key: "{playlist.id}",
                                class: "bg-white/5 border border-white/5 rounded-xl p-6 hover:bg-white/10 transition-all cursor-pointer group relative",
                                onclick: move |_| selected_playlist_id.set(Some(playlist_id_nav.clone())),
                                div { class: "mb-4 w-full aspect-square rounded-xl flex items-center justify-center overflow-hidden transition-all bg-white/5",
                                    if let Some(url) = cover_url {
                                        img { src: "{url}", class: "w-full h-full object-cover", decoding: "async", loading: "lazy" }
                                    } else {
                                        div {
                                            class: "w-full h-full flex items-center justify-center",
                                            style: "background: color-mix(in srgb, var(--color-indigo-500), transparent 80%); color: var(--color-indigo-400)",
                                            i { class: "fa-solid fa-server text-2xl" }
                                        }
                                    }
                                }
                                div { class: "flex items-start justify-between gap-2",
                                    div { class: "min-w-0 flex-1",
                                        h3 { class: "text-xl font-bold text-white mb-1 truncate", "{playlist.name}" }
                                        p { class: "text-sm text-slate-400", "Server • {playlist.tracks.len()} tracks" }
                                    }
                                    if can_radio {
                                        {
                                            let pid_menu = playlist.id.clone();
                                            let is_menu_open = active_menu.read().as_deref() == Some(playlist.id.as_str());
                                            let start_radio = components::radio_actions::playlist_radio_handler(playlist.id.clone());
                                            rsx! {
                                                div { onclick: move |evt: Event<MouseData>| evt.stop_propagation(),
                                                    DotsMenu {
                                                        actions: radio_actions.clone(),
                                                        is_open: is_menu_open,
                                                        on_open: move |_| active_menu.set(Some(pid_menu.clone())),
                                                        on_close: move |_| active_menu.set(None),
                                                        button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100".to_string(),
                                                        anchor: "right".to_string(),
                                                        on_action: move |_: usize| {
                                                            active_menu.set(None);
                                                            if let Some(handler) = start_radio {
                                                                handler.call(());
                                                            }
                                                        },
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                if caps().downloads {
                                    button {
                                        class: "absolute top-4 right-4 w-8 h-8 rounded-full bg-black/40 border border-white/10 flex items-center justify-center text-white/60 hover:text-white hover:border-white/30 transition-colors opacity-0 group-hover:opacity-100",
                                        title: if all_downloaded { "Remove downloads" } else { "Download playlist for offline playback" },
                                        disabled: is_dl,
                                        onclick: move |evt| {
                                            evt.stop_propagation();
                                            if all_downloaded {
                                                hooks::downloads::remove(playlist.tracks.clone());
                                            } else {
                                                hooks::downloads::start(playlist.tracks.clone());
                                            }
                                        },
                                        if is_dl {
                                            i { class: "fa-solid fa-spinner fa-spin text-xs" }
                                        } else if all_downloaded {
                                            i { class: "fa-solid fa-trash text-xs" }
                                        } else {
                                            i { class: "fa-solid fa-download text-xs" }
                                        }
                                    }
                                }
                            }
                        }
                    })}
                }
            }
        }
    }
}

/// Borrowed bundle for the folder-tree layout (keeps the function signature sane).
struct FoldersCtx<'a> {
    selected_playlist_id: Signal<Option<String>>,
    store: reader::PlaylistStore,
    cover_for: &'a dyn Fn(&reader::models::Playlist) -> Option<utils::CoverUrl>,
    active_menu: Signal<Option<String>>,
    open_folder_id: Signal<Option<String>>,
    move_target_id: Signal<Option<String>>,
    rename_playlist_id: Signal<Option<String>>,
    rename_playlist_name: Signal<String>,
    rename_folder_id: Signal<Option<String>>,
    rename_folder_name: Signal<String>,
}

fn folders_layout(ctx: FoldersCtx<'_>) -> Element {
    let FoldersCtx {
        mut selected_playlist_id,
        store,
        cover_for,
        mut active_menu,
        mut open_folder_id,
        mut move_target_id,
        mut rename_playlist_id,
        mut rename_playlist_name,
        mut rename_folder_id,
        mut rename_folder_name,
    } = ctx;

    let folders = store.folders.clone();
    let all_playlists = store.playlists.clone();
    let root_playlists: Vec<_> = all_playlists
        .iter()
        .filter(|p| !folders.iter().any(|f| f.playlist_ids.contains(&p.id)))
        .cloned()
        .collect();
    let open_folder = open_folder_id
        .read()
        .as_ref()
        .and_then(|id| folders.iter().find(|f| f.id == *id).cloned());
    let folder_playlists: Vec<_> = if let Some(ref folder) = open_folder {
        folder
            .playlist_ids
            .iter()
            .filter_map(|pid| all_playlists.iter().find(|p| p.id == *pid).cloned())
            .collect()
    } else {
        vec![]
    };

    let delete_playlist_text = i18n::t("delete_playlist").to_string();
    let rename_playlist_text = i18n::t("rename_playlist").to_string();
    let rename_folder_text = i18n::t("rename_folder").to_string();
    let move_text = i18n::t("move_to_folder").to_string();
    let remove_folder_text = i18n::t("remove_from_folder").to_string();
    let delete_folder_text = i18n::t("delete_folder").to_string();

    let radio_text = components::radio_actions::radio_label();
    let can_radio = consume_context::<Signal<::server::source::ActiveSource>>()
        .read()
        .capabilities()
        .radio
        .playlist;

    let build_playlist_actions = |in_folder: bool| -> (Vec<MenuAction>, Vec<PlaylistCardAction>) {
        let mut entries = vec![(
            MenuAction::new(move_text.as_str(), "fa-solid fa-folder-open"),
            PlaylistCardAction::MoveToFolder,
        )];
        if in_folder {
            entries.push((
                MenuAction::new(remove_folder_text.as_str(), "fa-solid fa-folder-minus"),
                PlaylistCardAction::RemoveFromFolder,
            ));
        }
        entries.push((
            MenuAction::new(rename_playlist_text.as_str(), "fa-solid fa-pen"),
            PlaylistCardAction::Rename,
        ));
        if can_radio {
            entries.push((
                MenuAction::new(radio_text.as_str(), components::radio_actions::RADIO_ICON),
                PlaylistCardAction::StartRadio,
            ));
        }
        entries.push((
            MenuAction::new(delete_playlist_text.as_str(), "fa-solid fa-trash").destructive(),
            PlaylistCardAction::Delete,
        ));
        entries.into_iter().unzip()
    };

    let folder_actions = vec![
        MenuAction::new(rename_folder_text.as_str(), "fa-solid fa-pen"),
        MenuAction::new(delete_folder_text.as_str(), "fa-solid fa-trash").destructive(),
    ];

    let render_card = |playlist: &reader::models::Playlist, in_folder: bool| {
        let cover_url = cover_for(playlist);
        let pid = playlist.id.clone();
        let pid_click = playlist.id.clone();
        let pid_menu = playlist.id.clone();
        let pid_action = playlist.id.clone();
        let name_for_rename = playlist.name.clone();
        let name = playlist.name.clone();
        let count = playlist.tracks.len();
        let is_menu_open = active_menu.read().as_deref() == Some(playlist.id.as_str());
        let (actions, action_kinds) = build_playlist_actions(in_folder);
        // Resolved during render, like the track rows' radio: the handler reads
        // context, which an event closure can't do.
        let start_radio = components::radio_actions::playlist_radio_handler(playlist.id.clone());
        rsx! {
            div {
                key: "{pid}",
                class: "bg-white/5 border border-white/5 rounded-xl p-4 hover:bg-white/10 transition-all cursor-pointer group relative",
                onclick: move |_| selected_playlist_id.set(Some(pid_click.clone())),
                div { class: "mb-4 w-full h-32 rounded-xl flex items-center justify-center overflow-hidden transition-all bg-white/5",
                    if let Some(url) = cover_url {
                        img {
                            src: "{url}",
                            class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500",
                            decoding: "async",
                            loading: "lazy",
                        }
                    } else {
                        div {
                            class: "w-full h-full flex items-center justify-center",
                            style: "background: color-mix(in srgb, var(--color-indigo-500), transparent 80%); color: var(--color-indigo-400)",
                            i { class: "fa-solid fa-list-ul text-2xl" }
                        }
                    }
                }
                div { class: "flex items-start justify-between gap-2",
                    div { class: "min-w-0 flex-1",
                        h3 { class: "text-xl font-bold text-white mb-1 truncate", "{name}" }
                        {
                            let track_text = i18n::t_with("playlist_track_count", &[("count", count.to_string())]);
                            rsx! { p { class: "text-sm text-slate-400", "{track_text}" } }
                        }
                    }
                    div { onclick: move |evt| evt.stop_propagation(),
                        DotsMenu {
                            actions,
                            is_open: is_menu_open,
                            on_open: move |_| active_menu.set(Some(pid_menu.clone())),
                            on_close: move |_| active_menu.set(None),
                            button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100".to_string(),
                            anchor: "right".to_string(),
                            on_action: move |idx: usize| {
                                active_menu.set(None);
                                let Some(kind) = action_kinds.get(idx).copied() else {
                                    return;
                                };
                                match kind {
                                    PlaylistCardAction::MoveToFolder => {
                                        move_target_id.set(Some(pid_action.clone()))
                                    }
                                    PlaylistCardAction::RemoveFromFolder => {
                                        hooks::playlist_actions::move_to_folder(
                                            pid_action.clone(),
                                            None,
                                        );
                                    }
                                    PlaylistCardAction::Rename => {
                                        rename_playlist_id.set(Some(pid_action.clone()));
                                        rename_playlist_name.set(name_for_rename.clone());
                                    }
                                    PlaylistCardAction::StartRadio => {
                                        if let Some(handler) = start_radio {
                                            handler.call(());
                                        }
                                    }
                                    // Deleting from inside a folder also drops the
                                    // membership row, so the folder doesn't keep a
                                    // dangling id.
                                    // The daemon drops the folder membership with
                                    // the playlist, so there is nothing to undo
                                    // here for a playlist that sat in one.
                                    PlaylistCardAction::Delete => {
                                        hooks::playlist_actions::delete(pid_action.clone());
                                    }
                                }
                            },
                        }
                    }
                }
            }
        }
    };

    rsx! {
        div {
            if let Some(target_id) = move_target_id.read().clone() {
                FolderPickerModal {
                    playlist_id: target_id,
                    on_close: move |_| move_target_id.set(None),
                }
            }
            if let Some(rename_id) = rename_playlist_id.read().clone() {
                RenameTextModal {
                    title: rename_playlist_text.clone(),
                    value: rename_playlist_name,
                    on_close: move |_| {
                        rename_playlist_id.set(None);
                        rename_playlist_name.set(String::new());
                    },
                    on_save: move |_| {
                        let name = rename_playlist_name.read().trim().to_string();
                        if name.is_empty() {
                            return;
                        }
                        hooks::playlist_actions::rename(rename_id.clone(), name);
                        rename_playlist_id.set(None);
                        rename_playlist_name.set(String::new());
                    },
                }
            }
            if let Some(rename_id) = rename_folder_id.read().clone() {
                RenameTextModal {
                    title: rename_folder_text.clone(),
                    value: rename_folder_name,
                    on_close: move |_| {
                        rename_folder_id.set(None);
                        rename_folder_name.set(String::new());
                    },
                    on_save: move |_| {
                        let name = rename_folder_name.read().trim().to_string();
                        if name.is_empty() {
                            return;
                        }
                        hooks::playlist_actions::rename_folder(rename_id.clone(), name);
                        rename_folder_id.set(None);
                        rename_folder_name.set(String::new());
                    },
                }
            }

            if let Some(ref folder) = open_folder {
                div {
                    div { class: "flex items-center gap-3 mb-8",
                        components::back_button::BackButton {
                            class: "",
                            on_click: move |_| open_folder_id.set(None),
                        }
                        span { class: "text-white/30", "/" }
                        span { class: "text-white font-semibold", "{folder.name}" }
                    }
                    if folder_playlists.is_empty() {
                        div { class: "flex flex-col items-center justify-center h-48 text-slate-500",
                            i { class: "fa-regular fa-folder-open text-4xl mb-4 opacity-50" }
                            p { "{i18n::t(\"no_playlists_yet\")}" }
                        }
                    } else {
                        div { class: "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-6",
                            {folder_playlists.iter().map(|p| render_card(p, true))}
                        }
                    }
                }
            } else {
                div {
                    if folders.is_empty() && root_playlists.is_empty() {
                        div { class: "flex flex-col items-center justify-center h-64 text-slate-500",
                            i { class: "fa-regular fa-folder-open text-4xl mb-4 opacity-50" }
                            p { "{i18n::t(\"no_playlists_yet\")}" }
                        }
                    } else {
                        if !folders.is_empty() {
                            div { class: "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-6 mb-8",
                                {folders.iter().map(|folder| {
                                    let fid = folder.id.clone();
                                    let fid_open = folder.id.clone();
                                    let fid_menu = folder.id.clone();
                                    let fid_del = folder.id.clone();
                                    let fid_rename = folder.id.clone();
                                    let fname = folder.name.clone();
                                    let fname_rename = folder.name.clone();
                                    let count = folder.playlist_ids.len();
                                    let is_menu_open = active_menu.read().as_deref() == Some(folder.id.as_str());
                                    let cover_url = folder
                                        .playlist_ids
                                        .first()
                                        .and_then(|pid| all_playlists.iter().find(|p| p.id == *pid))
                                        .and_then(cover_for);
                                    let folder_actions = folder_actions.clone();
                                    rsx! {
                                        div {
                                            key: "{fid}",
                                            class: "bg-white/5 border border-white/5 rounded-xl p-4 hover:bg-white/10 transition-all cursor-pointer group relative",
                                            onclick: move |_| open_folder_id.set(Some(fid_open.clone())),
                                            div { class: "mb-4 w-full h-32 rounded-xl flex items-center justify-center overflow-hidden transition-all bg-white/5",
                                                if let Some(url) = cover_url {
                                                    img {
                                                        src: "{url}",
                                                        class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500",
                                                        decoding: "async",
                                                        loading: "lazy",
                                                    }
                                                } else {
                                                    div {
                                                        class: "w-full h-full flex items-center justify-center",
                                                        style: "background: color-mix(in srgb, var(--color-amber-500), transparent 80%); color: var(--color-amber-400)",
                                                        i { class: "fa-solid fa-folder text-2xl" }
                                                    }
                                                }
                                            }
                                            div { class: "flex items-start justify-between gap-2",
                                                div { class: "min-w-0 flex-1",
                                                    h3 { class: "text-xl font-bold text-white mb-1 truncate", "{fname}" }
                                                    p { class: "text-sm text-slate-400", "{count} playlists" }
                                                }
                                                div { onclick: move |evt| evt.stop_propagation(),
                                                    DotsMenu {
                                                        actions: folder_actions,
                                                        is_open: is_menu_open,
                                                        on_open: move |_| active_menu.set(Some(fid_menu.clone())),
                                                        on_close: move |_| active_menu.set(None),
                                                        button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100".to_string(),
                                                        anchor: "right".to_string(),
                                                        on_action: move |idx: usize| {
                                                            active_menu.set(None);
                                                            if idx == 0 {
                                                                rename_folder_id.set(Some(fid_rename.clone()));
                                                                rename_folder_name.set(fname_rename.clone());
                                                            } else {
                                                                hooks::playlist_actions::delete_folder(
                                                                    fid_del.clone(),
                                                                );
                                                            }
                                                        },
                                                    }
                                                }
                                            }
                                        }
                                    }
                                })}
                            }
                        }
                        if !root_playlists.is_empty() {
                            if !folders.is_empty() {
                                h2 { class: "text-sm font-semibold text-white/40 mb-4",
                                    "{i18n::t(\"playlists\")}"
                                }
                            }
                            div { class: "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-6",
                                {root_playlists.iter().map(|p| render_card(p, false))}
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RenameTextModal(
    title: String,
    value: Signal<String>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
) -> Element {
    rsx! {
        div {
            class: "fixed inset-0 bg-black/70 flex items-center justify-center z-50",
            onclick: move |_| on_close.call(()),
            div {
                class: "bg-neutral-900 border border-white/10 rounded-lg p-6 w-80 shadow-2xl",
                onclick: move |evt| evt.stop_propagation(),
                h2 { class: "text-lg font-bold text-white mb-4", "{title}" }
                input {
                    class: "w-full bg-white/10 border border-white/10 rounded-lg px-3 py-2 text-sm text-white placeholder-slate-500 focus:outline-none focus:border-white/25 mb-4",
                    value: "{value()}",
                    oninput: move |evt| value.set(evt.value()),
                    onkeydown: move |evt| {
                        evt.stop_propagation();
                        if evt.key() == Key::Enter {
                            on_save.call(());
                        }
                    },
                }
                div { class: "flex justify-end gap-2",
                    button {
                        class: "px-3 py-2 rounded-lg text-sm text-slate-400 hover:text-white hover:bg-white/10 transition-colors",
                        onclick: move |_| on_close.call(()),
                        "{i18n::t(\"cancel\")}"
                    }
                    button {
                        class: "px-3 py-2 bg-white text-black rounded-lg text-sm font-medium hover:bg-slate-200 transition-colors disabled:opacity-50 disabled:cursor-not-allowed",
                        disabled: value.read().trim().is_empty(),
                        onclick: move |_| on_save.call(()),
                        "{i18n::t(\"save\")}"
                    }
                }
            }
        }
    }
}
