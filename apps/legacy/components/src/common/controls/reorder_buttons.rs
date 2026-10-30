use dioxus::prelude::*;

/// Up/down arrows that split the row's full height between them, so each
/// half of the strip is a target rather than just the chevron.
#[component]
pub fn ReorderButtons(
    can_move_up: bool,
    can_move_down: bool,
    on_move_up: EventHandler<MouseEvent>,
    on_move_down: EventHandler<MouseEvent>,
    #[props(default = "pr-2".to_string())] class: String,
    #[props(default = "text-[9px]".to_string())] icon_class: String,
) -> Element {
    let enabled = "w-7 flex-1 min-h-0 flex items-center justify-center rounded-md text-slate-500 hover:bg-white/10 hover:text-white active:bg-white/15 transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500";
    let disabled = "w-7 flex-1 min-h-0 flex items-center justify-center rounded-md text-slate-700 cursor-default";
    rsx! {
        div { class: "flex flex-col self-stretch shrink-0 {class}",
            button {
                disabled: !can_move_up,
                class: if can_move_up { enabled } else { disabled },
                onclick: move |evt| {
                    evt.stop_propagation();
                    if can_move_up {
                        on_move_up.call(evt);
                    }
                },
                ondoubleclick: move |evt| evt.stop_propagation(),
                i { class: "fa-solid fa-chevron-up {icon_class}" }
            }
            button {
                disabled: !can_move_down,
                class: if can_move_down { enabled } else { disabled },
                onclick: move |evt| {
                    evt.stop_propagation();
                    if can_move_down {
                        on_move_down.call(evt);
                    }
                },
                ondoubleclick: move |evt| evt.stop_propagation(),
                i { class: "fa-solid fa-chevron-down {icon_class}" }
            }
        }
    }
}
