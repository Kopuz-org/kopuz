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
    #[props(default = "bottom".to_string())]
    pub placement: String,
    #[props(default)]
    pub position: Option<(f64, f64)>,
    #[props(default = "fa-solid fa-ellipsis-vertical".to_string())]
    pub icon: String,
}

/// The panel measures itself pinned to the viewport origin, because `left`/`right`
/// left to `auto` shrink-to-fit against whatever room sits beside the trigger's
/// static position — in RTL that is a few pixels, which collapses the panel to
/// icon width. `anchor` is likewise phrased in LTR terms and mirrored for RTL so
/// the panel opens towards the middle of the window instead of off-screen.
#[component]
pub fn DotsMenu(props: DotsMenuProps) -> Element {
    let mut trigger_element = use_signal(|| None::<MountedEvent>);
    let mut panel_geometry = use_signal(|| None::<(f64, f64, f64, f64)>);
    let is_rtl = i18n::is_rtl();

    let base_button_class = format!(
        "w-8 h-8 flex items-center justify-center rounded-full hover:bg-white/10 text-slate-400 hover:text-white transition-colors {}",
        props.button_class
    );
    let panel_style = match *panel_geometry.read() {
        Some((left, top, width, height)) => format!(
            "position: fixed; left: clamp(8px, {left}px, calc(100vw - {width}px - 8px)); top: clamp(8px, {top}px, calc(100vh - {height}px - 8px)); visibility: visible;"
        ),
        None => "position: fixed; left: 0; top: 0; visibility: hidden;".to_string(),
    };

    rsx! {
        div {
            class: if props.is_open { "relative dots-menu-root" } else { "relative" },

            button {
                class: "{base_button_class}",
                onmounted: move |evt| trigger_element.set(Some(evt)),
                onclick: move |evt| {
                    evt.stop_propagation();
                    if props.is_open {
                        panel_geometry.set(None);
                        props.on_close.call(());
                    } else {
                        panel_geometry.set(None);
                        props.on_open.call(());
                    }
                },
                i { class: "{props.icon}" }
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
                    class: "w-auto flex flex-col bg-neutral-900 border border-white/10 rounded-lg dots-menu-panel py-1 shadow-xl",
                    style: "{panel_style}",
                    onmounted: {
                        let anchor = props.anchor.clone();
                        let placement = props.placement.clone();
                        let position = props.position;
                        move |panel_evt: MountedEvent| {
                            let trigger_evt = trigger_element.peek().clone();
                            let anchor = anchor.clone();
                            let placement = placement.clone();
                            async move {
                                let Ok(panel_rect) = panel_evt.get_client_rect().await else {
                                    return;
                                };
                                let (left, top) = if let Some((left, top)) = position {
                                    (left, top)
                                } else {
                                    let Some(trigger_evt) = trigger_evt else {
                                        return;
                                    };
                                    let Ok(trigger_rect) = trigger_evt.get_client_rect().await else {
                                        return;
                                    };
                                    let left = if (anchor == "left") != is_rtl {
                                        trigger_rect.min_x()
                                    } else {
                                        trigger_rect.max_x() - panel_rect.width()
                                    };
                                    let top = if placement == "top" {
                                        trigger_rect.min_y() - panel_rect.height() - 4.0
                                    } else {
                                        trigger_rect.max_y() + 4.0
                                    };
                                    (left, top)
                                };
                                panel_geometry.set(Some((
                                    left,
                                    top,
                                    panel_rect.width(),
                                    panel_rect.height(),
                                )));
                            }
                        }
                    },
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
                                    class: "px-4 py-2 text-sm {text_color} hover:bg-white/10 flex items-center gap-2 transition-colors whitespace-nowrap",
                                    onclick: move |_| {
                                        panel_geometry.set(None);
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
