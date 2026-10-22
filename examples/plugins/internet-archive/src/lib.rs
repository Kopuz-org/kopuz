//! An example community plugin adding a service kopuz has no built-in for:
//! a source is the address of an archive.org item (one album) or collection
//! (its most downloaded items). `scripts/plugin_market.nu` serves it from a
//! local marketplace.

use kopuz_plugin_sdk::http::Client;
use kopuz_plugin_sdk::{
    Album, ArtistCredit, AuthOutcome, Connection, LibrarySnapshot, Result, Source, SourceError,
    StreamInfo, StreamQuality, Track, cover,
};
use serde::Deserialize;

const ARCHIVE: &str = "https://archive.org";

/// How many items of a collection become albums.
const COLLECTION_ITEMS: usize = 50;

/// Audio formats in the order they are preferred; an item's tracks are its
/// files in the first one it has.
const FORMATS: &[&str] = &["VBR MP3", "MP3", "128Kbps MP3", "Ogg Vorbis", "Flac"];

pub struct InternetArchive {
    http: Client,
    /// The item or collection the source was added with.
    identifier: Option<String>,
}

kopuz_plugin_sdk::export_source!(InternetArchive);

#[derive(Debug, Deserialize, Default)]
struct Metadata {
    #[serde(default)]
    metadata: ItemMetadata,
    #[serde(default)]
    files: Vec<File>,
}

#[derive(Debug, Deserialize, Default)]
struct ItemMetadata {
    #[serde(default)]
    identifier: String,
    #[serde(default)]
    mediatype: Option<String>,
    #[serde(default)]
    title: Option<Value>,
    #[serde(default)]
    creator: Option<Value>,
    #[serde(default)]
    date: Option<Value>,
}

/// The metadata API gives a field as a string or a list of them.
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
enum Value {
    One(String),
    Many(Vec<String>),
}

impl Value {
    fn first(&self) -> &str {
        match self {
            Value::One(value) => value,
            Value::Many(values) => values.first().map(String::as_str).unwrap_or_default(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
struct File {
    name: String,
    #[serde(default)]
    format: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    creator: Option<String>,
    #[serde(default)]
    track: Option<String>,
    #[serde(default)]
    length: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    response: SearchDocs,
}

#[derive(Debug, Deserialize)]
struct SearchDocs {
    #[serde(default)]
    docs: Vec<SearchDoc>,
}

#[derive(Debug, Deserialize)]
struct SearchDoc {
    identifier: String,
}

/// The identifier in an archive.org address: `/details/<id>` or a bare id.
fn identifier_of(address: &str) -> Option<String> {
    let address = address.trim().trim_end_matches('/');
    let id = match url::Url::parse(address) {
        Ok(url) => {
            let mut segments = url.path_segments()?;
            match segments.next()? {
                "details" | "download" | "metadata" => segments.next()?.to_string(),
                _ => return None,
            }
        }
        Err(_) => address.to_string(),
    };
    let valid = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    valid.then_some(id)
}

/// "123.45" seconds or "m:ss" / "h:mm:ss".
fn seconds(length: &str) -> u64 {
    if let Ok(secs) = length.parse::<f64>() {
        return secs.max(0.0) as u64;
    }
    length
        .split(':')
        .try_fold(0u64, |total, part| {
            part.trim()
                .parse::<f64>()
                .ok()
                .map(|n| total * 60 + n as u64)
        })
        .unwrap_or(0)
}

/// "3" or "3/12".
fn track_number(track: &str) -> Option<u32> {
    track.split('/').next()?.trim().parse().ok()
}

/// An item's audio files, in the first preferred format it has.
fn audio_files(files: &[File]) -> Vec<&File> {
    let Some(format) = FORMATS
        .iter()
        .find(|format| files.iter().any(|file| file.format == **format))
    else {
        return Vec::new();
    };
    let mut audio: Vec<&File> = files.iter().filter(|file| file.format == *format).collect();
    audio.sort_by_key(|file| {
        (
            file.track
                .as_deref()
                .and_then(track_number)
                .unwrap_or(u32::MAX),
            file.name.clone(),
        )
    });
    audio
}

/// One item as an album and its tracks. A track's id is
/// `<identifier>/<file name>`, everything a stream URL needs.
fn album_of(item: &Metadata) -> Option<(Album, Vec<Track>)> {
    let identifier = &item.metadata.identifier;
    if identifier.is_empty() {
        return None;
    }
    let title = item
        .metadata
        .title
        .as_ref()
        .map(|title| title.first().to_string())
        .unwrap_or_else(|| identifier.clone());
    let creator = item
        .metadata
        .creator
        .as_ref()
        .map(|creator| creator.first().to_string())
        .unwrap_or_default();
    let year = item
        .metadata
        .date
        .as_ref()
        .and_then(|date| date.first().get(..4)?.parse().ok())
        .unwrap_or(0);
    let album_id = cover::item_ref("plugin", identifier, None);
    let art = cover::url(&format!("{ARCHIVE}/services/img/{identifier}"));
    let tracks: Vec<Track> = audio_files(&item.files)
        .into_iter()
        .map(|file| {
            let artist = file.creator.clone().unwrap_or_else(|| creator.clone());
            Track {
                item_id: format!("{identifier}/{}", file.name),
                cover: Some(art.clone()),
                album_id: album_id.clone(),
                title: file.title.clone().unwrap_or_else(|| file.name.clone()),
                album: title.clone(),
                duration_secs: file.length.as_deref().map(seconds).unwrap_or(0),
                track_number: file.track.as_deref().and_then(track_number),
                artists: vec![artist.clone()],
                credits: vec![ArtistCredit::unlinked(&artist)],
                artist,
                ..Track::default()
            }
        })
        .collect();
    if tracks.is_empty() {
        return None;
    }
    Some((
        Album {
            id: album_id,
            title,
            artist: creator,
            genre: String::new(),
            year,
            cover: Some(art),
            artist_id: None,
        },
        tracks,
    ))
}

impl InternetArchive {
    fn identifier(&self) -> Result<&str> {
        self.identifier.as_deref().ok_or_else(|| {
            SourceError::InvalidInput("not an archive.org item or collection address".into())
        })
    }

    fn metadata(&self, identifier: &str) -> Result<Metadata> {
        let response = self
            .http
            .get(format!("{ARCHIVE}/metadata/{identifier}"))
            .send()?;
        if !response.is_success() {
            return Err(SourceError::from_status(
                response.status(),
                "archive.org metadata",
            ));
        }
        response
            .json()
            .map_err(|error| SourceError::Backend(error.to_string()))
    }

    fn collection_items(&self, collection: &str) -> Result<Vec<String>> {
        let query = format!("collection:{collection} AND mediatype:audio");
        let rows = COLLECTION_ITEMS.to_string();
        let response = self
            .http
            .get(format!("{ARCHIVE}/advancedsearch.php"))
            .query(&[
                ("q", query.as_str()),
                ("fl[]", "identifier"),
                ("sort[]", "downloads desc"),
                ("rows", rows.as_str()),
                ("output", "json"),
            ])
            .send()?;
        if !response.is_success() {
            return Err(SourceError::from_status(
                response.status(),
                "archive.org search",
            ));
        }
        let found: SearchResponse = response
            .json()
            .map_err(|error| SourceError::Backend(error.to_string()))?;
        Ok(found
            .response
            .docs
            .into_iter()
            .map(|doc| doc.identifier)
            .collect())
    }
}

impl Source for InternetArchive {
    fn open(connection: Connection) -> Self {
        Self {
            http: Client::new().with_timeout(std::time::Duration::from_secs(30)),
            identifier: identifier_of(&connection.url),
        }
    }

    fn validate(&self) -> AuthOutcome {
        let Ok(identifier) = self.identifier() else {
            return AuthOutcome::Unreachable;
        };
        match self.metadata(identifier) {
            Ok(item) if !item.metadata.identifier.is_empty() => AuthOutcome::Valid,
            _ => AuthOutcome::Unreachable,
        }
    }

    fn fetch_library(&self) -> Result<LibrarySnapshot> {
        let identifier = self.identifier()?;
        let root = self.metadata(identifier)?;
        let items = if root.metadata.mediatype.as_deref() == Some("collection") {
            let mut items = Vec::new();
            for id in self.collection_items(identifier)? {
                match self.metadata(&id) {
                    Ok(item) => items.push(item),
                    Err(error) => kopuz_plugin_sdk::warn!("skipping {id}: {error}"),
                }
            }
            items
        } else {
            vec![root]
        };
        let mut albums = Vec::new();
        let mut tracks = Vec::new();
        for (album, album_tracks) in items.iter().filter_map(album_of) {
            albums.push(album);
            tracks.extend(album_tracks);
        }
        Ok(LibrarySnapshot {
            albums,
            tracks,
            artists: Vec::new(),
        })
    }

    fn resolve_stream(&self, item_id: &str, _: StreamQuality) -> Result<StreamInfo> {
        let (identifier, file) = item_id
            .split_once('/')
            .ok_or_else(|| SourceError::InvalidInput("not an archive.org track".into()))?;
        let mut url = url::Url::parse(&format!("{ARCHIVE}/download/"))
            .map_err(|error| SourceError::Backend(error.to_string()))?;
        url.path_segments_mut()
            .map_err(|()| SourceError::Backend("archive.org URL".into()))?
            .pop_if_empty()
            .push(identifier)
            .extend(file.split('/'));
        Ok(StreamInfo::url(url.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_names_its_item() {
        assert_eq!(
            identifier_of("https://archive.org/details/gd1977-05-08.sbd.miller.97065/").as_deref(),
            Some("gd1977-05-08.sbd.miller.97065")
        );
        assert_eq!(identifier_of("etree").as_deref(), Some("etree"));
        assert_eq!(identifier_of("https://archive.org/search?q=x"), None);
        assert_eq!(identifier_of("../../etc"), None);
    }

    #[test]
    fn lengths_read_as_seconds_or_clock_time() {
        assert_eq!(seconds("123.9"), 123);
        assert_eq!(seconds("3:05"), 185);
        assert_eq!(seconds("1:00:01"), 3601);
        assert_eq!(seconds("n/a"), 0);
    }

    #[test]
    fn an_item_becomes_an_album_of_its_preferred_format() {
        let item: Metadata = serde_json::from_value(serde_json::json!({
            "metadata": {
                "identifier": "live-1977",
                "mediatype": "etree",
                "title": "Live in 1977",
                "creator": ["The Band"],
                "date": "1977-05-08"
            },
            "files": [
                { "name": "02.flac", "format": "Flac", "track": "2" },
                { "name": "02.mp3", "format": "VBR MP3", "title": "Second", "track": "2/2", "length": "3:05" },
                { "name": "01.mp3", "format": "VBR MP3", "title": "First", "track": "1/2", "length": "61.5" },
                { "name": "cover.jpg", "format": "JPEG" }
            ]
        }))
        .unwrap();
        let (album, tracks) = album_of(&item).expect("an album");
        assert_eq!(album.title, "Live in 1977");
        assert_eq!(album.artist, "The Band");
        assert_eq!(album.year, 1977);
        let titles: Vec<_> = tracks.iter().map(|track| track.title.as_str()).collect();
        assert_eq!(titles, ["First", "Second"]);
        assert_eq!(tracks[0].item_id, "live-1977/01.mp3");
        assert_eq!(tracks[1].duration_secs, 185);
        assert_eq!(tracks[0].album_id, "plugin:live-1977");
        assert_eq!(
            tracks[0].cover.as_deref(),
            Some("directurl:https://archive.org/services/img/live-1977")
        );
    }

    #[test]
    fn a_stream_url_encodes_the_file_name() {
        let source = InternetArchive::open(Connection {
            service: "internet-archive".into(),
            url: "https://archive.org/details/live-1977".into(),
            token: String::new(),
            user_id: String::new(),
            device_id: String::new(),
            account: None,
            folders: Vec::new(),
        });
        let stream = source
            .resolve_stream("live-1977/disc 1/01 opener.mp3", StreamQuality::High)
            .unwrap();
        assert_eq!(
            stream.url,
            "https://archive.org/download/live-1977/disc%201/01%20opener.mp3"
        );
    }
}
