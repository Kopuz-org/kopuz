//! One album: the standard detail, and the catalog-styled one a remote source presents.

use std::rc::Rc;

use api::{
    AlbumInfo, AlbumPresentation, ArtworkRef, ArtworkTarget, CatalogDetail, CatalogDetailRequest,
    CatalogItemKind, SourceCapabilities, Table, TrackInfo,
};
use components::format::{t, t_with};
use components::{
    DetailActions, DetailHeader, EditableCover, EmptyState, HeaderLayout, Heading, IconButton,
    IconTone, Link, Loading, Look, Picture, SectionHeading, Tone, TrackActions, TrackList,
    TrackListEvent, TrackListOptions, TrackSection, icons, menus, style,
};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Window, div, px, rems,
};
use store::{
    DownloadsModel, Offline, Pictured, PlayerModel, Query, Route, SourcesModel, Target, actions,
};

use crate::ops;

// TODO(offline): filtering the listing to downloaded tracks waits on an offline seam the checklist flags as wrong.

// `w-64 h-64` on the showcase, `max-w-[300px]` on the catalog-styled page.
const HERO_COVER: Pixels = px(256.);
const REMOTE_COVER: Pixels = px(300.);
// `w-12 h-12`, the outlined select-all beside the hero's play.
const HERO_SECONDARY: Pixels = px(48.);
const COVER: &str = "album-cover";

/// One disc's rows; its list reports a play by the album-wide index its subscription carries.
struct DiscGroup {
    disc: Option<u32>,
    list: Entity<TrackList>,
}

pub struct AlbumDetail {
    id: SharedString,
    album: Entity<Query<Option<AlbumInfo>>>,
    tracks: Entity<Query<Vec<TrackInfo>>>,
    remote: Entity<Query<Option<CatalogDetail>>>,
    rows: Vec<TrackInfo>,
    // Derived from the rows and the downloads once per change, not once per frame.
    keys: Rc<[String]>,
    total_ms: Option<u64>,
    offline: Offline,
    groups: Vec<DiscGroup>,
    // The album ids the track query is pointed at, and the id the remote one is.
    aggregated: Vec<String>,
    remote_id: Option<String>,
    capabilities: SourceCapabilities,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
    _lists: Vec<Subscription>,
}

impl AlbumDetail {
    pub fn new(id: SharedString, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let album_id = id.to_string();
        let album = cx.new(|cx| {
            let album_id = album_id.clone();
            Query::new(
                &[Table::Albums],
                move |api| {
                    let id = album_id.clone();
                    async move { api.album(id).await }
                },
                cx,
            )
        });
        let tracks = cx.new(|cx| {
            let ids = vec![album_id.clone()];
            Query::new(
                &[Table::Tracks],
                move |api| {
                    let ids = ids.clone();
                    async move { Ok(collect_album_tracks(api, ids).await) }
                },
                cx,
            )
        });
        let remote = cx.new(|cx| {
            Query::new(
                &[Table::Albums],
                |_| async move { Ok::<Option<CatalogDetail>, api::ApiError>(None) },
                cx,
            )
        });

        let sources = SourcesModel::global(cx);
        let downloads = DownloadsModel::global(cx);
        let subscriptions = vec![
            cx.observe(&album, |this, _, cx| this.sync(cx)),
            cx.observe(&sources, |this, _, cx| this.sync(cx)),
            cx.observe(&tracks, |this, _, cx| this.refresh_rows(cx)),
            cx.observe(&remote, |this, _, cx| this.refresh_rows(cx)),
            cx.observe(&downloads, |this, _, cx| {
                this.refresh_offline(cx);
                cx.notify();
            }),
            // The shuffle button in the actions bar reads the player at render.
            cx.observe(&PlayerModel::global(cx), |_, _, cx| cx.notify()),
        ];

        Self {
            id,
            album,
            tracks,
            remote,
            rows: Vec::new(),
            keys: Rc::from([]),
            total_ms: None,
            offline: Offline::default(),
            groups: Vec::new(),
            aggregated: vec![album_id],
            remote_id: None,
            // Sources usually loaded before the page opened, so their observer would not fire again.
            capabilities: SourcesModel::global(cx).read(cx).capabilities(),
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
            _lists: Vec::new(),
        }
    }

    /// Point the track and catalog reads at what the album turned out to be.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let capabilities = SourcesModel::global(cx).read(cx).capabilities();
        let album = self.album.read(cx).value().cloned().flatten();
        let settled = self.album.read(cx).is_settled();

        let ids = vec![match &album {
            Some(album) => album.id.clone(),
            None => self.id.to_string(),
        }];
        if ids != self.aggregated {
            self.aggregated = ids.clone();
            point_tracks(&self.tracks, ids, cx);
        }

        // A catalog-presented source stores only the saved subset; a miss means none of it is stored.
        let remote_id = if capabilities.albums == AlbumPresentation::Remote {
            Some(
                album
                    .as_ref()
                    .map(|album| album.id.clone())
                    .unwrap_or_else(|| self.id.to_string()),
            )
        } else if settled && album.is_none() {
            Some(self.id.to_string())
        } else {
            None
        };
        if remote_id != self.remote_id {
            self.remote_id = remote_id.clone();
            point_remote(&self.remote, remote_id, cx);
        }

        if capabilities != self.capabilities {
            self.capabilities = capabilities;
            self.rebuild_groups(cx);
        }
        cx.notify();
    }

    fn refresh_rows(&mut self, cx: &mut Context<Self>) {
        let mut rows = match self.remote.read(cx).value().cloned().flatten() {
            Some(detail) if !detail.tracks.is_empty() => detail.tracks,
            _ => self.tracks.read(cx).value().cloned().unwrap_or_default(),
        };
        ops::in_album_order(&mut rows);
        if rows != self.rows {
            self.keys = ops::keys_of(&rows).into();
            self.total_ms = api::total_duration_secs(&rows).map(|secs| secs * 1000);
            self.rows = rows;
            self.refresh_offline(cx);
            self.rebuild_groups(cx);
        }
        cx.notify();
    }

    fn refresh_offline(&mut self, cx: &App) {
        self.offline = DownloadsModel::global(cx).read(cx).offline(&self.keys);
    }

    fn rebuild_groups(&mut self, cx: &mut Context<Self>) {
        let remote = self.capabilities.albums == AlbumPresentation::Remote;
        let options = TrackListOptions {
            show_index: true,
            show_cover: false,
            show_album: false,
            track_numbers: true,
            selectable: !remote,
        };
        let row_actions = TrackActions {
            remove_from_playlist: false,
            delete: self.capabilities.delete_from_disk && !remote,
            reorder: false,
        };

        let mut groups = Vec::new();
        let mut subscriptions = Vec::new();
        let mut offset = 0;
        for (disc, chunk) in by_disc(&self.rows) {
            let list = cx.new(|cx| {
                TrackList::new(cx)
                    .with_options(options)
                    .with_actions(row_actions)
            });
            commands::bind_track_list(&list, cx);
            let count = chunk.len();
            list.update(cx, |list, cx| list.set_rows(chunk, cx));
            subscriptions.push(
                cx.subscribe(&list, move |this, _, event: &TrackListEvent, cx| {
                    this.on_list_event(offset, event, cx)
                }),
            );
            groups.push(DiscGroup { disc, list });
            offset += count;
        }
        self.groups = groups;
        self._lists = subscriptions;
    }

    fn on_list_event(&mut self, offset: usize, event: &TrackListEvent, cx: &mut Context<Self>) {
        match event {
            TrackListEvent::Play(index) => {
                let keys: Vec<String> = self.rows.iter().map(|row| row.key.clone()).collect();
                actions::play_tracks(keys, Some((offset + index) as u32), cx);
            }
            TrackListEvent::Delete(keys) => ops::delete_tracks(keys.clone(), cx),
            _ => cx.notify(),
        }
    }

    fn selected_keys(&self, cx: &App) -> Vec<String> {
        self.groups
            .iter()
            .flat_map(|group| group.list.read(cx).selected_keys())
            .collect()
    }

    fn is_selecting(&self, cx: &App) -> bool {
        self.groups
            .iter()
            .any(|group| group.list.read(cx).is_selecting())
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        for group in &self.groups {
            group.list.update(cx, |list, cx| list.select_all(cx));
        }
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        for group in &self.groups {
            group.list.update(cx, |list, cx| list.clear_selection(cx));
        }
    }

    fn cover(&self, album: &AlbumInfo, size: Pixels, radius: Pixels) -> EditableCover {
        let cover = EditableCover::new(COVER, album.artwork.as_ref())
            .size(size)
            .rounded(radius)
            .placeholder(icons::DISC);
        if !self.capabilities.edit_tags {
            return cover;
        }
        let (id, remove_id) = (album.id.clone(), album.id.clone());
        cover
            .on_pick(move |_, cx| store::pick_and_upload(ArtworkTarget::Album(id.clone()), cx))
            .on_remove(t("remove_cover"), move |_, cx| {
                store::remove_artwork(ArtworkTarget::Album(remove_id.clone()), cx)
            })
    }

    /// `text-sm font-bold text-white/60 hover:text-white hover:underline transition-colors`.
    fn artist_link(artist: &str, key: Option<String>, id: &'static str) -> impl IntoElement {
        // An album that bills nobody the daemon could key opens nothing.
        Link::new(id)
            .label(artist.to_string())
            .route(key.map(Route::Artist))
            .tone(Tone::Quiet)
            .underline(true)
            .max_w_full()
            .text_sm()
            .font_weight(gpui_kit::FontWeight::BOLD)
    }

    fn tracks_body(&self) -> AnyElement {
        if self.rows.is_empty() {
            return EmptyState::new(t("no_tracks_found")).into_any_element();
        }
        div()
            .id("album-tracks")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .children(self.groups.iter().map(|group| {
                let section = TrackSection::new(
                    format!("album-disc-{}", group.disc.unwrap_or(0)),
                    group.list.clone(),
                )
                .gap(px(0.));
                match group.disc {
                    Some(disc) => section
                        .heading(
                            SectionHeading::new(t_with("disc_heading", &[("number", disc.into())]))
                                .heading(Heading::Group)
                                .icon(icons::DISC),
                        )
                        .wash(true),
                    None => section,
                }
            }))
            .into_any_element()
    }

    fn selection_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.is_selecting(cx) {
            return None;
        }
        let keys = self.selected_keys(cx);
        let first = self
            .rows
            .iter()
            .find(|row| keys.first() == Some(&row.key))
            .map(|row| row.title.clone())
            .unwrap_or_default();
        let picked = keys.iter().cloned().map(SharedString::from).collect();
        let page = cx.entity().downgrade();
        let (acted, cancelled) = (page.clone(), page);
        let bar = commands::selection_bar(
            "album-selection",
            keys.len(),
            Target::selection(picked, first),
            move |command, target, window, cx| {
                command.perform(target, window, cx);
                acted.update(cx, |this, cx| this.clear_selection(cx)).ok();
            },
            cx,
        )
        .lift(rems(6.))
        .on_cancel(move |_, cx| {
            cancelled
                .update(cx, |this, cx| this.clear_selection(cx))
                .ok();
        });
        let bar = match self.capabilities.delete_from_disk {
            true => bar.on_delete(move |_, cx| ops::delete_tracks(keys.clone(), cx)),
            false => bar,
        };
        Some(bar.into_any_element())
    }

    fn count(&self) -> SharedString {
        t_with("showcase_song_count", &[("count", self.rows.len().into())])
    }

    fn minutes(&self) -> Option<String> {
        self.total_ms
            .map(|total| format!("{} {}", total / 60_000, t("min")))
    }

    fn standard_detail(
        &self,
        album: AlbumInfo,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let radius = style::thumb_radius() * 3.;
        let select_all = IconButton::new("album-select-all", icons::CHECK)
            .size(HERO_SECONDARY)
            .tone(IconTone::Outline)
            .tooltip(t("select_all"))
            .on_click(cx.listener(|this, _, _, cx| this.select_all(cx)));
        let picture = picture(&album);
        let spot = picture.as_ref().map(Picture::spot);
        let mut header = DetailHeader::new(SharedString::from(album.title.clone()))
            .cover(self.cover(&album, HERO_COVER, radius).spot(spot))
            .cover_menu(cover_menu(&album), picture)
            .eyebrow(Self::artist_link(
                &album.artist,
                album.artist_key.clone(),
                "album-artist",
            ))
            .meta(self.count())
            .when_some(self.minutes(), |header, minutes| header.meta(minutes))
            .action(
                DetailActions::new("album", self.keys.clone())
                    .offline(self.offline)
                    .shuffle(commands::set_shuffle)
                    .play(commands::play_all)
                    .extra(select_all)
                    .download(commands::set_offline),
            );
        if album.year > 0 {
            header = header.meta(album.year.to_string());
        }
        components::page()
            .relative()
            .pb_8()
            .overflow_hidden()
            .child(div().flex_shrink_0().mb_12().child(header))
            .child(self.tracks_body())
            .children(self.selection_bar(cx))
            .into_any_element()
    }

    fn remote_detail(&self, view: RemoteView, _: &mut Window, _: &mut Context<Self>) -> AnyElement {
        let cover = AlbumInfo {
            id: view.id.clone(),
            title: view.title.to_string(),
            artist: view.artist.to_string(),
            artwork: view.artwork.clone(),
            ..AlbumInfo::default()
        };
        let browse_id = view.browse_id.clone();
        let first_key = self.rows.first().map(|row| row.key.clone());
        let radius = style::thumb_radius() * 2.;
        let picture = picture(&cover);
        let spot = picture.as_ref().map(Picture::spot);
        let mut header = DetailHeader::new(view.title.clone())
            .layout(HeaderLayout::Column)
            .cover(self.cover(&cover, REMOTE_COVER, radius).spot(spot))
            .cover_menu(cover_menu(&cover), picture)
            .eyebrow(Self::artist_link(
                &view.artist,
                view.artist_key.clone(),
                "album-remote-artist",
            ))
            .kind(t("album"))
            .meta(self.count())
            .when_some(self.minutes(), |header, minutes| header.meta(minutes))
            .action(
                DetailActions::new("album-remote", self.keys.clone())
                    .look(Look::Catalog)
                    .offline(self.offline)
                    .download(commands::set_offline)
                    .artist(view.artist_key.clone())
                    .play(commands::play_all)
                    .shuffle(commands::set_shuffle)
                    .share(move |_, _, cx| {
                        ops::share_album(browse_id.clone(), first_key.clone(), cx)
                    }),
            );
        if let Some(year) = view.year.clone() {
            header = header.meta(year);
        }

        components::page()
            .relative()
            .pb_8()
            .overflow_hidden()
            .child(
                h_flex()
                    .size_full()
                    .min_h_0()
                    .gap_10()
                    .items_start()
                    .child(header)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .min_h_0()
                            .child(self.tracks_body()),
                    ),
            )
            .into_any_element()
    }
}

impl Render for AlbumDetail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        components::scroll::smooth("album-detail-scroll", &self.scroll, window, cx);
        let album = self.album.read(cx).value().cloned().flatten();
        let remote = self.remote.read(cx).value().cloned().flatten();

        if self.capabilities.albums == AlbumPresentation::Remote {
            if let Some(view) = RemoteView::of(album.as_ref(), remote.as_ref()) {
                return self.remote_detail(view, window, cx);
            }
        } else if let Some(album) = album.clone() {
            return self.standard_detail(album, window, cx);
        } else if let Some(view) = RemoteView::of(None, remote.as_ref()) {
            return self.remote_detail(view, window, cx);
        }

        match self.album.read(cx).is_settled() && self.remote.read(cx).is_settled() {
            true => EmptyState::new(t("album_not_found"))
                .icon(icons::DISC)
                .into_any_element(),
            false => Loading::new().into_any_element(),
        }
    }
}

/// The header the catalog-styled page draws, from the stored row, the catalog listing, or both.
struct RemoteView {
    id: String,
    title: SharedString,
    artist: SharedString,
    year: Option<SharedString>,
    artwork: Option<ArtworkRef>,
    browse_id: Option<String>,
    artist_key: Option<String>,
}

impl RemoteView {
    fn of(album: Option<&AlbumInfo>, remote: Option<&CatalogDetail>) -> Option<Self> {
        match (album, remote) {
            (Some(album), _) => Some(Self {
                id: album.id.clone(),
                title: album.title.clone().into(),
                artist: album.artist.clone().into(),
                year: remote
                    .and_then(|detail| detail.year.clone())
                    .map(SharedString::from)
                    .or_else(|| (album.year > 0).then(|| album.year.to_string().into())),
                artwork: album
                    .artwork
                    .clone()
                    .or_else(|| remote.and_then(|detail| detail.artwork.clone())),
                browse_id: remote.map(|detail| detail.id.clone()),
                artist_key: remote
                    .and_then(|detail| detail.artist_key.clone())
                    .or_else(|| album.artist_key.clone()),
            }),
            (None, Some(detail)) => Some(Self {
                id: detail.id.clone(),
                title: detail.title.clone().into(),
                artist: detail.subtitle.clone().unwrap_or_default().into(),
                year: detail.year.clone().map(SharedString::from),
                artwork: detail.artwork.clone(),
                browse_id: Some(detail.id.clone()),
                artist_key: detail.artist_key.clone(),
            }),
            (None, None) => None,
        }
    }
}

/// Rows split per disc, or one unlabelled group when the album has only one.
fn by_disc(rows: &[TrackInfo]) -> Vec<(Option<u32>, Vec<TrackInfo>)> {
    let discs: Vec<u32> = {
        let mut discs: Vec<u32> = rows
            .iter()
            .map(|row| row.disc_number.unwrap_or(1))
            .collect();
        discs.dedup();
        discs
    };
    if discs.len() < 2 {
        return match rows.is_empty() {
            true => Vec::new(),
            false => vec![(None, rows.to_vec())],
        };
    }
    let mut groups: Vec<(Option<u32>, Vec<TrackInfo>)> = Vec::new();
    for row in rows {
        let disc = row.disc_number.unwrap_or(1);
        match groups.last_mut() {
            Some((Some(current), chunk)) if *current == disc => chunk.push(row.clone()),
            _ => groups.push((Some(disc), vec![row.clone()])),
        }
    }
    groups
}

async fn collect_album_tracks(
    api: std::sync::Arc<dyn api::KopuzApi>,
    ids: Vec<String>,
) -> Vec<TrackInfo> {
    let mut tracks = Vec::new();
    for id in ids {
        if let Ok(page) = api.album_tracks(id, store::all()).await {
            tracks.extend(page.items);
        }
    }
    tracks
}

fn point_tracks(query: &Entity<Query<Vec<TrackInfo>>>, ids: Vec<String>, cx: &mut App) {
    query.update(cx, |query, cx| {
        query.set_fetch(
            move |api| {
                let ids = ids.clone();
                async move { Ok(collect_album_tracks(api, ids).await) }
            },
            cx,
        )
    });
}

fn point_remote(query: &Entity<Query<Option<CatalogDetail>>>, id: Option<String>, cx: &mut App) {
    query.update(cx, |query, cx| {
        query.set_fetch(
            move |api| {
                let id = id.clone();
                async move {
                    let Some(id) = id else {
                        return Ok(None);
                    };
                    api.catalog_detail(CatalogDetailRequest::new(CatalogItemKind::Album, id))
                        .await
                        .map(Some)
                }
            },
            cx,
        )
    });
}

/// The cover's picture, named `Artist - Album` when it is saved.
fn picture(album: &AlbumInfo) -> Option<Picture> {
    Picture::new(
        album.artwork.as_ref(),
        Pictured::Album {
            artist: album.artist.clone().into(),
            title: album.title.clone().into(),
        },
    )
}

/// What a right-click on the cover offers: the album's own menu, as on its card.
fn cover_menu(album: &AlbumInfo) -> components::Menu {
    menus::menu_for(Target::Album {
        id: album.id.clone().into(),
        title: album.title.clone().into(),
    })
}
