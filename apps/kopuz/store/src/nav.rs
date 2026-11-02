use gpui_kit::{App, Context, Entity, SharedString};

/// Everywhere the app can be. Detail params travel in the route, so back navigation restores them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Route {
    Home,
    Search,
    Genre(SharedString),
    Discover,
    DiscoverPlaylist {
        id: SharedString,
        title: SharedString,
    },
    DiscoverArtist(String),
    /// A catalog page the source declares or a tile opens, read with `CatalogDetailRequest::page`.
    CatalogPage {
        id: SharedString,
    },
    Library,
    /// The library with a facet and a filter set on it, encoded so back and forward restore them.
    LibraryView(SharedString),
    Albums,
    Album(SharedString),
    Artists,
    Artist(String),
    Playlists,
    PlaylistFolder(SharedString),
    Playlist(SharedString),
    Favorites,
    Activity,
    Radio,
    Downloader,
    Settings {
        section: Option<SharedString>,
    },
}

impl Route {
    /// The artist page for a track's billed credit; a credit the daemon could not key opens nothing.
    pub fn artist_of(track: &api::TrackInfo) -> Option<Route> {
        let key = track.primary_credit()?.key.clone()?;
        Some(Route::Artist(key))
    }

    /// The page a queue's origin opens, for the "playing from" link; a context with no page has none.
    pub fn playing_from(context: &api::QueueContext) -> Option<Route> {
        match context {
            api::QueueContext::Album { id } => Some(Route::Album(id.clone().into())),
            api::QueueContext::Artist { artist } => Some(Route::Artist(artist.clone())),
            api::QueueContext::Genre { name } => Some(Route::Genre(name.clone().into())),
            api::QueueContext::Playlist { id } | api::QueueContext::PlaylistRadio { id } => {
                Some(Route::Playlist(id.clone().into()))
            }
            _ => None,
        }
    }

    /// The sidebar item this route lights up.
    pub fn root(&self) -> Route {
        match self {
            Route::Genre(_) => Route::Search,
            Route::LibraryView(_) => Route::Library,
            Route::Album(_) => Route::Albums,
            Route::Artist(_) => Route::Artists,
            Route::Playlist(_) | Route::PlaylistFolder(_) => Route::Playlists,
            Route::DiscoverPlaylist { .. }
            | Route::DiscoverArtist(_)
            | Route::CatalogPage { .. } => Route::Discover,
            Route::Settings { .. } => Route::Settings { section: None },
            other => other.clone(),
        }
    }

    /// Stable key for caching a page view and its scroll position.
    pub fn key(&self) -> String {
        // Every filter set is the same view: keying on the encoding would rebuild it per chip.
        match self {
            Route::Library | Route::LibraryView(_) => "Library".to_string(),
            other => format!("{other:?}"),
        }
    }

    /// The facet and filter set this route asks the library for, if it names one.
    pub fn library_view(&self) -> Option<&SharedString> {
        match self {
            Route::LibraryView(view) => Some(view),
            _ => None,
        }
    }

    /// Where a route that is no longer a place now lands, so an old link never reaches a 404.
    pub fn redirect(self) -> Route {
        match self {
            // The listings are facets of one library now, and most-played is a sort of it.
            Route::Activity => Route::Library,
            Route::Albums | Route::Artists | Route::Playlists => Route::Library,
            // Search and Discover are one destination; two keys would cache two of it.
            Route::Search => Route::Discover,
            // Until the URL tool is a dialog off the downloads popover, it has nowhere of its own.
            Route::Downloader => Route::Home,
            other => other,
        }
    }

    /// What a redirected route was asking for, for the view that has to honour it instead.
    pub fn intent(&self) -> Option<&'static str> {
        match self {
            Route::Activity => Some("activity"),
            Route::Downloader => Some("downloads"),
            _ => None,
        }
    }

    /// The route a nav destination opens; the names are `layout::DESTINATIONS`.
    pub fn destination(name: &str) -> Option<Route> {
        Some(match name {
            "home" => Route::Home,
            "library" => Route::Library,
            "discover" => Route::Discover,
            "radio" => Route::Radio,
            "playlists" => Route::Playlists,
            "favorites" => Route::Favorites,
            "activity" => Route::Activity,
            "downloader" => Route::Downloader,
            _ => return None,
        })
    }

    /// The destination this route lights up, detail pages included.
    ///
    /// `artists_remote` is [`api::ArtistPresentation::Remote`], the same fact
    /// the router opens the page on, so the lit destination is the page that
    /// actually opened.
    pub fn destination_of(&self, artists_remote: bool) -> Option<&'static str> {
        Some(match self {
            Route::Home => "home",
            Route::Library
            | Route::LibraryView(_)
            | Route::Albums
            | Route::Album(_)
            | Route::Artists => "library",
            Route::Favorites => "favorites",
            Route::Artist(_) => match artists_remote {
                true => "discover",
                false => "library",
            },
            Route::Discover
            | Route::DiscoverPlaylist { .. }
            | Route::DiscoverArtist(_)
            | Route::CatalogPage { .. }
            | Route::Search
            | Route::Genre(_) => "discover",
            Route::Radio => "radio",
            Route::Playlists | Route::PlaylistFolder(_) | Route::Playlist(_) => "playlists",
            Route::Activity => "activity",
            Route::Downloader => "downloader",
            Route::Settings { .. } => return None,
        })
    }

    /// The parameterless routes, under the names `KOPUZ_ROUTE` accepts.
    pub fn from_name(name: &str) -> Option<Route> {
        Some(match name {
            "home" => Route::Home,
            "search" => Route::Search,
            "discover" => Route::Discover,
            "library" => Route::Library,
            "albums" => Route::Albums,
            "artists" => Route::Artists,
            "playlists" => Route::Playlists,
            "favorites" => Route::Favorites,
            "activity" => Route::Activity,
            "radio" => Route::Radio,
            "downloader" => Route::Downloader,
            "settings" => Route::Settings { section: None },
            // `settings/appearance` opens straight onto one category.
            other => match other.strip_prefix("settings/") {
                Some(section) => Route::Settings {
                    section: Some(SharedString::from(section.to_string())),
                },
                None => return None,
            },
        })
    }
}

/// Where the app is, and how it got there.
pub struct Navigator {
    current: Route,
    back: Vec<Route>,
    forward: Vec<Route>,
    intent: Option<&'static str>,
}

impl Navigator {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        // KOPUZ_ROUTE opens straight onto a page, so a smoke run can render each one.
        let asked = std::env::var("KOPUZ_ROUTE")
            .ok()
            .and_then(|name| Route::from_name(&name))
            .or_else(|| Route::destination(&crate::layout(cx).nav.start))
            .unwrap_or(Route::Home);
        Self {
            current: asked.clone().redirect(),
            back: Vec::new(),
            forward: Vec::new(),
            intent: asked.intent(),
        }
    }

    pub fn current(&self) -> &Route {
        &self.current
    }

    /// What the route that redirected here was after, for the page that has to stand in for it.
    pub fn intent(&self) -> Option<&'static str> {
        self.intent
    }

    /// Take the intent, so asking for the same one again is a fresh request, not a no-op.
    pub fn take_intent(&mut self) -> Option<&'static str> {
        self.intent.take()
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn go(&mut self, route: Route, cx: &mut Context<Self>) {
        let intent = route.intent();
        let route = route.redirect();
        if route == self.current {
            // A redirected route still says what it wanted, even when it lands where it already is.
            if self.intent != intent {
                self.intent = intent;
                crate::notify(cx);
            }
            return;
        }
        self.intent = intent;
        self.back.push(std::mem::replace(&mut self.current, route));
        self.forward.clear();
        crate::notify(cx);
    }

    /// Swap where the app is without pushing history: a filter keystroke is not a place to go back to.
    pub fn replace(&mut self, route: Route, cx: &mut Context<Self>) {
        let route = route.redirect();
        if route == self.current {
            return;
        }
        self.current = route;
        crate::notify(cx);
    }

    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(route) = self.back.pop() {
            self.forward
                .push(std::mem::replace(&mut self.current, route));
            self.intent = None;
            crate::notify(cx);
        }
    }

    pub fn go_forward(&mut self, cx: &mut Context<Self>) {
        if let Some(route) = self.forward.pop() {
            self.back.push(std::mem::replace(&mut self.current, route));
            self.intent = None;
            crate::notify(cx);
        }
    }

    /// Navigate from anywhere holding an `App`.
    pub fn navigate(route: Route, cx: &mut App) {
        Self::global(cx).update(cx, |this, cx| this.go(route, cx));
    }

    /// What a back chevron does: pop history, or go to `fallback` if there is none to pop.
    ///
    /// Navigating to the parent instead would push onto the back stack, so pressing back would
    /// walk further forward — and from a page reached sideways it lands somewhere never visited.
    pub fn back_or(fallback: Route, cx: &mut App) {
        Self::global(cx).update(cx, |this, cx| match this.can_go_back() {
            true => this.go_back(cx),
            false => this.go(fallback, cx),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::DESTINATIONS;

    #[test]
    fn a_route_that_is_no_longer_a_place_lands_somewhere() {
        // Most-played is an order over the library, not a page of its own.
        for route in [Route::Activity, Route::Albums, Route::Artists] {
            assert_eq!(route.redirect(), Route::Library);
        }
        assert_eq!(Route::Playlists.redirect(), Route::Library);
        assert_eq!(Route::Search.redirect(), Route::Discover);
        assert_eq!(Route::Downloader.redirect(), Route::Home);
    }

    #[test]
    fn a_route_that_is_still_a_place_is_left_alone() {
        for route in [
            Route::Home,
            Route::Library,
            Route::Favorites,
            Route::Discover,
            Route::Radio,
        ] {
            assert_eq!(route.clone().redirect(), route);
        }
    }

    #[test]
    fn a_redirect_says_what_it_was_after() {
        assert_eq!(Route::Activity.intent(), Some("activity"));
        assert_eq!(Route::Favorites.intent(), None);
        assert_eq!(Route::Library.intent(), None);
    }

    #[test]
    fn the_smoke_route_names_still_resolve() {
        for name in ["home", "library", "favorites", "activity", "downloader"] {
            let route = Route::from_name(name).expect("a name the smoke script uses");
            assert!(matches!(
                route.redirect(),
                Route::Home | Route::Library | Route::Favorites | Route::Discover | Route::Radio
            ));
        }
    }

    #[test]
    fn every_destination_opens_a_route() {
        for name in DESTINATIONS {
            assert!(Route::destination(name).is_some(), "{name} opens nothing");
        }
    }

    #[test]
    fn a_destination_lights_up_from_its_own_route() {
        for name in DESTINATIONS {
            let route = Route::destination(name).expect("a destination route");
            assert_eq!(route.destination_of(false), Some(*name));
        }
    }

    /// An artist lights the destination the page actually opened under, which is what the source presents.
    #[test]
    fn an_artist_lights_up_by_presentation() {
        let artist = Route::Artist(String::from("lib:1"));

        assert_eq!(artist.destination_of(true), Some("discover"));
        assert_eq!(artist.destination_of(false), Some("library"));
    }

    #[test]
    fn a_detail_page_lights_the_destination_it_came_from() {
        assert_eq!(
            Route::Album("x".into()).destination_of(false),
            Some("library")
        );
        assert_eq!(Route::Search.destination_of(false), Some("discover"));
        let page = Route::CatalogPage {
            id: "FEmusic_charts".into(),
        };
        assert_eq!(page.destination_of(false), Some("discover"));
        assert_eq!(page.root(), Route::Discover);
        assert_eq!(
            Route::Playlist("x".into()).destination_of(false),
            Some("playlists")
        );
    }

    #[test]
    fn every_library_filter_set_is_the_same_cached_view() {
        assert_eq!(
            Route::Library.key(),
            Route::LibraryView("f=albums".into()).key()
        );
        assert_ne!(Route::Library.key(), Route::Albums.key());
        assert_ne!(Route::Library.key(), Route::Favorites.key());
    }

    #[test]
    fn a_filtered_library_still_lights_the_library() {
        let route = Route::LibraryView("f=tracks&fav=1".into());
        assert_eq!(route.destination_of(false), Some("library"));
        assert_eq!(route.root(), Route::Library);
        assert_eq!(
            route.library_view().map(|view| view.to_string()),
            Some("f=tracks&fav=1".to_string())
        );
    }

    #[test]
    fn settings_is_not_a_destination() {
        assert_eq!(
            Route::Settings { section: None }.destination_of(false),
            None
        );
    }

    #[test]
    fn favorites_is_a_place_of_its_own() {
        assert_eq!(Route::Favorites.clone().redirect(), Route::Favorites);
        assert_eq!(Route::Favorites.destination_of(false), Some("favorites"));
    }
}
