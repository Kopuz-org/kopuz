//! Regression tests for multi-artist tag parsing (issue #314).
//!
//! Downloaded files often carry a single joined `ARTIST` tag. The scanner
//! must split `;`-separated values into `artists`, expose the primary
//! artist via the singular `artist` field, and never guess on commas
//! (names like "Tyler, The Creator" stay intact).

use reader::{Library, Track, read};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn read_fixture(name: &str) -> (Track, Library) {
    let cache = std::env::temp_dir();
    let mut lib = Library::default();
    let track = read(&fixture(name), &cache, &mut lib)
        .unwrap_or_else(|| panic!("failed to read fixture {name}"));
    (track, lib)
}

fn album_artist(lib: &Library) -> String {
    lib.albums
        .first()
        .map(|a| a.artist.clone())
        .expect("album should be created")
}

#[test]
fn single_artist_unchanged() {
    let (track, lib) = read_fixture("single.opus");
    assert_eq!(track.artist, "Solo Artist");
    assert_eq!(track.artists, vec!["Solo Artist"]);
    assert_eq!(album_artist(&lib), "Solo Artist");
}

#[test]
fn semicolon_tag_splits_and_primary_artist_is_first() {
    let (track, lib) = read_fixture("semicolon.opus");
    assert_eq!(
        track.artists,
        vec!["Kero Kero Bonito", "Douglas Lobban", "Sarah Perry"]
    );
    // Singular artist must be the primary (first) artist, not the joined
    // string, otherwise the artists page shows a phantom "A;B;C" entry.
    assert_eq!(track.artist, "Kero Kero Bonito");
    // Album artist falls back to track.artist when no ALBUMARTIST tag.
    assert_eq!(album_artist(&lib), "Kero Kero Bonito");
}

#[test]
fn comma_tag_is_not_guessed() {
    // Comma-joined tags stay as-is on the read side: a comma may be part
    // of a real name, so splitting is the writer's job (yt-dlp download fix).
    let (track, lib) = read_fixture("comma.opus");
    let joined = "Kero Kero Bonito, Douglas Lobban, Sarah Perry";
    assert_eq!(track.artist, joined);
    assert_eq!(track.artists, vec![joined]);
    assert_eq!(album_artist(&lib), joined);
}

#[test]
fn comma_in_artist_name_stays_intact() {
    let (track, lib) = read_fixture("tyler.opus");
    assert_eq!(track.artist, "Tyler, The Creator");
    assert_eq!(track.artists, vec!["Tyler, The Creator"]);
    assert_eq!(album_artist(&lib), "Tyler, The Creator");
}

#[test]
fn explicit_album_artist_wins() {
    let (track, lib) = read_fixture("albumartist.opus");
    assert_eq!(track.artists, vec!["First One", "Second One"]);
    assert_eq!(track.artist, "First One");
    // ALBUMARTIST tag takes precedence over the track artist fallback.
    assert_eq!(album_artist(&lib), "The Band");
}
