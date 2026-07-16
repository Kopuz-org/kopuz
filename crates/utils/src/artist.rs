//! Artist-name keying shared by the UI and the photo-fetch pipeline.

/// The key artist caches are stored under (the `artist_images` table's
/// `artist_norm`, the fetch skip-sets): trimmed, lowercased display name.
pub fn normalize_artist_key(value: &str) -> String {
    value.trim().to_lowercase()
}

/// The normalized primary artist of a joined collab credit ("COOL&CREATE,
/// beatMARIO, & MARON" → "cool&create"), or None for a plain name. Older
/// synced rows (and album-artist fields) still carry such joined strings as
/// one "artist"; when the primary also exists on its own, the joined entry is
/// a duplicate tile wearing the primary's photo. Only ever used to drop a
/// credit whose primary is independently present — a legit comma name
/// ("Tyler, The Creator") has no standalone "tyler" and is left alone.
pub fn joined_credit_primary(norm: &str) -> Option<&str> {
    let (primary, rest) = norm.split_once(',')?;
    let primary = primary.trim();
    (!primary.is_empty() && !rest.trim().is_empty()).then_some(primary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_and_lowercases() {
        assert_eq!(normalize_artist_key("  COOL&CREATE "), "cool&create");
    }

    #[test]
    fn joined_credit_primary_splits_only_real_joins() {
        assert_eq!(
            joined_credit_primary("cool&create, beatmario, & maron"),
            Some("cool&create")
        );
        // A plain name and a trailing comma are not joins.
        assert_eq!(joined_credit_primary("cool&create"), None);
        assert_eq!(joined_credit_primary("name,"), None);
        assert_eq!(joined_credit_primary(", name"), None);
        // "Tyler, The Creator" splits — the CALLER only drops it when a
        // standalone "tyler" tile exists, which it doesn't for legit names.
        assert_eq!(joined_credit_primary("tyler, the creator"), Some("tyler"));
    }
}
