use config::AppConfig;
use dioxus::prelude::*;

pub(crate) fn high_quality_artwork_url(cover: String) -> String {
    if cover.starts_with("artwork://") || cover.starts_with("http://artwork.dioxus.localhost/") {
        format!("{cover}&hq=1")
    } else {
        cover
    }
}

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

    let src = high_quality_artwork_url(cover);

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

#[cfg(test)]
mod tests {
    use super::high_quality_artwork_url;

    #[test]
    fn local_artwork_uses_the_hq_protocol_variant() {
        assert_eq!(
            high_quality_artwork_url("artwork://local?p=%2Fcover.jpg".to_string()),
            "artwork://local?p=%2Fcover.jpg&hq=1"
        );
        assert_eq!(
            high_quality_artwork_url(
                "http://artwork.dioxus.localhost/local?p=C%3A%5Ccover.jpg".to_string()
            ),
            "http://artwork.dioxus.localhost/local?p=C%3A%5Ccover.jpg&hq=1"
        );
    }
}
