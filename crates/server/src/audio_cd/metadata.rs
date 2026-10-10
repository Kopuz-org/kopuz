//! Exact TOC lookup, independent of drive I/O and playback. A lookup failure
//! leaves the numbered listing usable; conflicting releases are never guessed.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use sha1::{Digest, Sha1};
use tokio::sync::Mutex;

use super::Disc;
use crate::source::LibrarySnapshot;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscMetadata {
    pub release_id: String,
    pub cover_url: Option<String>,
    pub title: String,
    pub artist: String,
    pub year: u16,
    pub disc_number: u32,
    pub tracks: Vec<TrackMetadata>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackMetadata {
    pub title: String,
    pub artist: String,
    pub artists: Vec<String>,
    pub recording_id: Option<String>,
    pub track_id: Option<String>,
}

impl DiscMetadata {
    pub fn apply(&self, snapshot: &mut LibrarySnapshot) {
        if snapshot.tracks.len() != self.tracks.len() {
            return;
        }
        for album in &mut snapshot.albums {
            album.title = self.title.clone();
            album.artist = self.artist.clone();
            album.year = self.year;
            album.cover_path = self.cover_url.as_ref().map(std::path::PathBuf::from);
        }
        for (track, name) in snapshot.tracks.iter_mut().zip(&self.tracks) {
            track.title = name.title.clone();
            track.artist = name.artist.clone();
            track.artists = name.artists.clone();
            track.credits = name
                .artists
                .iter()
                .map(reader::ArtistCredit::unlinked)
                .collect();
            track.album = self.title.clone();
            track.disc_number = Some(self.disc_number);
            track.musicbrainz_release_id = Some(self.release_id.clone());
            track.musicbrainz_recording_id = name.recording_id.clone();
            track.musicbrainz_track_id = name.track_id.clone();
            track.cover = self.cover_url.clone();
        }
    }
}

/// MusicBrainz uses sector offsets including the 150-sector lead-in and its
/// own Base64 alphabet. The internal SHA-256 track identity remains unchanged.
/// See https://musicbrainz.org/doc/Disc_ID_Calculation.
pub fn disc_id(disc: &Disc) -> Option<String> {
    let audio: Vec<_> = disc.tracks.iter().filter(|track| track.audio).collect();
    let first = audio.first()?;
    let last = audio.last()?;
    if first.number == 0
        || last.number > 99
        || audio
            .windows(2)
            .any(|pair| pair[1].number != pair[0].number + 1)
    {
        return None;
    }
    let mut hash = Sha1::new();
    hash.update(format!(
        "{:02X}{:02X}{:08X}",
        first.number,
        last.number,
        last.end.checked_add(150)?
    ));
    for number in 1..=99 {
        let offset = match audio.iter().find(|track| track.number == number) {
            Some(track) => track.start.checked_add(150)?,
            None => 0,
        };
        hash.update(format!("{offset:08X}"));
    }
    Some(
        base64::engine::general_purpose::STANDARD
            .encode(hash.finalize())
            .replace('+', ".")
            .replace('/', "_")
            .replace('=', "-"),
    )
}

static REQUESTS: Mutex<Option<tokio::time::Instant>> = Mutex::const_new(None);
static CACHE: LazyLock<Mutex<HashMap<String, DiscMetadata>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub async fn lookup(disc: &Disc) -> Option<DiscMetadata> {
    let id = disc_id(disc)?;
    if let Some(metadata) = CACHE.lock().await.get(&id).cloned() {
        return Some(metadata);
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?;
    {
        // Serialize request starts, including multiple drives inserted together.
        let mut last = REQUESTS.lock().await;
        if let Some(at) = *last {
            tokio::time::sleep_until(at + Duration::from_millis(1100)).await;
        }
        *last = Some(tokio::time::Instant::now());
    }
    let mut response = client
        .get(format!("https://musicbrainz.org/ws/2/discid/{id}"))
        .header(
            reqwest::header::USER_AGENT,
            concat!(
                "Kopuz/",
                env!("CARGO_PKG_VERSION"),
                " (https://github.com/Kopuz-org/kopuz)"
            ),
        )
        .query(&[
            ("fmt", "json"),
            ("inc", "recordings+artist-credits"),
            ("cdstubs", "no"),
        ])
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len() + chunk.len() > 8 * 1024 * 1024 {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    let metadata = parse(
        &body,
        &id,
        disc.tracks.iter().filter(|track| track.audio).count(),
    )?;
    let mut cache = CACHE.lock().await;
    if cache.len() >= 32 {
        cache.clear();
    }
    cache.insert(id, metadata.clone());
    Some(metadata)
}

#[derive(Deserialize)]
struct Lookup {
    id: String,
    #[serde(default)]
    releases: Vec<Release>,
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Release {
    id: String,
    title: String,
    date: Option<String>,
    status: Option<String>,
    #[serde(default)]
    cover_art_archive: CoverArt,
    #[serde(default)]
    artist_credit: Vec<Credit>,
    #[serde(default)]
    media: Vec<Medium>,
}
#[derive(Default, Deserialize)]
struct CoverArt {
    #[serde(default)]
    front: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Medium {
    position: u32,
    track_count: usize,
    #[serde(default)]
    discs: Vec<DiscRef>,
    #[serde(default)]
    tracks: Vec<Track>,
}
#[derive(Deserialize)]
struct DiscRef {
    id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Track {
    id: Option<String>,
    title: Option<String>,
    position: usize,
    #[serde(default)]
    artist_credit: Vec<Credit>,
    recording: Option<Recording>,
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Recording {
    id: Option<String>,
    title: Option<String>,
    #[serde(default)]
    artist_credit: Vec<Credit>,
}
#[derive(Deserialize)]
struct Credit {
    name: Option<String>,
    #[serde(default)]
    joinphrase: String,
    artist: Artist,
}
#[derive(Deserialize)]
struct Artist {
    name: String,
}

fn artist(credits: &[Credit]) -> String {
    credits
        .iter()
        .map(|credit| {
            format!(
                "{}{}",
                credit.name.as_deref().unwrap_or(&credit.artist.name),
                credit.joinphrase
            )
        })
        .collect::<String>()
}

fn parse(body: &[u8], id: &str, count: usize) -> Option<DiscMetadata> {
    let mut lookup: Lookup = serde_json::from_slice(body).ok()?;
    if lookup.id != id {
        return None;
    }
    lookup.releases.sort_by(|a, b| {
        (
            a.status.as_deref() != Some("Official"),
            a.date.as_deref().unwrap_or("9999"),
            &a.id,
        )
            .cmp(&(
                b.status.as_deref() != Some("Official"),
                b.date.as_deref().unwrap_or("9999"),
                &b.id,
            ))
    });
    let mut chosen: Option<DiscMetadata> = None;
    for release in lookup.releases {
        for mut medium in release.media {
            if medium.track_count != count
                || medium.tracks.len() != count
                || !medium.discs.iter().any(|disc| disc.id == id)
            {
                continue;
            }
            medium.tracks.sort_by_key(|track| track.position);
            if medium
                .tracks
                .iter()
                .enumerate()
                .any(|(index, track)| track.position != index + 1)
            {
                continue;
            }
            let mut names = Vec::new();
            for track in medium.tracks {
                let recording = track.recording.as_ref();
                let Some(title) = track
                    .title
                    .as_deref()
                    .filter(|title| !title.trim().is_empty())
                    .or_else(|| recording.and_then(|r| r.title.as_deref()))
                else {
                    break;
                };
                let credits = if !track.artist_credit.is_empty() {
                    &track.artist_credit
                } else if let Some(recording) = recording.filter(|r| !r.artist_credit.is_empty()) {
                    &recording.artist_credit
                } else {
                    &release.artist_credit
                };
                names.push(TrackMetadata {
                    title: title.to_string(),
                    artist: artist(credits),
                    artists: credits
                        .iter()
                        .map(|c| c.name.as_ref().unwrap_or(&c.artist.name).clone())
                        .collect(),
                    recording_id: recording.and_then(|r| r.id.clone()),
                    track_id: track.id,
                });
            }
            if names.len() != count || release.title.trim().is_empty() {
                continue;
            }
            let candidate = DiscMetadata {
                release_id: release.id.clone(),
                cover_url: if release.cover_art_archive.front {
                    uuid::Uuid::parse_str(&release.id)
                        .ok()
                        .map(|id| format!("https://coverartarchive.org/release/{id}/front-1200"))
                } else {
                    None
                },
                title: release.title.clone(),
                artist: artist(&release.artist_credit),
                year: release
                    .date
                    .as_deref()
                    .and_then(|date| date.get(..4))
                    .and_then(|year| year.parse().ok())
                    .unwrap_or_default(),
                disc_number: medium.position.max(1),
                tracks: names,
            };
            if let Some(previous) = &chosen {
                // Different pressings of the same album are fine; conflicting names aren't.
                if previous.title != candidate.title
                    || previous.artist != candidate.artist
                    || previous
                        .tracks
                        .iter()
                        .zip(&candidate.tracks)
                        .any(|(a, b)| a.title != b.title || a.artist != b.artist)
                {
                    return None;
                }
            } else {
                chosen = Some(candidate);
            }
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::super::{Disc, DiscTrack};
    use super::*;

    fn example() -> Disc {
        let offsets = [0, 15213, 32164, 46442, 63264, 80339, 95312];
        Disc::new(
            offsets
                .windows(2)
                .enumerate()
                .map(|(n, offsets)| DiscTrack {
                    number: n as u8 + 1,
                    start: offsets[0],
                    end: offsets[1],
                    audio: true,
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn audio_cd_musicbrainz_id_matches_published_toc_example() {
        assert_eq!(disc_id(&example()).unwrap(), "49HHV7Eb8UKF3aQiNmu1GR8vKTY-");
        let mut disc = example();
        // A separate data session does not change the audio Disc ID.
        disc.tracks.push(DiscTrack {
            number: 7,
            start: 106712,
            end: 120000,
            audio: false,
        });
        assert_eq!(disc_id(&disc), disc_id(&example()));
    }

    fn response() -> serde_json::Value {
        serde_json::json!({"id":"matching-disc", "releases":[{
            "id":"11111111-1111-1111-1111-111111111111", "title":"Test Album", "date":"2001-02-03", "status":"Official",
            "cover-art-archive":{"front":true},
            "artist-credit":[{"name":"Album Artist", "artist":{"name":"Artist"}}],
            "media":[{"position":2, "track-count":2, "discs":[{"id":"matching-disc"}], "tracks":[
                {"position":2,"id":"track-two", "title":"Second", "recording":{"id":"recording-two"}},
                {"position":1,"id":"track-one", "title":"First", "artist-credit":[{"name":"Guest", "artist":{"name":"Guest"}}],"recording":{"id":"recording-one"}}
            ]}]
        }]})
    }

    #[test]
    fn audio_cd_recognition_maps_titles_artists_disc_numbers_and_ids() {
        let parsed = parse(
            &serde_json::to_vec(&response()).unwrap(),
            "matching-disc",
            2,
        )
        .unwrap();
        let disc = Disc::new(vec![
            DiscTrack {
                number: 1,
                start: 0,
                end: 75,
                audio: true,
            },
            DiscTrack {
                number: 2,
                start: 75,
                end: 150,
                audio: true,
            },
        ])
        .unwrap();
        let mut rows = crate::source::audio_cd_snapshot(disc);
        let key = rows.tracks[0].id.clone();
        parsed.apply(&mut rows);
        assert_eq!(rows.albums[0].title, "Test Album");
        assert_eq!(rows.albums[0].year, 2001);
        let expected = reader::CoverRef::EmbeddedUrl(
            "https://coverartarchive.org/release/11111111-1111-1111-1111-111111111111/front-1200"
                .into(),
        );
        assert_eq!(reader::CoverRef::for_track(&rows.tracks[0]), expected);
        assert_eq!(
            reader::CoverRef::parse(
                &rows.albums[0]
                    .cover_path
                    .as_ref()
                    .unwrap()
                    .to_string_lossy()
            ),
            expected
        );
        assert_eq!(rows.tracks[0].title, "First");
        assert_eq!(rows.tracks[0].artist, "Guest");
        assert_eq!(rows.tracks[1].artist, "Album Artist");
        assert_eq!(rows.tracks[1].disc_number, Some(2));
        assert_eq!(
            rows.tracks[0].musicbrainz_recording_id.as_deref(),
            Some("recording-one")
        );
        assert_eq!(rows.tracks[0].id, key);
    }

    #[test]
    fn audio_cd_missing_artwork_keeps_recognition_usable() {
        let mut value = response();
        for art in [serde_json::json!({"front":false}), serde_json::json!({})] {
            value["releases"][0]["cover-art-archive"] = art;
            let metadata = parse(&serde_json::to_vec(&value).unwrap(), "matching-disc", 2).unwrap();
            assert_eq!(metadata.title, "Test Album");
            assert!(metadata.cover_url.is_none());
        }
        value["releases"][0]["cover-art-archive"] = serde_json::json!({"front":true});
        value["releases"][0]["id"] = "invalid-release-id".into();
        assert!(
            parse(&serde_json::to_vec(&value).unwrap(), "matching-disc", 2)
                .unwrap()
                .cover_url
                .is_none()
        );
    }

    #[test]
    fn audio_cd_recognition_rejects_mismatches_and_conflicting_releases() {
        let value = response();
        let encoded = serde_json::to_vec(&value).unwrap();
        assert!(parse(&encoded, "other-disc", 2).is_none());
        assert!(parse(&encoded, "matching-disc", 3).is_none());
        let mut bad_medium = value.clone();
        bad_medium["releases"][0]["media"][0]["discs"][0]["id"] = "other-disc".into();
        assert!(
            parse(
                &serde_json::to_vec(&bad_medium).unwrap(),
                "matching-disc",
                2
            )
            .is_none()
        );
        let mut conflict = value.clone();
        let mut second = value["releases"][0].clone();
        second["title"] = "Unrelated Album".into();
        conflict["releases"].as_array_mut().unwrap().push(second);
        assert!(parse(&serde_json::to_vec(&conflict).unwrap(), "matching-disc", 2).is_none());
    }

    #[tokio::test]
    #[ignore = "queries the live MusicBrainz service"]
    async fn audio_cd_musicbrainz_live_lookup() {
        let result = lookup(&example())
            .await
            .expect("published example is recognized");
        assert_eq!(result.title, "Ettella Diamant");
        assert_eq!(result.tracks.len(), 6);
    }
}
