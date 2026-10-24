//! A genre's page: every track filed under it.

use std::rc::Rc;

use api::{Table, TrackPage};
use components::format::{t, t_with};
use components::{
    DetailActions, EmptyState, Loading, Look, PageHeader, TrackActions, TrackList, TrackListEvent,
    TrackListOptions, icons,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled, Subscription, Window, div,
};
use store::{Query, SourcesModel, actions, all};

use crate::states;

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
            cx.observe(&tracks, |this, tracks, cx| {
                let rows = tracks
                    .read(cx)
                    .value()
                    .map(|page| page.items.clone())
                    .unwrap_or_default();
                this.keys = rows.iter().map(|row| row.key.clone()).collect();
                this.list.update(cx, |list, cx| list.set_rows(rows, cx));
                cx.notify();
            }),
            cx.subscribe(&list, |_, list, event: &TrackListEvent, cx| match event {
                TrackListEvent::Play(index) => {
                    let keys = list.read(cx).keys();
                    actions::play_tracks(keys, Some(*index as u32), cx);
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
            keys: Rc::from([]),
            _subscriptions: subscriptions,
        }
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
            div()
                .flex_1()
                .min_h_0()
                .child(self.list.clone())
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
