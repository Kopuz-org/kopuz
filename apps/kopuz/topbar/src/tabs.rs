use components::icons;

/// The glyph a destination wears in the bar.
pub(crate) fn icon(name: &str) -> &'static str {
    match name {
        "home" => icons::HOME,
        "library" => icons::LIBRARY,
        "discover" => icons::COMPASS,
        "radio" => icons::RADIO,
        "playlists" => icons::LIST,
        "favorites" => icons::HEART,
        "downloader" => icons::DOWNLOAD,
        "activity" => icons::ACTIVITY,
        _ => icons::MUSIC,
    }
}

/// The i18n key a destination is named by; every one of these is already in `en.ftl`.
pub(crate) fn label(name: &str) -> &'static str {
    match name {
        "home" => "home",
        "library" => "library",
        "discover" => "discover",
        "radio" => "radio",
        "playlists" => "playlists",
        "favorites" => "favorites",
        "downloader" => "downloader",
        "activity" => "activity",
        _ => "home",
    }
}

/// The registry id a destination's tab shares, so its tooltip can name the key it answers to.
pub(crate) fn command(name: &str) -> &'static str {
    match name {
        "home" => "nav.home",
        "library" => "nav.library",
        "discover" => "nav.discover",
        "radio" => "nav.radio",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use store::layout::DESTINATIONS;

    #[test]
    fn every_destination_has_a_glyph_and_a_name() {
        for name in DESTINATIONS {
            assert_ne!(icon(name), icons::MUSIC, "{name} has no glyph");
            assert_eq!(label(name), *name, "{name} is named after something else");
        }
    }
}
