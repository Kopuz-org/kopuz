//! Route to page view. Views are cached per route key, so going back restores a page as it was.

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::{AnyView, App, AppContext as _, Context, Entity, Render, Window};
use store::{Freeze, Route};

use crate::shell::Shell;

type Freezer = Rc<dyn Fn(&mut App)>;

/// A built page, and how to freeze it if it can be.
struct Page {
    view: AnyView,
    freeze: Option<Freezer>,
    // Settings reads the sources live, so a source switch has no reason to reset its tab.
    survives_switch: bool,
}

impl<T: Render> From<Entity<T>> for Page {
    fn from(page: Entity<T>) -> Self {
        Self {
            view: page.into(),
            freeze: None,
            survives_switch: false,
        }
    }
}

impl Page {
    fn freezable<T: Render + Freeze>(page: Entity<T>) -> Self {
        let held = page.clone();
        Self {
            view: page.into(),
            freeze: Some(Rc::new(move |cx: &mut App| {
                held.update(cx, |page, cx| page.freeze(cx))
            })),
            survives_switch: false,
        }
    }

    fn surviving_switch(mut self) -> Self {
        self.survives_switch = true;
        self
    }
}

#[derive(Default)]
pub struct Router {
    pages: HashMap<String, Page>,
    // Least recently visited first, so the cache can drop one page instead of all of them.
    order: Vec<String>,
}

// Enough for a back-and-forth between a grid and a few details before anything is rebuilt.
const CACHED_PAGES: usize = 8;

impl Router {
    /// The view for `route`, built on first visit.
    pub fn page(&mut self, route: &Route, window: &mut Window, cx: &mut Context<Shell>) -> AnyView {
        let key = route.key();
        if let Some(page) = self.pages.get(&key) {
            let page = page.view.clone();
            self.touch(key);
            return page;
        }
        // Evict the oldest, not the lot. A route key carries its parameters, so nine album visits
        // was enough to flush everything: going back then rebuilt the grid from scratch, scroll
        // position and all, and only ever on the ninth visit.
        while self.pages.len() >= CACHED_PAGES && !self.order.is_empty() {
            let oldest = self.order.remove(0);
            self.pages.remove(&oldest);
        }
        let page = build(route, window, cx);
        let view = page.view.clone();
        self.pages.insert(key.clone(), page);
        self.touch(key);
        view
    }

    /// Stop `view` following the app, if it is a page that can, so it can be held over its replacement.
    pub fn freeze(&self, view: &AnyView, cx: &mut App) {
        let freeze = self
            .pages
            .values()
            .find(|page| page.view.entity_id() == view.entity_id())
            .and_then(|page| page.freeze.clone());
        if let Some(freeze) = freeze {
            freeze(cx);
        }
    }

    fn touch(&mut self, key: String) {
        self.order.retain(|held| held != &key);
        self.order.push(key);
    }

    /// Drop cached pages, for a source switch that makes every page's contents wrong.
    pub fn clear(&mut self) {
        self.pages.retain(|_, page| page.survives_switch);
        self.order.retain(|key| self.pages.contains_key(key));
    }
}

fn build(route: &Route, window: &mut Window, cx: &mut Context<Shell>) -> Page {
    match route.clone() {
        Route::Home | Route::Downloader => {
            Page::freezable(cx.new(|cx| page_home::Home::new(window, cx)))
        }
        // One browsing destination: the landing, genres and results are the same entity.
        Route::Search | Route::Discover => {
            Page::freezable(cx.new(|cx| discover::Discover::new(window, cx)))
        }
        Route::Genre(name) => cx.new(|cx| discover::Genre::new(name, window, cx)).into(),
        Route::DiscoverPlaylist { id, title } => cx
            .new(|cx| discover::CatalogPlaylist::new(id, title, window, cx))
            .into(),
        Route::DiscoverArtist(key) => cx
            .new(|cx| discover::CatalogArtist::new(key, window, cx))
            .into(),
        Route::CatalogPage { id } => cx
            .new(|cx| discover::CatalogPage::new(id, window, cx))
            .into(),
        // Both library routes collapse to one cache key, so a chip does not rebuild the page.
        // The four old listings redirect here, and only reach this arm if one ever slips through.
        Route::Library
        | Route::LibraryView(_)
        | Route::Albums
        | Route::Artists
        | Route::Playlists
        | Route::Activity => Page::freezable(cx.new(|cx| library::Library::new(window, cx))),
        Route::Favorites => Page::freezable(cx.new(|cx| library::Library::favorites(window, cx))),
        Route::Album(id) => cx
            .new(|cx| page_albums::AlbumDetail::new(id, window, cx))
            .into(),
        // A source that presents artists itself gets its own profile.
        Route::Artist(key) => {
            let remote = store::SourcesModel::global(cx)
                .read(cx)
                .capabilities()
                .artists
                == api::ArtistPresentation::Remote;
            match remote {
                true => cx
                    .new(|cx| discover::CatalogArtist::new(key, window, cx))
                    .into(),
                false => cx
                    .new(|cx| page_artists::ArtistDetail::new(key, window, cx))
                    .into(),
            }
        }
        Route::PlaylistFolder(id) => cx
            .new(|cx| page_playlists::PlaylistFolder::new(id, window, cx))
            .into(),
        Route::Playlist(id) => cx
            .new(|cx| page_playlists::PlaylistDetail::new(id, window, cx))
            .into(),
        Route::Radio => cx.new(|cx| page_radio::Radio::new(window, cx)).into(),
        Route::Settings { section } => {
            Page::from(cx.new(|cx| page_settings::Settings::new(section, window, cx)))
                .surviving_switch()
        }
    }
}
