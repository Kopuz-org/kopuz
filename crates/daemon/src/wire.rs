//! Boundary conversion from the internal track model to wire rows.

use api::{TrackInfo, TrackKind};
use reader::Track;
use server::playback_ref::PlaybackItemRef;

/// The wire row for a track: sentinel durations become an explicit kind, and
/// the offline flag is derived from the config's registration map so clients
/// never see paths.
pub(crate) fn track_info(track: &Track, config: &config::AppConfig) -> TrackInfo {
    let key = track.id.key().to_string();
    let uid = track.id.uid();
    let item_ref = PlaybackItemRef::parse(&uid);
    let radio = track.duration == u64::MAX;
    let offline = item_ref
        .primary_id()
        .is_some_and(|id| config.offline_tracks.contains_key(id));
    TrackInfo {
        key,
        uid,
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        album_id: track.album_id.clone(),
        duration_ms: (!radio).then(|| track.duration.saturating_mul(1000)),
        khz: track.khz,
        bitrate: track.bitrate,
        track_number: track.track_number,
        disc_number: track.disc_number,
        kind: if radio {
            TrackKind::Radio
        } else {
            TrackKind::Normal
        },
        seekable: !radio,
        offline,
        format: track_format(track),
        musicbrainz_release_id: track.musicbrainz_release_id.clone(),
        musicbrainz_recording_id: track.musicbrainz_recording_id.clone(),
        musicbrainz_track_id: track.musicbrainz_track_id.clone(),
        artwork: crate::artwork::track_ref(track),
        credits: credits(track),
    }
}

/// Every credit in billing order, with the artist it opens where its origin says which that is.
fn credits(track: &Track) -> Vec<api::ArtistCredit> {
    if track.credits.is_empty() {
        let named = match track.artists.is_empty() {
            true => std::slice::from_ref(&track.artist),
            false => track.artists.as_slice(),
        };
        return named
            .iter()
            .filter(|name| !name.trim().is_empty())
            .map(|name| api::ArtistCredit {
                name: name.clone(),
                key: None,
            })
            .collect();
    }
    track
        .credits
        .iter()
        .map(|credit| api::ArtistCredit {
            name: credit.name.clone(),
            key: credit.key.clone(),
        })
        .collect()
}

/// Stamp the credits of rows `source` just listed, so their ids stay its own wherever the rows go next.
pub(crate) fn listed_by<'a>(
    source: &config::Source,
    tracks: impl IntoIterator<Item = &'a mut Track>,
) {
    for credit in tracks
        .into_iter()
        .flat_map(|track| track.credits.iter_mut())
    {
        credit.source = Some(source.clone());
    }
}

/// The container a local file is in, for the badge a row shows. Only the
/// formats the player actually decodes are named; anything else, and every
/// track that came from a service, has none.
fn track_format(track: &Track) -> Option<String> {
    let extension = track
        .id
        .local_path()?
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "mp3" | "flac" | "m4a" | "wav" | "ogg" | "opus" | "mp4" | "mka"
    )
    .then(|| extension.to_uppercase())
}

#[cfg(test)]
mod tests {
    use super::{credits, listed_by};
    use config::Source;
    use reader::{ArtistCredit, Track, TrackId};

    fn track(credits: Vec<ArtistCredit>) -> Track {
        Track {
            id: TrackId::Server {
                service: config::MusicService::YtMusic,
                item_id: "y-1".into(),
            },
            cover: None,
            album_id: String::new(),
            title: "t".into(),
            artist: "Ada".into(),
            album: String::new(),
            duration: 1,
            khz: 44,
            bitrate: 320,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: vec!["Ada".into()],
            credits,
        }
    }

    #[test]
    fn a_credit_listed_but_never_filed_carries_its_key_to_the_wire() {
        let credit = ArtistCredit::linked("Ada", "UC-ada");
        assert_eq!(credit.key.as_deref(), Some("UC-ada"));

        let sent = credits(&track(vec![credit, ArtistCredit::unlinked("Boris")]));

        assert_eq!(sent[0].key.as_deref(), Some("UC-ada"));
        assert_eq!(sent[1].key, None);
    }

    #[test]
    fn a_listed_credit_opens_by_the_id_its_source_issued() {
        let yt = Source::Server("yt-1".into());
        let mut row = track(vec![ArtistCredit::linked("Ada", "UC-ada")]);

        listed_by(&yt, [&mut row]);

        let sent = credits(&row);
        assert_eq!(sent[0].key.as_deref(), Some("UC-ada"));
    }

    /// The persisted queue carries the stamp, so a restart or a source switch never re-keys a row.
    #[test]
    fn a_stamp_survives_the_stored_queue() {
        let yt = Source::Server("yt-1".into());
        let mut row = track(vec![ArtistCredit::linked("Ada", "UC-ada")]);
        listed_by(&yt, [&mut row]);

        let stored = serde_json::to_string(&row).unwrap();
        let restored: Track = serde_json::from_str(&stored).unwrap();

        assert_eq!(credits(&restored), credits(&row));
    }

    #[test]
    fn a_stored_credit_opens_by_the_key_it_is_filed_under() {
        let filed = |credit: ArtistCredit, key: &str| ArtistCredit {
            key: Some(key.into()),
            ..credit
        };
        let row = track(vec![
            filed(ArtistCredit::linked("Ada", "UC-ada"), "UC-ada"),
            filed(ArtistCredit::unlinked("Boris"), "9f2b"),
        ]);

        let sent = credits(&row);

        assert_eq!(sent[0].key.as_deref(), Some("UC-ada"));
        assert_eq!(sent[1].key.as_deref(), Some("9f2b"));
    }

    /// A row that only names its artists still lists them all; there is nothing to open them by.
    #[test]
    fn a_row_without_credits_sends_its_names_unkeyed() {
        let sent = credits(&track(Vec::new()));

        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].name.as_str(), &sent[0].key), ("Ada", &None));
    }
}
