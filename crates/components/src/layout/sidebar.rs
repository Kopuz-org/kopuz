use dioxus::prelude::*;
use kopuz_route::Route;

#[derive(Props, Clone, PartialEq)]
pub struct SidebarProps {
    pub current_route: Signal<Route>,
    pub on_navigate: EventHandler<Route>,
}

#[derive(Clone, Copy)]
pub struct SidebarCollapsed(pub Signal<bool>);

#[component]
pub fn Sidebar(props: SidebarProps) -> Element {
    let config = use_context::<Signal<config::AppConfig>>();
    match config.read().ui_style {
        config::UiStyle::Vaxry => rsx! {
            crate::vaxry::sidebar::SidebarVaxry {
                current_route: props.current_route,
                on_navigate: props.on_navigate,
            }
        },
        config::UiStyle::Normal => rsx! {
            crate::normal::sidebar::SidebarNormal {
                current_route: props.current_route,
                on_navigate: props.on_navigate,
            }
        },
    }
}
