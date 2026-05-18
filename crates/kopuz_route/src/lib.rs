#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Route {
    Home,
    Search,
    Library,
    Album,
    Artist,
    Playlists,
    Favorites,
    Activity,
    Radio,
    Ytdlp,
    Settings,
    ThemeEditor,
}
