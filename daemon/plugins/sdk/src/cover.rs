//! Cover references in the form the host stores them. An id the host signs
//! when it fetches the picture keeps credentials out of the database.

pub const NO_COVER: &str = "none";

/// `service:item[:value]`. The host splits on the first two colons, so pass
/// `None` for `value` when the item id can hold a colon.
pub fn item_ref(service: &str, item_id: &str, value: Option<&str>) -> String {
    match value {
        Some(value) => format!("{service}:{item_id}:{value}"),
        None => format!("{service}:{item_id}"),
    }
}

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
