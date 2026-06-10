use dioxus::prelude::*;
use kopuz_route::Route;

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
}
