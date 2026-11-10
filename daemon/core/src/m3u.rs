//! Reading an M3U playlist file and finding its entries in the library.
//!
//! An exported playlist names files where the old player kept them, which is
//! rarely where this library keeps them, so an entry that is not a library
//! path is matched by its file name and as many parent folders as agree, and
//! failing that by the artist and title its `#EXTINF` line gives.

use std::collections::HashMap;

/// One entry of the file: where it says the song is, and what `#EXTINF` called it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub location: String,
    pub artist: Option<String>,
    pub title: Option<String>,
}

/// A library track as matching sees it.
#[derive(Debug, Clone)]
pub struct LibraryTrack {
    pub key: String,
    pub title: String,
    pub artist: String,
}

pub fn parse(text: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut info: Option<(Option<String>, Option<String>)> = None;
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            info = rest
                .split_once(',')
                .map(|(_, display)| split_display(display));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let (artist, title) = info.take().unwrap_or_default();
        entries.push(Entry {
            location: line.to_string(),
            artist,
            title,
        });
    }
    entries
}

/// `Artist - Title`, or a bare title when there is no separator.
fn split_display(display: &str) -> (Option<String>, Option<String>) {
    let display = display.trim();
    if display.is_empty() {
        return (None, None);
    }
    match display.split_once(" - ") {
        Some((artist, title)) if !artist.trim().is_empty() && !title.trim().is_empty() => (
            Some(artist.trim().to_string()),
            Some(title.trim().to_string()),
        ),
        _ => (None, Some(display.to_string())),
    }
}

/// The entry's path with forward slashes and no `file://`, resolved against
/// `base_dir` when relative, and `.`/`..` folded away.
fn resolve(location: &str, base_dir: Option<&str>) -> String {
    let location = match location.strip_prefix("file://") {
        Some(url) => percent_decode(url.strip_prefix("localhost").unwrap_or(url)),
        None => location.to_string(),
    };
    let location = location.replace('\\', "/");
    let absolute = location.starts_with('/') || has_drive(&location);
    let joined = match base_dir {
        Some(base) if !absolute => format!("{}/{location}", base.replace('\\', "/")),
        _ => location,
    };
    let rooted = joined.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    let path = parts.join("/");
    if rooted { format!("/{path}") } else { path }
}

fn has_drive(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(byte) = value
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn components(path: &str) -> Vec<String> {
    path.replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn fold(value: &str) -> String {
    value.trim().to_lowercase()
}

/// The library key each entry plays as, in the file's order; `None` for an entry nothing answers to.
pub fn match_entries(
    entries: &[Entry],
    base_dir: Option<&str>,
    library: &[LibraryTrack],
) -> Vec<Option<String>> {
    let keys: HashMap<String, usize> = library
        .iter()
        .enumerate()
        .map(|(index, track)| (track.key.replace('\\', "/"), index))
        .collect();
    let mut by_name: HashMap<String, Vec<(usize, Vec<String>)>> = HashMap::new();
    for (index, track) in library.iter().enumerate() {
        let parts = components(&track.key);
        if let Some(name) = parts.last() {
            by_name
                .entry(name.clone())
                .or_default()
                .push((index, parts.clone()));
        }
    }
    let mut by_tags: HashMap<(String, String), Vec<usize>> = HashMap::new();
    let mut by_title: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, track) in library.iter().enumerate() {
        by_tags
            .entry((fold(&track.artist), fold(&track.title)))
            .or_default()
            .push(index);
        by_title.entry(fold(&track.title)).or_default().push(index);
    }

    entries
        .iter()
        .map(|entry| {
            let path = resolve(&entry.location, base_dir);
            let found = keys
                .get(&path)
                .copied()
                .or_else(|| by_path_tail(&path, &by_name))
                .or_else(|| by_info(entry, &by_tags, &by_title));
            found.map(|index| library[index].key.clone())
        })
        .collect()
}

/// The library file sharing the entry's name and the most parent folders, when that is one file.
fn by_path_tail(path: &str, by_name: &HashMap<String, Vec<(usize, Vec<String>)>>) -> Option<usize> {
    let wanted = components(path);
    let candidates = by_name.get(wanted.last()?)?;
    let shared = |parts: &[String]| {
        wanted
            .iter()
            .rev()
            .zip(parts.iter().rev())
            .take_while(|(a, b)| a == b)
            .count()
    };
    let mut best: Option<(usize, usize)> = None;
    let mut tied = false;
    for (index, parts) in candidates {
        let score = shared(parts);
        match best {
            Some((_, top)) if score < top => {}
            Some((_, top)) if score == top => tied = true,
            _ => {
                best = Some((*index, score));
                tied = false;
            }
        }
    }
    best.filter(|_| !tied).map(|(index, _)| index)
}

/// The one library track with the entry's `#EXTINF` artist and title, or its title alone when it gave no artist.
fn by_info(
    entry: &Entry,
    by_tags: &HashMap<(String, String), Vec<usize>>,
    by_title: &HashMap<String, Vec<usize>>,
) -> Option<usize> {
    let title = fold(entry.title.as_deref()?);
    let matches = match &entry.artist {
        Some(artist) => by_tags.get(&(fold(artist), title))?,
        None => by_title.get(&title)?,
    };
    match matches.as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(rows: &[(&str, &str, &str)]) -> Vec<LibraryTrack> {
        rows.iter()
            .map(|(key, artist, title)| LibraryTrack {
                key: key.to_string(),
                artist: artist.to_string(),
                title: title.to_string(),
            })
            .collect()
    }

    #[test]
    fn extended_m3u_entries_carry_their_display_names() {
        let entries = parse(
            "\u{feff}#EXTM3U\r\n#EXTINF:215,Ada - Hello World\r\nA/1.flac\r\n\r\n#EXTINF:-1,Untitled\nB/2.mp3\nC/3.ogg\n",
        );
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].location, "A/1.flac");
        assert_eq!(entries[0].artist.as_deref(), Some("Ada"));
        assert_eq!(entries[0].title.as_deref(), Some("Hello World"));
        assert_eq!(entries[1].artist, None);
        assert_eq!(entries[1].title.as_deref(), Some("Untitled"));
        assert_eq!(
            entries[2].title, None,
            "info does not leak to the next entry"
        );
    }

    #[test]
    fn paths_resolve_against_the_file_and_from_urls() {
        assert_eq!(resolve("../b/1.flac", Some("/music/a")), "/music/b/1.flac");
        assert_eq!(
            resolve("file:///music/My%20Song.flac", None),
            "/music/My Song.flac"
        );
        assert_eq!(
            resolve("C:\\Music\\a.flac", Some("/elsewhere")),
            "C:/Music/a.flac"
        );
    }

    #[test]
    fn entries_match_by_path_then_folders_then_tags() {
        let library = library(&[
            ("/lib/Ada/Hits/01 Intro.flac", "Ada", "Intro"),
            ("/lib/Bo/Hits/01 Intro.flac", "Bo", "Intro"),
            ("/lib/Cy/Live/02 Song.flac", "Cy", "Song"),
            ("/lib/Dee/Solo/track.flac", "Dee", "Only One"),
        ]);
        let entries = parse(
            "#EXTM3U\n\
             /lib/Cy/Live/02 Song.flac\n\
             D:\\Old\\Bo\\Hits\\01 Intro.flac\n\
             /old/Hits/01 Intro.flac\n\
             #EXTINF:1,Dee - Only One\n\
             gone.mp3\n\
             nothing.mp3\n",
        );
        let matched = match_entries(&entries, Some("/old"), &library);
        assert_eq!(
            matched,
            [
                Some("/lib/Cy/Live/02 Song.flac".to_string()),
                Some("/lib/Bo/Hits/01 Intro.flac".to_string()),
                None,
                Some("/lib/Dee/Solo/track.flac".to_string()),
                None,
            ]
        );
    }
}
