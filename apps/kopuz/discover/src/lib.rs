//! Discover: the browsing destination. The source's mixes and pages, the library's forgotten corners, and what a query turned up.

mod detail;
mod genres;
mod landing;
mod page;
mod results;
mod rows;
mod search;
mod shelves;
mod states;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use api::{AlbumInfo, ApiError, KopuzApi, SearchFilter, SearchRequest, SearchResults, Table};
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
use crate::search::CatalogSearch;
use store::Playing;

pub use detail::{CatalogArtist, CatalogPlaylist};
pub use genres::Genre;
pub use page::{CatalogChips, CatalogPage};

/// Results that answer the previous query stay up at this opacity rather than blanking.
const FADED: f32 = 0.55;

type Answer = Pin<Box<dyn Future<Output = Result<SearchResults, ApiError>> + Send>>;

/// The library read behind one query.
fn search_fetch(query: String) -> impl Fn(Arc<dyn KopuzApi>) -> Answer + 'static {
    move |api| {
        let query = query.clone();
        Box::pin(async move { api.search(SearchRequest::new(query)).await })
    }
}

/// The browsing destination: the landing's rows when nothing is asked for, results when something is.
pub struct Discover {
    landing: Entity<Landing>,

    // What the palette put in the box, and what came back for it.
    query: SharedString,
    // The source's search filters; with any, it answers searches as shelves rather than a library listing.
    filters: Vec<SearchFilter>,
    source: Option<String>,
    // The library query last asked, none while the search is the source's own.
    asked: Option<String>,
    catalog: Entity<CatalogSearch>,
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
            Query::held(
                &[Table::Tracks, Table::Albums],
                SearchResults::default(),
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
                this.ask(cx);
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
                    let radio = SourcesModel::global(cx)
                        .read(cx)
                        .capabilities()
                        .search_radio;
                    match keys.get(*index).cloned().filter(|_| radio) {
                        // A hit seeds a radio rather than queueing the unrelated matches around it.
                        Some(key) => actions::start_track_radio(key, cx),
                        None => actions::play_listing(keys, *index, cx),
                    }
                }
                TrackListEvent::Delete(keys) => genres::delete_tracks(keys.clone(), cx),
                _ => {}
            }),
            cx.observe(&sources, |this, _, cx| {
                let actions = genres::row_actions(cx);
                this.tracks
                    .update(cx, |list, cx| list.set_actions(actions, cx));
                this.follow_source(cx);
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

        let mut discover = Self {
            landing: cx.new(|cx| Landing::new(window, cx)),
            query,
            filters: Vec::new(),
            source: None,
            asked: None,
            catalog: cx.new(CatalogSearch::new),
            results,
            tracks,
            result_albums: Vec::new(),
            result_artists: Vec::new(),
            playing: Playing::default(),
            _subscriptions: subscriptions,
        };
        discover.follow_source(cx);
        discover.ask(cx);
        discover
    }

    /// A new source, or new filters on it, is a new search for whatever the box holds.
    fn follow_source(&mut self, cx: &mut Context<Self>) {
        let (source, filters) = {
            let sources = SourcesModel::global(cx).read(cx);
            let source = sources.active().map(|source| source.id.clone());
            (source, sources.capabilities().search_filters)
        };
        if source == self.source && filters == self.filters {
            return;
        }
        if source != self.source {
            self.catalog.update(cx, |search, cx| search.forget(cx));
        }
        self.source = source;
        self.filters = filters;
        self.ask(cx);
        cx.notify();
    }

    fn is_catalog(&self) -> bool {
        !self.filters.is_empty()
    }

    /// Hand the query to whichever search the source answers; the other one stops asking.
    fn ask(&mut self, cx: &mut Context<Self>) {
        let query = self.query.trim().to_string();
        let catalog = self.is_catalog();
        let (shelved, filters) = match catalog {
            true => (query.clone(), self.filters.clone()),
            false => (String::new(), Vec::new()),
        };
        self.catalog
            .update(cx, |search, cx| search.set(shelved, filters, cx));
        let wanted = (!catalog && !query.is_empty()).then(|| query.to_lowercase());
        if wanted == self.asked {
            return;
        }
        self.asked = wanted.clone();
        self.results.update(cx, |results, cx| match wanted {
            Some(query) => results.set_fetch(search_fetch(query), cx),
            None => results.hold(SearchResults::default(), cx),
        });
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
        if self.is_catalog() {
            return self.page(self.catalog.clone().into_any_element(), cx);
        }
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

        self.page(body, cx)
    }

    fn page(&self, body: AnyElement, cx: &mut Context<Self>) -> AnyElement {
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
