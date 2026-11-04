//! Route definitions for the Kopuz Dioxus application: enum of all navigable
//! screens (Home, Discover, Album, Artist, Playlist, Settings, etc.).

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Route {
    Home,
    Discover,
    DiscoverPlaylist,
    /// A catalog page other than the one Discover opens on.
    Browse,
    Search,
    Library,
    Album,
    Artist,
    Playlists,
    Favorites,
    Activity,
    Radio,

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    Downloader,
    Settings,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    ThemeEditor,
}

impl Route {
    pub fn sidebar_entry(self) -> Route {
        match self {
            Route::Browse => Route::Discover,
            route => route,
        }
    }
}
