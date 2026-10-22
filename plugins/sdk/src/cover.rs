//! Cover references in the form the host stores them.
//!
//! A track's or album's `cover` is not always a URL: a server whose images
//! need credentials hands out an id, and the host signs it only when it
//! fetches the picture, so no credential is ever stored. These helpers write
//! each form the host reads back.

/// The stored form of "the source reported no cover for this item".
pub const NO_COVER: &str = "none";

/// An item-scoped reference, `service:item[:value]`, which the host resolves
/// against the server that issued it.
///
/// The host splits on the first two colons, so pass `None` for `value` when
/// the item id can hold a colon itself.
pub fn item_ref(service: &str, item_id: &str, value: Option<&str>) -> String {
    match value {
        Some(value) => format!("{service}:{item_id}:{value}"),
        None => format!("{service}:{item_id}"),
    }
}

/// A picture any client can fetch as is.
pub fn url(url: &str) -> String {
    format!("directurl:{url}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_take_the_forms_the_host_parses() {
        assert_eq!(item_ref("subsonic", "al-1", None), "subsonic:al-1");
        assert_eq!(item_ref("jellyfin", "id", Some("tag")), "jellyfin:id:tag");
        assert_eq!(
            url("https://img.test/a.jpg"),
            "directurl:https://img.test/a.jpg"
        );
    }
}
