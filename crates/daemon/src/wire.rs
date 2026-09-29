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
        credits: credits(track, config),
    }
}

/// Every credit in billing order, with the artist it opens where the daemon can tell which that is.
fn credits(track: &Track, config: &config::AppConfig) -> Vec<api::ArtistCredit> {
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
    // A row the library never stored is one the active source just listed, unless another service issued it.
    let listed_here = track.id.service() == config.active_service();
    track
        .credits
        .iter()
        .map(|credit| api::ArtistCredit {
            name: credit.name.clone(),
            key: match (&credit.library, credit.id.as_deref()) {
                (Some(filed), id) => Some(crate::artist_key::of_library(filed, id)),
                (None, Some(id)) if listed_here => {
                    Some(crate::artist_key::issued(&config.active_source, id))
                }
                (None, _) => None,
            },
        })
        .collect()
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
    use super::credits;
    use reader::{ArtistCredit, Track, TrackId};

    fn track(id: TrackId, credits: Vec<ArtistCredit>) -> Track {
        Track {
            id,
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

    fn yt_config() -> config::AppConfig {
        let mut config = config::AppConfig::default();
        config.set_active_server_snapshot(config::MusicServer {
            id: Some("srv-1".into()),
            name: "brave".into(),
            url: String::new(),
            service: config::MusicService::YtMusic,
            ..Default::default()
        });
        config
    }

    fn yt(item_id: &str) -> TrackId {
        TrackId::Server {
            service: config::MusicService::YtMusic,
            item_id: item_id.into(),
        }
    }

    /// An id means nothing to a source that did not issue it, and only the daemon can tell whose a row is.
    #[test]
    fn a_listed_id_from_another_service_opens_nothing() {
        let config = yt_config();
        let foreign = track(
            TrackId::Server {
                service: config::MusicService::Jellyfin,
                item_id: "j-1".into(),
            },
            vec![ArtistCredit::linked("Ada", "jf-ada")],
        );

        let sent = credits(&foreign, &config);

        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].name.as_str(), &sent[0].key), ("Ada", &None));
    }

    #[test]
    fn a_listed_id_from_the_active_source_opens_its_artist() {
        let config = yt_config();
        let own = track(yt("y-1"), vec![ArtistCredit::linked("Ada", "UC-ada")]);

        let sent = credits(&own, &config);

        let expected = crate::artist_key::issued(&config.active_source, "UC-ada");
        assert_eq!(sent[0].key, Some(expected));
    }

    /// A stored row says which source filed it, so switching sources never re-keys it under the new one.
    #[test]
    fn a_stored_credit_is_keyed_by_the_row_it_is_filed_under() {
        let config = yt_config();
        let filed = |pk: i64| {
            Some(reader::LibraryArtist {
                pk,
                source: "srv-0".into(),
            })
        };
        let stored = track(
            yt("y-1"),
            vec![
                ArtistCredit {
                    library: filed(3),
                    ..ArtistCredit::linked("Ada", "UC-ada")
                },
                ArtistCredit {
                    library: filed(4),
                    ..ArtistCredit::unlinked("Boris")
                },
            ],
        );

        let sent = credits(&stored, &config);

        let elsewhere = config::Source::Server("srv-0".into());
        assert_eq!(
            sent[0].key,
            Some(crate::artist_key::issued(&elsewhere, "UC-ada"))
        );
        assert_eq!(sent[1].key, Some(crate::artist_key::library(4)));
    }

    /// A row that only names its artists still lists them all; there is nothing to open them by.
    #[test]
    fn a_row_without_credits_sends_its_names_unkeyed() {
        let config = yt_config();

        let sent = credits(&track(yt("y-2"), Vec::new()), &config);

        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].name.as_str(), &sent[0].key), ("Ada", &None));
    }
}
