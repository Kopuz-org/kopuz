//! The playlists page: folders and playlists, or the flat remote layout.

use std::collections::HashMap;
use std::rc::Rc;

use api::{AlbumPresentation, JobKind, PlaylistInfo, SourceCapabilities};
use components::format::t;
use components::{
    DownloadToggle, EmptyState, Heading, IconButton, Loading, PageHeader, SectionHeading,
    SyncStatus, ToggleSize, icons,
};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};
use store::{DownloadsModel, JobsModel, Offline, PlaylistsModel, SourcesModel, actions};

use crate::cards::{folder_card, playlist_card};
use crate::dialogs;
use components::{Tile, tile_grid as card_grid};

pub struct Playlists {
    // Each playlist's keys and what the downloads say of them, rebuilt on a change, not per card per frame.
    offline: HashMap<String, (Rc<[String]>, Offline)>,
    _subscriptions: Vec<Subscription>,
}

impl Playlists {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&PlaylistsModel::global(cx), |this, _, cx| {
                this.rekey_offline(cx);
                cx.notify();
            }),
            cx.observe(&JobsModel::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&DownloadsModel::global(cx), |this, _, cx| {
                this.restate_offline(cx);
                cx.notify();
            }),
            cx.observe(&SourcesModel::global(cx), |this: &mut Self, _, cx| {
                this.rekey_offline(cx);
                cx.notify();
            }),
        ];
        let mut page = Self {
            offline: HashMap::new(),
            _subscriptions: subscriptions,
        };
        page.rekey_offline(cx);
        page
    }

    /// The playlists or the source changed: take every playlist's keys afresh.
    fn rekey_offline(&mut self, cx: &App) {
        self.offline.clear();
        if !SourcesModel::global(cx).read(cx).capabilities().downloads {
            return;
        }
        let downloads = DownloadsModel::global(cx).read(cx);
        for playlist in PlaylistsModel::global(cx).read(cx).playlists() {
            let offline = downloads.offline(&playlist.track_keys);
            self.offline.insert(
                playlist.id.clone(),
                (playlist.track_keys.clone().into(), offline),
            );
        }
    }

    /// A download moved: the keys stand, only what the downloads say of them is asked again.
    fn restate_offline(&mut self, cx: &App) {
        let downloads = DownloadsModel::global(cx).read(cx);
        for (keys, offline) in self.offline.values_mut() {
            *offline = downloads.offline(keys);
        }
    }

    fn header(&self, capabilities: &SourceCapabilities, cx: &mut Context<Self>) -> PageHeader {
        let add = cx.listener(|_, _: &ClickEvent, window, cx| {
            dialogs::open_add_playlist(window, cx);
        });
        PageHeader::new(t("playlists"))
            .when(capabilities.folders, |header| {
                header.action(
                    IconButton::new("new-folder", icons::FOLDER)
                        .tooltip(t("new_folder"))
                        .on_click(|_, _, cx| {
                            let name = t("new_folder").to_string();
                            actions::run(cx, move |api| async move {
                                api.create_playlist_folder(name).await
                            });
                        }),
                )
            })
            .action(
                IconButton::new("add-playlist", icons::PLUS)
                    .tooltip(t("add_playlist"))
                    .on_click(add),
            )
    }

    fn folders_layout(&self, cx: &mut Context<Self>) -> AnyElement {
        let model = PlaylistsModel::global(cx);
        let folders = model.read(cx).folders().to_vec();
        let playlists = model.read(cx).playlists().to_vec();
        let root: Vec<PlaylistInfo> = playlists
            .iter()
            .filter(|playlist| {
                !folders
                    .iter()
                    .any(|folder| folder.playlist_ids.contains(&playlist.id))
            })
            .cloned()
            .collect();

        if folders.is_empty() && root.is_empty() {
            return EmptyState::new(t("no_playlists_yet"))
                .icon(icons::FOLDER)
                .into_any_element();
        }
        v_flex()
            .gap_8()
            .when(!folders.is_empty(), |column| {
                column.child(
                    card_grid()
                        .children(folders.iter().map(|folder| folder_card(folder, &playlists))),
                )
            })
            .when(!root.is_empty(), |column| {
                column.child(
                    v_flex()
                        .gap_4()
                        .when(!folders.is_empty(), |section| {
                            section
                                .child(SectionHeading::new(t("playlists")).heading(Heading::Group))
                        })
                        .child(card_grid().children(root.iter().map(playlist_card))),
                )
            })
            .into_any_element()
    }

    fn flat_layout(&self, capabilities: &SourceCapabilities, cx: &mut Context<Self>) -> AnyElement {
        let playlists = PlaylistsModel::global(cx).read(cx).playlists().to_vec();
        let anonymous = SourcesModel::global(cx)
            .read(cx)
            .active()
            .is_some_and(|source| source.anonymous);
        let remote = capabilities.albums == AlbumPresentation::Remote;

        v_flex()
            .gap_3()
            .when(remote, |column| {
                column.child(self.sync_row(playlists.len()))
            })
            .child(if playlists.is_empty() {
                if anonymous {
                    EmptyState::new(t("source_anon_playlists"))
                        .icon(icons::LIST)
                        .into_any_element()
                } else {
                    EmptyState::new(t("no_playlists_found"))
                        .icon(icons::FOLDER)
                        .into_any_element()
                }
            } else {
                let mut cards = Vec::with_capacity(playlists.len());
                for playlist in &playlists {
                    cards.push(self.remote_card(playlist, capabilities));
                }
                card_grid().children(cards).into_any_element()
            })
            .into_any_element()
    }

    fn remote_card(&self, playlist: &PlaylistInfo, capabilities: &SourceCapabilities) -> Tile {
        let corner = capabilities
            .downloads
            .then(|| self.offline.get(&playlist.id))
            .flatten()
            .map(|(keys, offline)| {
                DownloadToggle::new(format!("download-{}", playlist.id), keys.clone())
                    .size(ToggleSize::Corner)
                    .offline(*offline)
                    .on_toggle(commands::set_offline)
            });
        playlist_card(playlist)
            .subtitle(format!(
                "{} • {}",
                t("server_playlist"),
                components::format::t_with(
                    "playlist_track_count",
                    &[("count", playlist.track_keys.len().into())],
                )
            ))
            .corner(corner)
    }

    fn sync_row(&self, total: usize) -> impl IntoElement {
        SyncStatus::new(JobKind::PlaylistSync)
            .total(total)
            .on_refresh(|_, _, cx| {
                JobsModel::global(cx).update(cx, |jobs, cx| jobs.start(JobKind::PlaylistSync, cx));
            })
    }
}

impl Render for Playlists {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let capabilities = SourcesModel::global(cx).read(cx).capabilities();
        let loaded = PlaylistsModel::global(cx).read(cx).is_loaded();
        let header = self.header(&capabilities, cx);
        let body = if !loaded {
            Loading::new().into_any_element()
        } else if capabilities.folders {
            self.folders_layout(cx)
        } else {
            self.flat_layout(&capabilities, cx)
        };
        components::page().child(header).child(
            v_flex()
                .id("playlists-body")
                .flex_1()
                .min_h_0()
                .pb_20()
                .overflow_y_scroll()
                .child(body),
        )
    }
}
