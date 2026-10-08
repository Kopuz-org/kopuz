const FAVICON: &str = include_str!(concat!(env!("OUT_DIR"), "/favicon.uri"));
// CSS/fonts are compiled in (not `asset!()`-collected) so styling works under a
// bare `cargo run` — see `build.rs::embed_fonts`, which bakes the font data: URIs.
const MAIN_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/main.css"));
const THEME_CSS: &str = include_str!("../assets/themes.css");
const MATERIAL3_CSS: &str = include_str!("../assets/material3.css");
const TAILWIND_CSS: &str = include_str!("../assets/tailwind.css");
const REDUCED_ANIMATIONS_CSS: &str = include_str!("../assets/reduced-animations.css");
const FONT_AWESOME_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/fontawesome.css"));
const JETBRAINS_MONO_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/jetbrains-mono.css"));
const ROBOTO_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/roboto.css"));

/// Put static assets in the initial document. `document::Style` sends the CSS
/// and embedded fonts over the WebView's JavaScript bridge after the first DOM
/// edits, allowing an unstyled frame and delaying font decoding on Android.
pub fn head() -> String {
    // Dioxus's default index has no charset; raw CSS must decode as UTF-8.
    let mut head = String::from("<meta charset=\"utf-8\">");
    head.push_str("<link rel=\"icon\" href=\"");
    head.push_str(FAVICON);
    head.push_str("\">");
    for (name, css) in [
        ("main", MAIN_CSS),
        ("themes", THEME_CSS),
        ("tailwind", TAILWIND_CSS),
        ("material3", MATERIAL3_CSS),
        ("reduced-animations", REDUCED_ANIMATIONS_CSS),
        ("jetbrains-mono", JETBRAINS_MONO_CSS),
        ("roboto", ROBOTO_CSS),
        ("fontawesome", FONT_AWESOME_CSS),
    ] {
        head.push_str("<style id=\"kopuz-");
        head.push_str(name);
        head.push_str("\">");
        head.push_str(css);
        head.push_str("</style>");
    }
    head
}
