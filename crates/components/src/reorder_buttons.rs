use dioxus::prelude::*;

#[component]
pub fn ReorderButtons(
    can_move_up: bool,
    can_move_down: bool,
    on_move_up: EventHandler<MouseEvent>,
    on_move_down: EventHandler<MouseEvent>,
    #[props(default = "flex flex-col pr-2 shrink-0".to_string())] class: String,
    #[props(default = "text-[9px]".to_string())] icon_class: String,
) -> Element {
    rsx! {
        div { class: "{class}",
            button {
                disabled: !can_move_up,
                class: if can_move_up {
                    "p-0.5 text-slate-500 hover:text-white transition-colors"
                } else {
                    "p-0.5 text-slate-700 cursor-default"
                },
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
                class: if can_move_down {
                    "p-0.5 text-slate-500 hover:text-white transition-colors"
                } else {
                    "p-0.5 text-slate-700 cursor-default"
                },
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
