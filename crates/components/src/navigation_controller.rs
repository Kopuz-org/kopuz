use dioxus::prelude::*;
use kopuz_route::Route;

#[derive(Clone, PartialEq)]
pub struct NavSnapshot {
    pub route: Route,
    pub album_id: String,
    pub artist_name: String,
    pub artist_channel_id: Option<String>,
    pub playlist_id: Option<String>,
    pub discover_playlist_id: Option<String>,
    pub discover_playlist_title: Option<String>,
}

#[derive(Clone, Copy)]
pub struct NavigationController {
    pub current_route: Signal<Route>,
    pub selected_artist_name: Signal<String>,
    /// YT Music channel id corresponding to the selected artist when
    /// the click site knew it. None means the artist page resolves it
    /// via search from the name. Local backends (Jellyfin / Subsonic /
    /// library scan) leave this None unconditionally.
    pub selected_artist_channel_id: Signal<Option<String>>,
    pub selected_album_id: Signal<String>,
    pub selected_playlist_id: Signal<Option<String>>,
    pub discover_playlist_id: Signal<Option<String>>,
    pub discover_playlist_title: Signal<Option<String>>,
    pub history: Signal<Vec<NavSnapshot>>,
    pub restoring: Signal<bool>,
}

impl NavigationController {
    /// Navigate by name only. Used by every artist click outside
    /// Discover (track row, sidebar tag, library entry, search hit).
    /// Clears any leftover YT channel id so the YT artist page knows
    /// to resolve from the name.
    pub fn navigate_to_artist(self, name: String) {
        if name.is_empty() {
            return;
        }
        let mut artist = self.selected_artist_name;
        let mut channel_id = self.selected_artist_channel_id;
        let mut route = self.current_route;
        channel_id.set(None);
        artist.set(name);
        route.set(Route::Artist);
    }

    /// Navigate when the YT channel id is already known (Discover
    /// tile, mix entry, anything that classify_flex_columns picked a
    /// UC… browseEndpoint out of). Skips the resolve roundtrip on the
    /// YT artist page.
    pub fn navigate_to_artist_yt(self, channel_id: String, name: String) {
        if channel_id.is_empty() {
            self.navigate_to_artist(name);
            return;
        }
        let mut artist = self.selected_artist_name;
        let mut cid = self.selected_artist_channel_id;
        let mut route = self.current_route;
        cid.set(Some(channel_id));
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
        let mut channel_id = self.selected_artist_channel_id;
        let mut playlist = self.selected_playlist_id;
        let mut discover_playlist = self.discover_playlist_id;
        let mut discover_title = self.discover_playlist_title;
        restoring.set(true);
        album.set(prev.album_id);
        artist.set(prev.artist_name);
        channel_id.set(prev.artist_channel_id);
        playlist.set(prev.playlist_id);
        discover_playlist.set(prev.discover_playlist_id);
        discover_title.set(prev.discover_playlist_title);
        route.set(prev.route);
    }
}
