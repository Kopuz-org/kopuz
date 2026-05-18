use dioxus::prelude::*;
use kopuz_route::Route;

#[derive(Clone, Copy)]
pub struct NavigationController {
    pub current_route: Signal<Route>,
    pub selected_artist_name: Signal<String>,
    pub selected_album_id: Signal<String>,
}

impl NavigationController {
    pub fn navigate_to_artist(self, name: String) {
        if name.is_empty() {
            return;
        }
        let mut artist = self.selected_artist_name;
        let mut route = self.current_route;
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
