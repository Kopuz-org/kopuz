//! Discover: the browsing destination. The source's mixes and pages, the library's forgotten corners, and what a query turned up.

mod detail;
mod genres;
mod landing;
mod page;
mod results;
mod rows;
mod shelves;
mod states;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use api::{AlbumInfo, ApiError, KopuzApi, SearchRequest, SearchResults, Table};
use components::format::{t, t_with};
use components::{
    Chip, ChipSize, EmptyState, Heading, Loading, PageHeader, SectionHeading, TrackList,
    TrackListEvent, icons,
};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled, Subscription, Window, div,
};
use store::{Chrome, PlayerModel, Query, SourcesModel, actions};

use crate::landing::Landing;
use store::Playing;

pub use detail::{CatalogArtist, CatalogPlaylist};
pub use genres::Genre;
pub use page::CatalogPage;

/// Results that answer the previous query stay up at this opacity rather than blanking.
const FADED: f32 = 0.55;

type Answer = Pin<Box<dyn Future<Output = Result<SearchResults, ApiError>> + Send>>;

/// The read behind one query; a blank one answers with nothing rather than asking the daemon.
fn search_fetch(query: String) -> impl Fn(Arc<dyn KopuzApi>) -> Answer + 'static {
    move |api| {
        let query = query.clone();
        Box::pin(async move {
            if query.is_empty() {
                return Ok(SearchResults::default());
            }
            api.search(SearchRequest::new(query)).await
        })
    }
}

/// The browsing destination: the landing's rows when nothing is asked for, results when something is.
pub struct Discover {
    landing: Entity<Landing>,

    // What the palette put in the box, and what came back for it.
    query: SharedString,
    results: Entity<Query<SearchResults>>,
    tracks: Entity<TrackList>,
    result_albums: Vec<AlbumInfo>,
    result_artists: Vec<(String, SharedString)>,
    playing: Playing,

    _subscriptions: Vec<Subscription>,
}

impl Discover {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = Chrome::global(cx).read(cx).search().clone();
        let results = cx.new(|cx| {
            Query::new(
                &[Table::Tracks, Table::Albums],
                search_fetch(query.trim().to_lowercase()),
                cx,
            )
        });
        let tracks = cx.new(|cx| {
            TrackList::new(cx)
                .with_options(components::TrackListOptions::default())
                .with_actions(genres::row_actions(cx))
        });
        commands::bind_track_list(&tracks, cx);
        let chrome = Chrome::global(cx);
        let sources = SourcesModel::global(cx);
        let player = PlayerModel::global(cx);

        let subscriptions = vec![
            cx.observe(&chrome, |this, chrome, cx| {
                let asked = chrome.read(cx).search().clone();
                if asked == this.query {
                    return;
                }
                this.query = asked;
                let fetch = search_fetch(this.query.trim().to_lowercase());
                this.results
                    .update(cx, |results, cx| results.set_fetch(fetch, cx));
                cx.notify();
            }),
            cx.observe(&results, |this, results, cx| {
                let (tracks, albums) = match results.read(cx).value() {
                    Some(found) => (found.tracks.clone(), found.albums.clone()),
                    None => (Vec::new(), Vec::new()),
                };
                this.result_artists = results::artists_of(&tracks, &albums);
                this.result_albums = albums;
                this.tracks.update(cx, |list, cx| list.set_rows(tracks, cx));
                cx.notify();
            }),
            cx.subscribe(&tracks, |_, list, event: &TrackListEvent, cx| match event {
                TrackListEvent::Play(index) => {
                    let keys = list.read(cx).keys();
                    actions::play_tracks(keys, Some(*index as u32), cx);
                }
                TrackListEvent::Delete(keys) => genres::delete_tracks(keys.clone(), cx),
                _ => {}
            }),
            cx.observe(&sources, |this, _, cx| {
                let actions = genres::row_actions(cx);
                this.tracks
                    .update(cx, |list, cx| list.set_actions(actions, cx));
            }),
            // The player notifies on every tick, so only a change of what is marked is worth a frame.
            cx.observe(&player, |this, player, cx| {
                let marked = player.read(cx).track().map(Playing::of);
                let marked = marked.unwrap_or_default();
                if marked == this.playing {
                    return;
                }
                this.playing = marked;
                cx.notify();
            }),
        ];

        Self {
            landing: cx.new(|cx| Landing::new(window, cx)),
            query,
            results,
            tracks,
            result_albums: Vec::new(),
            result_artists: Vec::new(),
            playing: Playing::default(),
            _subscriptions: subscriptions,
        }
    }

    fn clear_search(&mut self, cx: &mut Context<Self>) {
        Chrome::global(cx).update(cx, |chrome, cx| chrome.set_search("", cx));
    }

    /// The chip that puts the query back where it came from.
    fn clear_chip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Chip::new("discover-clear-search")
            .icon(icons::CLOSE)
            .label(t("discover_clear_search"))
            .size(ChipSize::Medium)
            .on_click(cx.listener(|this, _, _, cx| this.clear_search(cx)))
    }

    /// What one query turned up: the strips over the list of matching tracks.
    fn results_view(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (failure, settled, stale) = {
            let held = self.results.read(cx);
            (
                held.error().map(states::read_failure),
                held.is_settled(),
                held.is_stale(),
            )
        };
        let found = self.tracks.read(cx).rows().len();
        let nothing = found == 0 && self.result_albums.is_empty();

        let body = if let Some(message) = failure {
            EmptyState::failed(
                message,
                cx.listener(|this, _, _, cx| {
                    this.results.update(cx, |results, cx| results.refresh(cx));
                }),
            )
            .into_any_element()
        } else if !settled {
            Loading::new().into_any_element()
        } else if nothing {
            div()
                .flex_1()
                .min_h_0()
                .child(
                    EmptyState::new(t_with(
                        "no_results_found",
                        &[("query", self.query.to_string().into())],
                    ))
                    .detail(t("discover_no_results_hint"))
                    .icon(icons::SEARCH),
                )
                .into_any_element()
        } else {
            let albums = results::album_strip(&self.result_albums, t("albums"), &self.playing);
            let artists = results::artist_strip(&self.result_artists, t("artists"), &self.playing);
            v_flex()
                .flex_1()
                .min_h_0()
                // The held rows answer the previous query: dim them rather than blanking the page.
                .when(stale, |body| body.opacity(FADED))
                .children(albums)
                .children(artists)
                .when(found > 0, |body| {
                    body.child(
                        div()
                            .flex_shrink_0()
                            .mb_4()
                            .child(SectionHeading::new(t("tracks")).heading(Heading::Band)),
                    )
                    .child(div().flex_1().min_h_0().pb_8().child(self.tracks.clone()))
                })
                .into_any_element()
        };

        components::page()
            .child(
                PageHeader::new(t_with(
                    "discover_results_for",
                    &[("query", self.query.to_string().into())],
                ))
                .action(self.clear_chip(cx)),
            )
            .child(body)
            .into_any_element()
    }
}

impl store::Freeze for Discover {
    fn freeze(&mut self, cx: &mut Context<Self>) {
        self._subscriptions.clear();
        self.landing.update(cx, |landing, cx| landing.freeze(cx));
    }
}

impl Render for Discover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("discover");
        if !self.query.trim().is_empty() {
            return self.results_view(window, cx);
        }
        self.landing.clone().into_any_element()
    }
}
