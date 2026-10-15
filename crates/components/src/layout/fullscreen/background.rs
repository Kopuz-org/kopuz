use config::Settings;
use dioxus::prelude::*;

pub(crate) fn use_fullscreen_background(
    palette: Signal<Option<Vec<utils::color::Color>>>,
) -> (Memo<String>, Memo<Option<String>>) {
    let settings = use_context::<Signal<Settings>>();
    let ctrl = use_context::<hooks::use_player_controller::PlayerController>();

    let background_style = use_memo(move || {
        let conf = settings.read();
        if conf.ui.theme == "album-art"
            && !conf.background.cover_art_background
            && conf.background.custom_background_path.is_empty()
        {
            utils::color::get_background_style(palette.read().as_deref())
        } else {
            "background-color: var(--color-black); background-image: none;".to_string()
        }
    });

    let cover_background = use_memo(move || {
        let conf = settings.read();
        if !conf.background.custom_background_path.is_empty() {
            let path = std::path::PathBuf::from(&conf.background.custom_background_path);
            return utils::format_artwork_url(Some(&path)).map(|url| url.as_ref().to_string());
        }
        if conf.background.cover_art_background {
            return ctrl.current_cover_url(hooks::artwork::Size::Full);
        }
        None
    });

    (background_style, cover_background)
}
