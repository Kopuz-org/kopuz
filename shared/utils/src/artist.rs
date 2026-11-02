//! Artist-name keying shared by the UI and the photo-fetch pipeline.

/// The folded form of an artist's name (`artists.name_key`): trimmed, lowercased.
pub fn normalize_artist_key(value: &str) -> String {
    value.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_and_lowercases() {
        assert_eq!(normalize_artist_key("  COOL&CREATE "), "cool&create");
    }
}
