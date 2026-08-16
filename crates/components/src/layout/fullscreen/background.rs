use config::AppConfig;
use dioxus::prelude::*;

pub(crate) fn use_fullscreen_background(
    palette: Signal<Option<Vec<utils::color::Color>>>,
    current_song_cover_url: Signal<String>,
) -> (Memo<String>, Memo<Option<String>>) {
    let config = use_context::<Signal<AppConfig>>();

    let background_style = use_memo(move || {
        let conf = config.read();
        if conf.theme == "album-art"
            && !conf.cover_art_background
            && conf.custom_background_path.is_empty()
        {
            utils::color::get_background_style(palette.read().as_deref())
        } else {
            "background-color: var(--color-black); background-image: none;".to_string()
        }
    });

    let cover_background = use_memo(move || {
        let conf = config.read();
        if !conf.custom_background_path.is_empty() {
            let path = std::path::PathBuf::from(&conf.custom_background_path);
            return utils::format_artwork_url(Some(&path)).map(|url| url.as_ref().to_string());
        }
        if conf.cover_art_background {
            let url = current_song_cover_url.read().clone();
            return (!url.is_empty()).then_some(url);
        }
        None
    });

    (background_style, cover_background)
}
