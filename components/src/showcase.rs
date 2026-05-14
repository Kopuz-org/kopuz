use config::AppConfig;
use dioxus::prelude::*;
use reader::{Library, Track};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Props, Clone, PartialEq)]
pub struct ShowcaseProps {
    pub name: String,
    pub description: String,
    pub cover_url: Option<utils::CoverUrl>,
    pub tracks: Vec<Track>,
    pub library: Signal<Library>,
    pub on_play_all: EventHandler<()>,
    pub on_play: EventHandler<usize>,
    pub on_queue: Option<EventHandler<usize>>,
    pub on_add_to_playlist: Option<EventHandler<usize>>,
    pub on_delete_track: Option<EventHandler<usize>>,
    pub on_remove_from_playlist: Option<EventHandler<usize>>,
    pub on_download_track: Option<EventHandler<usize>>,
    pub active_track: Option<std::path::PathBuf>,
    pub on_click_menu: Option<EventHandler<usize>>,
    pub on_close_menu: Option<EventHandler<()>>,
    pub actions: Option<Element>,
    pub on_download_all: Option<EventHandler<()>>,
    pub on_delete_all: Option<EventHandler<()>>,
    #[props(default = false)]
    pub is_downloading_all: bool,
    #[props(default = false)]
    pub is_selection_mode: bool,
    #[props(default = HashSet::new())]
    pub selected_tracks: HashSet<PathBuf>,
    pub on_select: Option<EventHandler<(usize, bool)>>,
    pub on_select_all: Option<EventHandler<bool>>,
    #[props(default = false)]
    pub all_selected: bool,
    pub on_long_press: Option<EventHandler<usize>>,
    pub on_cover_click: Option<EventHandler<()>>,
    #[props(default = false)]
    pub is_reorderable: bool,
    #[props(default)]
    pub on_move_up: EventHandler<usize>,
    #[props(default)]
    pub on_move_down: EventHandler<usize>,
}

#[component]
pub fn Showcase(props: ShowcaseProps) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    match config.read().ui_style {
        config::UiStyle::Modern => rsx! {
            crate::modern::showcase::ShowcaseModern { ..props }
        },
        config::UiStyle::Normal => rsx! {
            crate::normal::showcase::ShowcaseNormal { ..props }
        },
    }
}
