//! Route definitions for the Kopuz Dioxus application: enum of all navigable
//! screens (Home, Discover, Album, Artist, Playlist, Settings, etc.).

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Route {
    Home,
    Discover,
    DiscoverPlaylist,
    /// One of the source's catalog pages, other than the one Discover opens on.
    Browse,
    Search,
    Library,
    Album,
    Artist,
    Playlists,
    Favorites,
    Activity,
    Radio,
    // URL downloads + the custom theme editor are desktop/web only — excluded on Android.
    #[cfg(not(target_os = "android"))]
    Downloader,
    Settings,
    #[cfg(not(target_os = "android"))]
    ThemeEditor,
}

impl Route {
    /// The sidebar entry lit while this route is on screen.
    pub fn sidebar_entry(self) -> Route {
        match self {
            Route::Browse => Route::Discover,
            route => route,
        }
    }
}
