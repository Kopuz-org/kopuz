use dioxus::{dioxus_core::NoOpMutations, prelude::VirtualDom};
use std::time::Duration;

pub async fn render_dom_with_work(mut dom: VirtualDom) -> String {
    dom.rebuild_in_place();

    for _ in 0..8 {
        match tokio::time::timeout(Duration::from_millis(20), dom.wait_for_work()).await {
            Ok(_) => dom.render_immediate(&mut NoOpMutations),
            Err(_) => break,
        }
    }

    dioxus_ssr::render(&dom)
}
