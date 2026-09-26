use dioxus::prelude::*;
use kopuz_route::Route;

#[derive(Clone, PartialEq)]
pub struct NavSnapshot {
    pub route: Route,
    pub album_id: String,
    pub artist_name: String,
    pub artist_id: Option<String>,
    pub playlist_id: Option<String>,
    pub discover_playlist_id: Option<String>,
    pub discover_playlist_title: Option<String>,
}

#[derive(Clone, Copy)]
pub struct NavigationController {
    pub current_route: Signal<Route>,
    pub selected_artist_name: Signal<String>,
    /// The source's own artist id, where it has one and a listing
    /// carried it. None means the artist page resolves it from the name;
    /// a source whose artists are a view of the library leaves it unset.
    pub selected_artist_id: Signal<Option<String>>,
    pub selected_album_id: Signal<String>,
    pub selected_playlist_id: Signal<Option<String>>,
    pub discover_playlist_id: Signal<Option<String>>,
    pub discover_playlist_title: Signal<Option<String>>,
    pub history: Signal<Vec<NavSnapshot>>,
    pub restoring: Signal<bool>,
}

impl NavigationController {
    /// Open an artist by name alone, for a click holding nothing better. The
    /// page then asks the source to resolve it, which costs a round trip and
    /// can land on someone else; prefer [`Self::open_artist`].
    pub fn navigate_to_artist(self, name: String) {
        self.open_artist(name, None);
    }

    /// Open an artist, by the id its source issued where the click held one.
    /// Name and id are always set together: a leftover id outlives the name it
    /// belonged to and opens the wrong artist.
    pub fn open_artist(self, name: String, id: Option<String>) {
        if name.is_empty() {
            return;
        }
        let mut artist = self.selected_artist_name;
        let mut artist_id = self.selected_artist_id;
        let mut route = self.current_route;
        artist_id.set(id.filter(|id| !id.trim().is_empty()));
        artist.set(name);
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

    pub fn can_go_back(self) -> bool {
        !self.history.read().is_empty()
    }

    pub fn go_back(self) {
        let mut history = self.history;
        let Some(prev) = history.write().pop() else {
            return;
        };
        let mut restoring = self.restoring;
        let mut route = self.current_route;
        let mut album = self.selected_album_id;
        let mut artist = self.selected_artist_name;
        let mut artist_id = self.selected_artist_id;
        let mut playlist = self.selected_playlist_id;
        let mut discover_playlist = self.discover_playlist_id;
        let mut discover_title = self.discover_playlist_title;
        restoring.set(true);
        album.set(prev.album_id);
        artist.set(prev.artist_name);
        artist_id.set(prev.artist_id);
        playlist.set(prev.playlist_id);
        discover_playlist.set(prev.discover_playlist_id);
        discover_title.set(prev.discover_playlist_title);
        route.set(prev.route);
    }
}
