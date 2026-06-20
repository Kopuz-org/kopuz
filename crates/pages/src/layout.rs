use config::UiStyle;

pub fn page_container_class(ui_style: &UiStyle) -> &'static str {
    if cfg!(target_os = "android") {
        "px-3 pt-3 absolute inset-0 flex flex-col overflow-x-hidden"
    } else {
        match ui_style {
            UiStyle::Modern => "px-6 pt-6 absolute inset-0 flex flex-col",
            UiStyle::Normal => "px-8 pt-8 absolute inset-0 flex flex-col",
        }
    }
}
