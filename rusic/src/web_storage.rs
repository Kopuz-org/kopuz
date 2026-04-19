#[cfg(target_arch = "wasm32")]
use reader::FavoritesStore;
#[cfg(target_arch = "wasm32")]
use rusic_route::Route;

#[cfg(target_arch = "wasm32")]
const WEB_CONFIG_STORAGE_KEY: &str = "rusic.config.v1";
#[cfg(target_arch = "wasm32")]
const WEB_UI_STATE_STORAGE_KEY: &str = "rusic.ui-state.v1";
#[cfg(target_arch = "wasm32")]
const WEB_LIBRARY_STORAGE_KEY: &str = "rusic.library.v1";
#[cfg(target_arch = "wasm32")]
const WEB_PLAYLISTS_STORAGE_KEY: &str = "rusic.playlists.v1";
#[cfg(target_arch = "wasm32")]
const WEB_FAVORITES_STORAGE_KEY: &str = "rusic.favorites.v1";

#[cfg(target_arch = "wasm32")]
fn route_to_storage(route: Route) -> &'static str {
    match route {
        Route::Home => "home",
        Route::Search => "search",
        Route::Library => "library",
        Route::Album => "album",
        Route::Artist => "artist",
        Route::Playlists => "playlists",
        Route::Favorites => "favorites",
        Route::Logs => "logs",
        Route::Settings => "settings",
        Route::ThemeEditor => "theme_editor",
    }
}

#[cfg(target_arch = "wasm32")]
fn route_from_storage(route: &str) -> Option<Route> {
    match route {
        "home" => Some(Route::Home),
        "search" => Some(Route::Search),
        "library" => Some(Route::Library),
        "album" => Some(Route::Album),
        "artist" => Some(Route::Artist),
        "playlists" => Some(Route::Playlists),
        "favorites" => Some(Route::Favorites),
        "logs" => Some(Route::Logs),
        "settings" => Some(Route::Settings),
        "theme_editor" => Some(Route::ThemeEditor),
        _ => None,
    }
}

#[cfg(target_arch = "wasm32")]
pub fn load_web_config() -> Option<config::AppConfig> {
    let storage = web_sys::window()
        .and_then(|w| w.local_storage().ok())
        .flatten()?;
    let raw = storage.get_item(WEB_CONFIG_STORAGE_KEY).ok().flatten()?;
    serde_json::from_str::<config::AppConfig>(&raw).ok()
}

#[cfg(target_arch = "wasm32")]
pub fn save_web_config(cfg: &config::AppConfig) {
    if let (Some(storage), Ok(raw)) = (
        web_sys::window()
            .and_then(|w| w.local_storage().ok())
            .flatten(),
        serde_json::to_string(cfg),
    ) {
        let _ = storage.set_item(WEB_CONFIG_STORAGE_KEY, &raw);
    }
}

#[cfg(target_arch = "wasm32")]
pub fn load_web_ui_state() -> Option<(Route, String, Option<String>, String, String)> {
    let storage = web_sys::window()
        .and_then(|w| w.local_storage().ok())
        .flatten()?;
    let raw = storage.get_item(WEB_UI_STATE_STORAGE_KEY).ok().flatten()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;

    let route = value
        .get("route")
        .and_then(|v| v.as_str())
        .and_then(route_from_storage)
        .unwrap_or(Route::Home);
    let selected_album_id = value
        .get("selected_album_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let selected_playlist_id = value
        .get("selected_playlist_id")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    let selected_artist_name = value
        .get("selected_artist_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let search_query = value
        .get("search_query")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    Some((
        route,
        selected_album_id,
        selected_playlist_id,
        selected_artist_name,
        search_query,
    ))
}

#[cfg(target_arch = "wasm32")]
pub fn save_web_ui_state(
    route: Route,
    selected_album_id: &str,
    selected_playlist_id: Option<&str>,
    selected_artist_name: &str,
    search_query: &str,
) {
    if let Some(storage) = web_sys::window()
        .and_then(|w| w.local_storage().ok())
        .flatten()
    {
        let value = serde_json::json!({
            "route": route_to_storage(route),
            "selected_album_id": selected_album_id,
            "selected_playlist_id": selected_playlist_id,
            "selected_artist_name": selected_artist_name,
            "search_query": search_query,
        });
        if let Ok(raw) = serde_json::to_string(&value) {
            let _ = storage.set_item(WEB_UI_STATE_STORAGE_KEY, &raw);
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn load_web_library() -> Option<reader::Library> {
    let storage = web_sys::window()
        .and_then(|w| w.local_storage().ok())
        .flatten()?;
    let raw = storage.get_item(WEB_LIBRARY_STORAGE_KEY).ok().flatten()?;
    serde_json::from_str::<reader::Library>(&raw).ok()
}

#[cfg(target_arch = "wasm32")]
pub fn save_web_library(library: &reader::Library) {
    if let (Some(storage), Ok(raw)) = (
        web_sys::window()
            .and_then(|w| w.local_storage().ok())
            .flatten(),
        serde_json::to_string(library),
    ) {
        let _ = storage.set_item(WEB_LIBRARY_STORAGE_KEY, &raw);
    }
}

#[cfg(target_arch = "wasm32")]
pub fn load_web_playlists() -> Option<reader::PlaylistStore> {
    let storage = web_sys::window()
        .and_then(|w| w.local_storage().ok())
        .flatten()?;
    let raw = storage.get_item(WEB_PLAYLISTS_STORAGE_KEY).ok().flatten()?;
    serde_json::from_str::<reader::PlaylistStore>(&raw).ok()
}

#[cfg(target_arch = "wasm32")]
pub fn save_web_playlists(store: &reader::PlaylistStore) {
    if let (Some(storage), Ok(raw)) = (
        web_sys::window()
            .and_then(|w| w.local_storage().ok())
            .flatten(),
        serde_json::to_string(store),
    ) {
        let _ = storage.set_item(WEB_PLAYLISTS_STORAGE_KEY, &raw);
    }
}

#[cfg(target_arch = "wasm32")]
pub fn load_web_favorites() -> Option<FavoritesStore> {
    let storage = web_sys::window()
        .and_then(|w| w.local_storage().ok())
        .flatten()?;
    let raw = storage.get_item(WEB_FAVORITES_STORAGE_KEY).ok().flatten()?;
    serde_json::from_str::<FavoritesStore>(&raw).ok()
}

#[cfg(target_arch = "wasm32")]
pub fn save_web_favorites(store: &FavoritesStore) {
    if let (Some(storage), Ok(raw)) = (
        web_sys::window()
            .and_then(|w| w.local_storage().ok())
            .flatten(),
        serde_json::to_string(store),
    ) {
        let _ = storage.set_item(WEB_FAVORITES_STORAGE_KEY, &raw);
    }
}