//! One artist: the tracks showcase, or their albums.

use std::rc::Rc;

use api::{AlbumInfo, AlbumPage, ArtworkTarget, SourceCapabilities, Table, TrackInfo, TrackPage};
use components::album_collection::{available_fields, compare_text};
use components::format::{t, t_with, total_duration};
use components::{
    AlbumCollection, DetailActions, DetailHeader, EmptyState, GridMode, Loading, Picture,
    TrackActions, TrackHeader, TrackList, TrackListEvent, TrackListOptions, icons, menus,
};
use config::{AlbumSortField, AlbumViewMode, SortCriterion, SortDirection};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Pixels,
    Render, SharedString, Styled as _, Subscription, Window, div, px, rems,
};
use store::{
    ConfigModel, DownloadsModel, LayoutModel, Offline, Pictured, PlayerModel, Query, SourcesModel,
    Target, actions,
};

use crate::controls::{self, ViewModeKey};

/// `w-64 h-64`: the showcase photo.
const PHOTO: Pixels = px(256.);

/// A sortable column of the tracks view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Title,
    Artist,
    Album,
    Duration,
}

impl Column {
    /// The table column it sorts, by the id the shared header names it with.
    fn id(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Duration => "duration",
        }
    }

    fn parse(id: &str) -> Option<Self> {
        [Self::Title, Self::Artist, Self::Album, Self::Duration]
            .into_iter()
            .find(|column| column.id() == id)
    }
}

/// One artist from the library: tracks and albums.
pub struct ArtistDetail {
    key: String,
    tracks: Entity<Query<TrackPage>>,
    detail: Entity<Query<api::ArtistDetail>>,
    list: Entity<TrackList>,
    column: Option<(Column, SortDirection)>,
    // Derived once per change, not once per frame.
    keys: Rc<[String]>,
    total_ms: Option<u64>,
    offline: Offline,
    billed: Entity<Query<AlbumPage>>,
    // The order the albums were last asked for; the daemon sorts, so a new one is a new read.
    asked: Vec<SortCriterion<AlbumSortField>>,
    albums: Rc<[AlbumInfo]>,
    album_fields: Vec<AlbumSortField>,
    collection: AlbumCollection,
    _subscriptions: Vec<Subscription>,
}

impl ArtistDetail {
    pub fn new(key: String, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let artist = key.clone();
        let tracks = cx.new(|cx| {
            Query::new(
                &[Table::Tracks],
                move |api| {
                    let artist = artist.clone();
                    async move { api.artist_tracks(artist, store::all()).await }
                },
                cx,
            )
        });
        let artist = key.clone();
        let detail = cx.new(|cx| {
            Query::new(
                &[Table::Tracks, Table::Albums, Table::Artists],
                move |api| {
                    let artist = artist.clone();
                    async move { api.artist(artist).await }
                },
                cx,
            )
        });
        let asked = controls::criteria::<AlbumSortField>(cx);
        let billed = cx.new(|cx| {
            Query::new(
                &[Table::Albums, Table::Tracks],
                controls::album_listing(key.clone(), &asked),
                cx,
            )
        });
        let list = cx.new(TrackList::new);
        commands::bind_track_list(&list, cx);
        let subscriptions = vec![
            cx.observe(&tracks, |this, _, cx| this.sync_rows(cx)),
            cx.observe(&billed, |this, _, cx| this.rebuild_albums(cx)),
            cx.observe(&LayoutModel::global(cx), |this, _, cx| {
                this.reorder_albums(cx);
                cx.notify();
            }),
            cx.observe(&ConfigModel::global(cx), |this, _, cx| this.sync_rows(cx)),
            cx.observe(&SourcesModel::global(cx), |this, _, cx| this.sync_rows(cx)),
            cx.observe(&DownloadsModel::global(cx), |this, _, cx| {
                this.refresh_offline(cx);
                cx.notify();
            }),
            cx.observe(&PlayerModel::global(cx), |_, _, cx| cx.notify()),
            cx.subscribe(&list, Self::on_list_event),
        ];
        Self {
            key,
            tracks,
            detail,
            list,
            column: None,
            keys: Rc::from([]),
            total_ms: None,
            offline: Offline::default(),
            billed,
            asked,
            albums: Rc::from([]),
            album_fields: Vec::new(),
            collection: AlbumCollection::new("artist-albums-scroll"),
            _subscriptions: subscriptions,
        }
    }

    fn on_list_event(
        &mut self,
        list: Entity<TrackList>,
        event: &TrackListEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            TrackListEvent::Play(index) => {
                let keys = list.read(cx).keys();
                actions::play_tracks(keys, Some(*index as u32), cx);
            }
            TrackListEvent::Delete(keys) => delete_tracks(keys.clone(), cx),
            _ => cx.notify(),
        }
    }

    fn sync_rows(&mut self, cx: &mut Context<Self>) {
        let options = TrackListOptions {
            show_index: true,
            show_cover: true,
            show_album: true,
            track_numbers: false,
            selectable: true,
        };
        let row_actions = TrackActions {
            remove_from_playlist: false,
            delete: capabilities(cx).delete_from_disk,
            reorder: false,
        };
        let rows = self.rows(cx);
        self.keys = rows.iter().map(|row| row.key.clone()).collect();
        self.total_ms = api::total_duration_secs(&rows).map(|secs| secs * 1000);
        self.refresh_offline(cx);
        self.list.update(cx, |list, cx| {
            list.set_options(options, cx);
            list.set_actions(row_actions, cx);
            list.set_rows(rows, cx);
        });
        cx.notify();
    }

    fn refresh_offline(&mut self, cx: &App) {
        self.offline = DownloadsModel::global(cx).read(cx).offline(&self.keys);
    }

    /// The artist's albums as the daemon listed them, and which orders are worth offering.
    fn rebuild_albums(&mut self, cx: &mut Context<Self>) {
        let albums: Vec<AlbumInfo> = self
            .billed
            .read(cx)
            .value()
            .map(|page| page.albums.clone())
            .unwrap_or_default();
        self.album_fields = available_fields(&albums, false);
        self.albums = albums.into();
        cx.notify();
    }

    /// Ask the daemon again when the albums' order changed.
    fn reorder_albums(&mut self, cx: &mut Context<Self>) {
        let wanted = controls::criteria::<AlbumSortField>(cx);
        if wanted == self.asked {
            return;
        }
        let listing = controls::album_listing(self.key.clone(), &wanted);
        self.billed
            .update(cx, |query, cx| query.set_fetch(listing, cx));
        self.asked = wanted;
    }

    fn rows(&self, cx: &App) -> Vec<TrackInfo> {
        // TODO(offline): filter to downloaded tracks when offline.
        let mut rows = self
            .tracks
            .read(cx)
            .value()
            .map(|page| page.items.clone())
            .unwrap_or_default();
        if let Some((column, direction)) = self.column {
            sort_tracks(&mut rows, column, direction);
        }
        rows
    }

    fn toggle_column(&mut self, column: Column, cx: &mut Context<Self>) {
        self.column = match self.column {
            Some((current, SortDirection::Asc)) if current == column => {
                Some((column, SortDirection::Desc))
            }
            Some((current, SortDirection::Desc)) if current == column => None,
            _ => Some((column, SortDirection::Asc)),
        };
        self.sync_rows(cx);
    }

    fn artwork(&self, cx: &App) -> Option<api::ArtworkRef> {
        self.detail.read(cx).value()?.info.artwork.clone()
    }

    /// Blank until the daemon answers: the key carries no display name.
    fn name(&self, cx: &App) -> SharedString {
        self.detail
            .read(cx)
            .value()
            .map(|detail| SharedString::from(detail.info.name.clone()))
            .unwrap_or_default()
    }
}

impl Render for ArtistDetail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let albums_view = controls::albums_first(cx);
        let failed = {
            let detail = self.detail.read(cx);
            detail
                .value()
                .is_none()
                .then(|| detail.error().cloned())
                .flatten()
        };
        let body = if let Some(error) = failed {
            let message = match error.code {
                api::ErrorCode::NotFound => t("artist_not_found"),
                _ => t_with(
                    "artist_load_failed",
                    &[("error", store::human(&error).to_string().into())],
                ),
            };
            EmptyState::new(message).icon(icons::MIC).into_any_element()
        } else if albums_view {
            self.albums_view(window, cx)
        } else {
            self.tracks_view(cx)
        };
        components::page()
            .gap_6()
            .child(
                h_flex()
                    .w_full()
                    .justify_end()
                    .items_center()
                    .child(controls::view_order_toggle(albums_view)),
            )
            .child(body)
    }
}

impl ArtistDetail {
    fn tracks_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.keys.len();
        let selecting = self.list.read(cx).is_selecting();
        let settled = self.tracks.read(cx).is_settled();
        let body = if !settled && count == 0 {
            Loading::new().into_any_element()
        } else if count == 0 {
            EmptyState::new(t("no_tracks_found"))
                .icon(icons::MUSIC)
                .into_any_element()
        } else {
            v_flex()
                .flex_1()
                .min_h_0()
                .child(self.column_header(count, cx))
                .child(div().flex_1().min_h_0().child(self.list.clone()))
                .into_any_element()
        };
        v_flex()
            .relative()
            .flex_1()
            .min_h_0()
            .gap_12()
            .child(self.hero(cx))
            .child(body)
            .when(selecting, |view| view.child(self.selection_bar(cx)))
            .into_any_element()
    }

    fn hero(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let capabilities = capabilities(cx);
        let artwork = self.artwork(cx);
        let artist = self.key.clone();
        let name = self.name(cx);
        let picture = Picture::new(artwork.as_ref(), Pictured::Artist(name.clone()));
        let menu = menus::menu_for(Target::Artist {
            key: self.key.clone(),
            name: name.clone(),
        });
        DetailHeader::new(name)
            .artwork(artwork.as_ref())
            .placeholder(icons::MIC)
            .cover_menu(menu, picture)
            .size(PHOTO)
            .when(capabilities.edit_tags, |header| {
                header.on_cover("artist-photo", Some(t("change_photo")), move |_, cx| {
                    store::pick_and_upload(ArtworkTarget::Artist(artist.clone()), cx)
                })
            })
            .meta(t_with(
                "showcase_song_count",
                &[("count", self.keys.len().into())],
            ))
            .when_some(self.total_ms, |header, total| {
                header.meta(total_duration(total))
            })
            .action(
                DetailActions::new("artist", self.keys.clone())
                    .offline(self.offline)
                    .shuffle(commands::set_shuffle)
                    .play(commands::play_all)
                    .download(commands::set_offline),
            )
            .into_any_element()
    }

    fn column_header(&mut self, count: usize, cx: &mut Context<Self>) -> AnyElement {
        let list = self.list.read(cx);
        let all_selected = count > 0 && count == list.selected_count();
        let active = self
            .column
            .map(|(column, direction)| (column.id(), direction == SortDirection::Desc));
        TrackHeader::new("artist-header", list.layout())
            .sort(
                active,
                |id| Column::parse(id).is_some(),
                cx.listener(|this, id: &str, _, cx| {
                    if let Some(column) = Column::parse(id) {
                        this.toggle_column(column, cx);
                    }
                }),
            )
            .select_all(
                all_selected,
                cx.listener(move |this, checked: &bool, _, cx| {
                    this.list.update(cx, |list, cx| {
                        if *checked {
                            list.select_all(cx);
                        } else {
                            list.clear_selection(cx);
                        }
                    });
                }),
            )
            .into_any_element()
    }

    fn selection_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (keys, first) = {
            let list = self.list.read(cx);
            let keys = list.selected_keys();
            let first = list
                .rows()
                .iter()
                .find(|row| keys.first() == Some(&row.key))
                .map(|row| row.title.clone())
                .unwrap_or_default();
            (keys, first)
        };
        let picked = keys.iter().cloned().map(SharedString::from).collect();
        let (acted, cancelled) = (self.list.downgrade(), self.list.downgrade());
        let bar = commands::selection_bar(
            "artist-selection",
            keys.len(),
            Target::selection(picked, first),
            move |command, target, window, cx| {
                command.perform(target, window, cx);
                acted.update(cx, |list, cx| list.clear_selection(cx)).ok();
            },
            cx,
        )
        .lift(rems(6.))
        .on_cancel(move |_, cx| {
            cancelled
                .update(cx, |list, cx| list.clear_selection(cx))
                .ok();
        });
        let bar = match capabilities(cx).delete_from_disk {
            true => bar.on_delete(move |_, cx| delete_tracks(keys.clone(), cx)),
            false => bar,
        };
        bar.into_any_element()
    }

    fn albums_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mode = match controls::view_mode(ViewModeKey::ArtistAlbums, cx) {
            AlbumViewMode::Grid => GridMode::Tiles,
            AlbumViewMode::List => GridMode::Rows,
        };
        let settled = self.billed.read(cx).is_settled();
        let empty = self.albums.is_empty();

        let body = if !settled && empty {
            Loading::new().into_any_element()
        } else if empty {
            EmptyState::new(t("no_albums_found"))
                .icon(icons::DISC)
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .child(
                    self.collection
                        .bands::<Self>(self.albums.clone(), mode, window, cx),
                )
                .into_any_element()
        };

        v_flex()
            .flex_1()
            .min_h_0()
            .gap_12()
            .child(self.hero(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_end()
                            .gap_2()
                            .items_center()
                            .child(controls::view_mode_toggle(ViewModeKey::ArtistAlbums, cx))
                            .child(controls::sort_control(
                                "artist-album-sort",
                                self.album_fields.clone(),
                                cx,
                            )),
                    )
                    .child(body),
            )
            .into_any_element()
    }
}

fn delete_tracks(keys: Vec<String>, cx: &mut App) {
    let from_disk = capabilities(cx).delete_from_disk;
    actions::run(cx, move |api| async move {
        api.delete_tracks(keys, from_disk).await
    });
}

fn capabilities(cx: &App) -> SourceCapabilities {
    let model = SourcesModel::global(cx);
    model.read(cx).capabilities()
}

fn sort_tracks(tracks: &mut [TrackInfo], column: Column, direction: SortDirection) {
    tracks.sort_by(|left, right| {
        let ordering = match column {
            Column::Title => compare_text(&left.title, &right.title),
            Column::Artist => compare_text(&left.artist, &right.artist),
            Column::Album => compare_text(&left.album, &right.album),
            Column::Duration => left.duration_ms.cmp(&right.duration_ms),
        };
        let ordering = match direction {
            SortDirection::Asc => ordering,
            SortDirection::Desc => ordering.reverse(),
        };
        match column {
            Column::Album => ordering
                .then_with(|| {
                    left.disc_number
                        .unwrap_or(0)
                        .cmp(&right.disc_number.unwrap_or(0))
                })
                .then_with(|| {
                    left.track_number
                        .unwrap_or(0)
                        .cmp(&right.track_number.unwrap_or(0))
                })
                .then_with(|| compare_text(&left.title, &right.title)),
            _ => ordering,
        }
    });
}
