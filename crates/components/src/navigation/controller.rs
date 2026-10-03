use dioxus::prelude::*;
use kopuz_route::Route;

#[derive(Clone, PartialEq)]
pub struct NavSnapshot {
    pub route: Route,
    pub album_id: String,
    pub artist: Option<String>,
    pub playlist_id: Option<String>,
    pub discover_playlist_id: Option<String>,
    pub discover_playlist_title: Option<String>,
}

#[derive(Clone, Copy)]
pub struct NavigationController {
    pub current_route: Signal<Route>,
    /// The open artist; `None` on the artist route is the grid of them all.
    pub selected_artist: Signal<Option<String>>,
    pub selected_album_id: Signal<String>,
    pub selected_playlist_id: Signal<Option<String>>,
    pub discover_playlist_id: Signal<Option<String>>,
    pub discover_playlist_title: Signal<Option<String>>,
    pub history: Signal<Vec<NavSnapshot>>,
    pub restoring: Signal<bool>,
}

impl NavigationController {
    pub fn open_artist(self, artist: String) {
        let mut selected = self.selected_artist;
        let mut route = self.current_route;
        selected.set(Some(artist));
        route.set(Route::Artist);
    }

    pub fn navigate_to_album(self, id: String) {
        if id.is_empty() {
            return;
        }
        let mut album = self.selected_album_id;
        let mut route = self.current_route;
        album.set(id);
        route.set(Route::Album);
    }

    pub fn close_playlist(self) {
        let mut restoring = self.restoring;
        let mut playlist = self.selected_playlist_id;
        restoring.set(true);
        playlist.set(None);
    }

    /// The active source changed: what is open and every step back names the old source's rows, so all of it goes.
    pub fn leave_source(self) {
        let mut restoring = self.restoring;
        let mut history = self.history;
        let mut route = self.current_route;
        let mut album = self.selected_album_id;
        let mut artist = self.selected_artist;
        let mut playlist = self.selected_playlist_id;
        let mut discover_playlist = self.discover_playlist_id;
        let mut discover_title = self.discover_playlist_title;
        restoring.set(true);
        history.write().clear();
        album.set(String::new());
        artist.set(None);
        playlist.set(None);
        discover_playlist.set(None);
        discover_title.set(None);
        if *route.peek() == Route::DiscoverPlaylist {
            route.set(Route::Home);
        }
    }

    pub fn go_back(self) {
        let mut history = self.history;
        let Some(prev) = history.write().pop() else {
            return;
        };
        let mut restoring = self.restoring;
        let mut route = self.current_route;
        let mut album = self.selected_album_id;
        let mut artist = self.selected_artist;
        let mut playlist = self.selected_playlist_id;
        let mut discover_playlist = self.discover_playlist_id;
        let mut discover_title = self.discover_playlist_title;
        restoring.set(true);
        album.set(prev.album_id);
        artist.set(prev.artist);
        playlist.set(prev.playlist_id);
        discover_playlist.set(prev.discover_playlist_id);
        discover_title.set(prev.discover_playlist_title);
        route.set(prev.route);
    }
}
