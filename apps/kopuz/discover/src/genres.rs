//! A genre's page: every track filed under it, and the genres the library's albums are filed under.

use std::collections::HashMap;
use std::rc::Rc;

use api::{AlbumInfo, ArtworkRef, Table, TrackInfo, TrackPage};
use components::album_collection::compare_text;
use components::format::{t, t_with};
use components::{
    DetailActions, EmptyState, Loading, Look, PageHeader, TrackActions, TrackHeader, TrackList,
    TrackListEvent, TrackListOptions, icons,
};
use config::SortDirection;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled, Subscription, Window, div,
};
use store::{Query, SourcesModel, actions, all};

use crate::states;

/// A sortable column of the genre's tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Title,
    Artist,
    Album,
    Duration,
}

impl Column {
    const ALL: [Self; 4] = [Self::Title, Self::Artist, Self::Album, Self::Duration];

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
        Self::ALL.into_iter().find(|column| column.id() == id)
    }
}

/// A press on a heading sorts by it ascending, then descending, then back to the order the daemon gave.
fn next_sort(
    current: Option<(Column, SortDirection)>,
    pressed: Column,
) -> Option<(Column, SortDirection)> {
    match current {
        Some((column, SortDirection::Asc)) if column == pressed => {
            Some((pressed, SortDirection::Desc))
        }
        Some((column, SortDirection::Desc)) if column == pressed => None,
        _ => Some((pressed, SortDirection::Asc)),
    }
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
        // An album keeps its own running order inside the sort.
        match column {
            Column::Album => ordering
                .then_with(|| left.disc_number.cmp(&right.disc_number))
                .then_with(|| left.track_number.cmp(&right.track_number)),
            _ => ordering,
        }
    });
}

/// Every genre the albums are filed under, by name, each with the first cover filed under it.
pub(crate) fn genres_of(albums: &[AlbumInfo]) -> Vec<(SharedString, Option<ArtworkRef>)> {
    let mut covers: HashMap<&str, Option<&ArtworkRef>> = HashMap::new();
    for album in albums {
        for genre in album.genre.split(['/', ';', ',']) {
            let genre = genre.trim();
            if genre.is_empty() {
                continue;
            }
            let cover = covers.entry(genre).or_default();
            if cover.is_none() {
                *cover = album.artwork.as_ref();
            }
        }
    }
    let mut genres: Vec<(SharedString, Option<ArtworkRef>)> = covers
        .into_iter()
        .map(|(name, cover)| (SharedString::from(name.to_string()), cover.cloned()))
        .collect();
    genres.sort_by(|left, right| left.0.cmp(&right.0));
    genres
}

fn track_count(count: usize) -> SharedString {
    t_with("track_count", &[("count", count.into())])
}

pub(crate) fn row_actions(cx: &App) -> TrackActions {
    TrackActions {
        delete: SourcesModel::global(cx)
            .read(cx)
            .capabilities()
            .delete_from_disk,
        ..Default::default()
    }
}

/// Delete gated on the active source, where the Dioxus frontend always asked for the disk.
pub(crate) fn delete_tracks(keys: Vec<String>, cx: &mut App) {
    let from_disk = SourcesModel::global(cx)
        .read(cx)
        .capabilities()
        .delete_from_disk;
    actions::run(cx, move |api| async move {
        api.delete_tracks(keys, from_disk).await
    });
}

/// Every track of one genre.
pub struct Genre {
    name: SharedString,
    tracks: Entity<Query<TrackPage>>,
    list: Entity<TrackList>,
    column: Option<(Column, SortDirection)>,
    // The rows' keys, taken once per answer rather than per frame.
    keys: Rc<[String]>,
    _subscriptions: Vec<Subscription>,
}

impl Genre {
    pub fn new(name: SharedString, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let genre = name.to_string();
        let tracks = cx.new(|cx| {
            Query::new(
                &[Table::Tracks],
                move |api| {
                    let genre = genre.clone();
                    async move { api.genre_tracks(genre, all()).await }
                },
                cx,
            )
        });
        let list = cx.new(|cx| {
            TrackList::new(cx)
                .with_options(TrackListOptions::default())
                .with_actions(row_actions(cx))
        });
        commands::bind_track_list(&list, cx);
        let sources = SourcesModel::global(cx);

        let subscriptions = vec![
            cx.observe(&tracks, |this, _, cx| this.sync_rows(cx)),
            cx.subscribe(&list, |_, list, event: &TrackListEvent, cx| match event {
                TrackListEvent::Play(index) => {
                    let keys = list.read(cx).keys();
                    actions::play_listing(keys, *index, cx);
                }
                TrackListEvent::Delete(keys) => delete_tracks(keys.clone(), cx),
                _ => {}
            }),
            cx.observe(&sources, |this, _, cx| {
                let actions = row_actions(cx);
                this.list
                    .update(cx, |list, cx| list.set_actions(actions, cx));
            }),
        ];

        Self {
            name,
            tracks,
            list,
            column: None,
            keys: Rc::from([]),
            _subscriptions: subscriptions,
        }
    }
}

impl Genre {
    fn sync_rows(&mut self, cx: &mut Context<Self>) {
        let mut rows = self
            .tracks
            .read(cx)
            .value()
            .map(|page| page.items.clone())
            .unwrap_or_default();
        if let Some((column, direction)) = self.column {
            sort_tracks(&mut rows, column, direction);
        }
        self.keys = rows.iter().map(|row| row.key.clone()).collect();
        self.list.update(cx, |list, cx| list.set_rows(rows, cx));
        cx.notify();
    }

    fn column_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self
            .column
            .map(|(column, direction)| (column.id(), direction == SortDirection::Desc));
        TrackHeader::new("genre-header", self.list.read(cx).layout()).sort(
            active,
            |id| Column::parse(id).is_some(),
            cx.listener(|this, id: &str, _, cx| {
                if let Some(column) = Column::parse(id) {
                    this.column = next_sort(this.column, column);
                    this.sync_rows(cx);
                }
            }),
        )
    }
}

impl Render for Genre {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("genre");
        let count = self.keys.len();
        let failure = self.tracks.read(cx).error().map(states::read_failure);
        let settled = self.tracks.read(cx).is_settled();
        let body = if let Some(message) = failure {
            EmptyState::failed(
                message,
                cx.listener(|this, _, _, cx| {
                    this.tracks.update(cx, |tracks, cx| tracks.refresh(cx));
                }),
            )
            .into_any_element()
        } else if !settled {
            Loading::new().into_any_element()
        } else if count == 0 {
            EmptyState::new(t("no_tracks_found"))
                .icon(icons::DISC)
                .into_any_element()
        } else {
            v_flex()
                .flex_1()
                .min_h_0()
                .child(self.column_header(cx))
                .child(div().flex_1().min_h_0().child(self.list.clone()))
                .into_any_element()
        };

        components::page()
            .pb_8()
            .child(
                PageHeader::new(self.name.clone())
                    .subtitle(track_count(count))
                    .when(count > 0, |header| {
                        header.action(
                            DetailActions::new("genre", self.keys.clone())
                                .look(Look::Compact)
                                .play(commands::play_all)
                                .shuffle_play(commands::shuffle_play),
                        )
                    }),
            )
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn album(genre: &str, cover: Option<&str>) -> AlbumInfo {
        AlbumInfo {
            genre: genre.to_string(),
            artwork: cover.map(|id| ArtworkRef::new(api::ArtworkTarget::Album(id.into()), 0)),
            ..AlbumInfo::default()
        }
    }

    fn track(title: &str, album: &str, disc: u32, number: u32) -> TrackInfo {
        TrackInfo {
            title: title.to_string(),
            album: album.to_string(),
            disc_number: Some(disc),
            track_number: Some(number),
            ..TrackInfo::default()
        }
    }

    fn titles(tracks: &[TrackInfo]) -> Vec<&str> {
        tracks.iter().map(|track| track.title.as_str()).collect()
    }

    #[test]
    fn a_genre_field_splits_on_every_separator_and_lists_each_name_once() {
        let albums = [
            album("Trip Hop / Electronic", None),
            album("electronic;Ambient, Trip Hop", None),
            album("  ", None),
        ];
        let names: Vec<SharedString> = genres_of(&albums).into_iter().map(|g| g.0).collect();
        assert_eq!(
            names,
            ["Ambient", "Electronic", "Trip Hop", "electronic"].map(SharedString::from)
        );
    }

    #[test]
    fn a_genre_takes_the_first_cover_filed_under_it() {
        let albums = [
            album("Jazz", None),
            album("Jazz", Some("/a.jpg")),
            album("Jazz", Some("/b.jpg")),
        ];
        assert_eq!(
            genres_of(&albums),
            [(
                SharedString::from("Jazz"),
                Some(ArtworkRef::new(
                    api::ArtworkTarget::Album("/a.jpg".into()),
                    0
                ))
            )]
        );
    }

    #[test]
    fn a_heading_sorts_up_then_down_then_lets_go() {
        let up = next_sort(None, Column::Title);
        assert_eq!(up, Some((Column::Title, SortDirection::Asc)));
        let down = next_sort(up, Column::Title);
        assert_eq!(down, Some((Column::Title, SortDirection::Desc)));
        assert_eq!(next_sort(down, Column::Title), None);
        assert_eq!(
            next_sort(down, Column::Artist),
            Some((Column::Artist, SortDirection::Asc))
        );
    }

    #[test]
    fn an_album_sort_keeps_each_album_in_running_order() {
        let mut tracks = [
            track("b2", "B", 1, 2),
            track("a2", "A", 2, 1),
            track("a1", "A", 1, 5),
            track("b1", "B", 1, 1),
        ];
        sort_tracks(&mut tracks, Column::Album, SortDirection::Asc);
        assert_eq!(titles(&tracks), ["a1", "a2", "b1", "b2"]);
        sort_tracks(&mut tracks, Column::Album, SortDirection::Desc);
        assert_eq!(titles(&tracks), ["b1", "b2", "a1", "a2"]);
    }
}
