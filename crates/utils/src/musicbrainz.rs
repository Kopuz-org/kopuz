//! MusicBrainz web links, built from what a row already carries. The lookup
//! that needs the network is a daemon job and lives in `server::musicbrainz`.

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

/// A shareable MusicBrainz page for a track: its release when the tags name
/// one, else a search the site resolves.
pub fn track_page_url(release_id: Option<&str>, artist: &str, title: &str) -> Option<String> {
    if let Some(id) = release_id {
        let id = id.trim();
        if !id.is_empty() {
            return Some(format!("https://musicbrainz.org/release/{id}"));
        }
    }

    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    Some(format!(
        "https://musicbrainz.org/search?query={}&type=recording",
        utf8_percent_encode(&recording_query(artist, title), NON_ALPHANUMERIC)
    ))
}

/// The Lucene query naming one recording, shared with the server-side lookup.
pub fn recording_query(artist: &str, title: &str) -> String {
    let mut query = format!("recording:\"{}\"", escape_lucene(title.trim()));
    let artist = artist.trim();
    if !artist.is_empty() {
        query.push_str(&format!(" AND artist:\"{}\"", escape_lucene(artist)));
    }
    query
}

fn escape_lucene(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}
