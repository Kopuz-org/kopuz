use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub struct MenuAction {
    pub label: String,
    pub icon: String,
    pub destructive: bool,
}

impl MenuAction {
    pub fn new(label: impl Into<String>, icon: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: icon.into(),
            destructive: false,
        }
    }

    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DotsMenuProps {
    pub actions: Vec<MenuAction>,
    pub on_action: EventHandler<usize>,
    pub is_open: bool,
    pub on_open: EventHandler<()>,
    pub on_close: EventHandler<()>,
    #[props(default)]
    pub button_class: String,
    #[props(default = "right".to_string())]
    pub anchor: String,
}

#[component]
pub fn DotsMenu(props: DotsMenuProps) -> Element {
    let dropdown_align = if props.anchor == "left" {
        "left-0"
    } else {
        "right-0"
    };

    let base_button_class = format!(
        "w-8 h-8 flex items-center justify-center rounded-full hover:bg-white/10 text-slate-400 hover:text-white transition-colors {}",
        props.button_class
    );

    rsx! {
        div {
            class: if props.is_open { "relative dots-menu-root" } else { "relative" },

            button {
                class: "{base_button_class}",
                onclick: move |evt| {
                    evt.stop_propagation();
                    if props.is_open {
                        props.on_close.call(());
                    } else {
                        props.on_open.call(());
                    }
                },
                i { class: "fa-solid fa-ellipsis-vertical" }
            }

            if props.is_open {
                div {
                    class: "fixed inset-0 dots-menu-backdrop",
                    onclick: move |evt| {
                        evt.stop_propagation();
                        props.on_close.call(());
                    }
                }

                div {
                    class: "absolute {dropdown_align} top-full mt-1 w-auto bg-neutral-900 border border-white/10 rounded-lg dots-menu-panel py-1 shadow-xl",
                    onclick: move |evt| evt.stop_propagation(),

                    for (idx, action) in props.actions.iter().enumerate() {
                        {
                            let label = action.label.clone();
                            let icon  = action.icon.clone();
                            let text_color = if action.destructive {
                                "text-red-400 hover:text-red-300"
                            } else {
                                "text-white"
                            };

                            rsx! {
                                button {
                                    key: "{idx}",
                                    class: "w-full text-left px-4 py-2 text-sm {text_color} hover:bg-white/10 flex items-center gap-2 transition-colors whitespace-nowrap",
                                    onclick: move |_| {
                                        props.on_action.call(idx);
                                    },
                                    i { class: "{icon}" }
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
