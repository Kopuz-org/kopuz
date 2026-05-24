mod test_support;

use components::navigation_controller::NavigationController;
use dioxus::prelude::*;
use kopuz_route::Route;

#[derive(Props, Clone, PartialEq)]
struct NavigationHarnessProps {
    target_album: String,
    target_artist: String,
}

#[component]
fn NavigationHarness(props: NavigationHarnessProps) -> Element {
    let current_route = use_signal(|| Route::Home);
    let selected_artist_name = use_signal(String::new);
    let selected_album_id = use_signal(String::new);
    let mut did_navigate = use_signal(|| false);

    use_context_provider(|| NavigationController {
        current_route,
        selected_artist_name,
        selected_album_id,
    });

    let nav = use_context::<NavigationController>();

    use_effect(move || {
        if *did_navigate.read() {
            return;
        }

        did_navigate.set(true);
        if !props.target_album.is_empty() {
            nav.navigate_to_album(props.target_album.clone());
        }
        if !props.target_artist.is_empty() {
            nav.navigate_to_artist(props.target_artist.clone());
        }
    });

    let status = format!(
        "route:{:?}|album:{}|artist:{}",
        *current_route.read(),
        selected_album_id.read().as_str(),
        selected_artist_name.read().as_str()
    );

    rsx! {
        div {
            "{status}"
        }
    }
}

#[tokio::test]
async fn navigation_controller_context_updates_album_route_and_selection() {
    let html = test_support::render_dom_with_work(VirtualDom::new_with_props(
        NavigationHarness,
        NavigationHarnessProps {
            target_album: "alb_patchwork".to_string(),
            target_artist: String::new(),
        },
    ))
    .await;

    assert!(html.contains("route:Album"));
    assert!(html.contains("album:alb_patchwork"));
    assert!(html.contains("artist:"));
}

#[tokio::test]
async fn navigation_controller_context_updates_artist_route_and_selection() {
    let html = test_support::render_dom_with_work(VirtualDom::new_with_props(
        NavigationHarness,
        NavigationHarnessProps {
            target_album: String::new(),
            target_artist: "Alohalii".to_string(),
        },
    ))
    .await;

    assert!(html.contains("route:Artist"));
    assert!(html.contains("artist:Alohalii"));
    assert!(html.contains("album:"));
}
