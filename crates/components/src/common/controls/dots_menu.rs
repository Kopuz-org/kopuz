use dioxus::prelude::*;
use std::sync::Mutex;

/// Where the next menu should open, when a right-click asked for it.
///
/// A right-click and the open it triggers are one gesture, and only one menu is
/// ever up, so the point rides here instead of through a prop on every surface
/// that has a context handler.
static CONTEXT_POINT: Mutex<Option<(f64, f64)>> = Mutex::new(None);

/// Anchor the menu this contextmenu event is about to open at the pointer.
/// Call it from `oncontextmenu`, before opening the menu.
pub fn open_at_pointer(evt: &Event<MouseData>) {
    let point = evt.client_coordinates();
    if let Ok(mut slot) = CONTEXT_POINT.lock() {
        *slot = Some((point.x, point.y));
    }
}

fn take_context_point() -> Option<(f64, f64)> {
    CONTEXT_POINT.lock().ok().and_then(|mut slot| slot.take())
}

fn clear_context_point() {
    if let Ok(mut slot) = CONTEXT_POINT.lock() {
        *slot = None;
    }
}

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
    #[props(default = "fa-solid fa-ellipsis-vertical".to_string())]
    pub icon: String,
    /// Accessible name for the icon-only trigger.
    pub aria_label: String,
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
            // `cursor-default` because rows that own a menu are often drag
            // handles carrying `cursor-grab`, and `cursor` inherits.
            class: if props.is_open {
                "relative dots-menu-root cursor-default"
            } else {
                "relative cursor-default"
            },
            // On the root, not the panel: focus stays on the trigger when the
            // menu opens by click, so a keydown on the panel would never fire.
            onkeydown: move |evt| {
                if props.is_open && evt.key() == Key::Escape {
                    evt.stop_propagation();
                    props.on_close.call(());
                }
            },
            // A press inside the menu must never reach the row behind it: the
            // draggable rows arm a drag on mousedown and a long-press on
            // touchstart, and either one swallows the click on a menu entry.
            onmousedown: move |evt| evt.stop_propagation(),
            ontouchstart: move |evt| evt.stop_propagation(),

            button {
                r#type: "button",
                class: "cursor-pointer {base_button_class}",
                aria_label: "{props.aria_label}",
                aria_haspopup: "menu",
                aria_expanded: if props.is_open { "true" } else { "false" },
                onmounted: move |evt| trigger_element.set(Some(evt)),
                onclick: move |evt| {
                    evt.stop_propagation();
                    // Anchor to the button, never to a stale right-click point.
                    clear_context_point();
                    panel_geometry.set(None);
                    if props.is_open {
                        props.on_close.call(());
                    } else {
                        props.on_open.call(());
                    }
                },
                i { class: "{props.icon}", aria_hidden: "true" }
            }

            if props.is_open {
                div {
                    class: "fixed inset-0 dots-menu-backdrop",
                    aria_hidden: "true",
                    onclick: move |evt| {
                        evt.stop_propagation();
                        props.on_close.call(());
                    }
                }

                div {
                    class: "w-auto flex flex-col bg-neutral-900 border border-white/10 rounded-lg dots-menu-panel py-1 shadow-xl",
                    style: "{panel_style}",
                    role: "menu",
                    // Focusable only so a pointer-opened menu can be given
                    // focus below; it stays out of the tab order.
                    tabindex: "-1",
                    onmounted: {
                        let anchor = props.anchor.clone();
                        let placement = props.placement.clone();
                        move |panel_evt: MountedEvent| {
                            let trigger_evt = trigger_element.peek().clone();
                            let anchor = anchor.clone();
                            let placement = placement.clone();
                            // Consumed on mount so the next open falls back to
                            // the trigger unless another right-click sets it.
                            let pointer = take_context_point();
                            async move {
                                let Ok(panel_rect) = panel_evt.get_client_rect().await else {
                                    return;
                                };
                                let (left, top) = if let Some((x, y)) = pointer {
                                    // Opened by right-click: the pointer is the
                                    // corner the panel grows away from, mirrored
                                    // in RTL so it still opens inwards.
                                    let left = if is_rtl { x - panel_rect.width() } else { x };
                                    (left, y)
                                } else {
                                    let Some(trigger_evt) = trigger_evt else {
                                        return;
                                    };
                                    let Ok(trigger_rect) = trigger_evt.get_client_rect().await
                                    else {
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
                                if pointer.is_some() {
                                    // A right-click leaves focus wherever it
                                    // was, so Escape would never reach the root
                                    // handler. A click-opened menu keeps focus
                                    // on the trigger, which is already inside
                                    // the root, so it is left alone.
                                    let _ = panel_evt.set_focus(true).await;
                                }
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
                                    r#type: "button",
                                    role: "menuitem",
                                    class: "px-4 py-2 text-sm cursor-pointer {text_color} hover:bg-white/10 flex items-center gap-2 transition-colors whitespace-nowrap",
                                    onclick: move |_| {
                                        panel_geometry.set(None);
                                        props.on_action.call(idx);
                                    },
                                    i { class: "{icon}", aria_hidden: "true" }
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
