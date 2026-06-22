//! Route definitions for the Kopuz Dioxus application: enum of all navigable
//! screens (Home, Discover, Album, Artist, Playlist, Settings, etc.).

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Route {
    Home,
    Discover,
    DiscoverPlaylist,
    Search,
    Library,
    Album,
    Artist,
    Playlists,
    Favorites,
    Activity,
    Radio,
    // yt-dlp downloads + the custom theme editor are desktop/web only — excluded on Android.
    #[cfg(not(target_os = "android"))]
    Ytdlp,
    Settings,
    #[cfg(not(target_os = "android"))]
    ThemeEditor,
}
