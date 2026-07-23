use config::AppConfig;
use dioxus::prelude::*;

/// Art backdrop: the cover under user-configurable blur and darkening so
/// text stays readable. The overscan grows with the blur radius to keep
/// blurred edge bleed outside the viewport.
#[component]
pub fn CoverArtBackground(cover: String) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let (scrim, blur) = {
        let conf = config.read();
        (
            conf.cover_art_darkening.min(95) as f32 / 100.0,
            conf.cover_art_blur.min(100),
        )
    };
    let img_style = if blur > 0 {
        let scale = 1.0 + blur as f32 * 0.004;
        format!("filter: blur({blur}px); transform: scale({scale});")
    } else {
        "filter: none; transform: none;".to_string()
    };

    let src = if cover.starts_with("artwork://") {
        format!("{cover}&hq=1")
    } else {
        cover
    };

    rsx! {
        div {
            class: "absolute inset-0 -z-10 overflow-hidden pointer-events-none bg-black",
            img {
                src: "{src}",
                class: "w-full h-full object-cover",
                style: "{img_style}",
            }
            div {
                class: "absolute inset-0",
                style: "background-color: rgba(0, 0, 0, {scrim});",
            }
        }
    }
}
