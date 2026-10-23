//! A window with nothing in it but the palette, for looking at what it draws.

use gpui_kit::component::{ActiveTheme as _, Root};
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Render, Styled,
    Window, WindowBounds, WindowOptions, div, px, size,
};
use store::Chrome;

struct Preview {
    palette: Entity<palette::Palette>,
}

impl Preview {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let palette = cx.new(|cx| palette::Palette::new(window, cx));
        Chrome::global(cx).update(cx, |chrome, cx| chrome.set_quick_search(true, cx));
        // The palette clears itself when it is told it has opened, so the query lands after that.
        if let Ok(query) = std::env::var("PREVIEW_QUERY") {
            let held = palette.clone();
            cx.spawn_in(window, async move |_, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                held.update_in(cx, |palette, window, cx| palette.seed(&query, window, cx))
                    .ok();
            })
            .detach();
        }
        Self { palette }
    }
}

impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .bg(cx.theme().background)
            .child(self.palette.clone())
    }
}

fn main() {
    let socket = std::env::var_os("KOPUZ_SOCKET")
        .map(Into::into)
        .or_else(link::default_socket_path)
        .expect("a runtime directory to find the daemon socket in");
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            i18n::init("en");
            link::init(link::Address::Socket(socket), cx).expect("the daemon link");
            store::init(cx);
            settings_interface::init(cx);
            palette::register_settings(cx);
            commands::init(cx);
            theme::apply_named("default", None, cx);
            commands::keymap(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1000.), px(700.)),
                    cx,
                ))),
                window_background: gpui_kit::WindowBackgroundAppearance::Opaque,
                app_id: Some("kopuz".into()),
                ..Default::default()
            };
            cx.open_window(options, |window, cx| {
                let preview = cx.new(|cx| Preview::new(window, cx));
                cx.new(|cx| Root::new(preview, window, cx))
            })
            .expect("failed to open the preview window");
        });
}
