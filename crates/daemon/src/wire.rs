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
        artists: track.artists.clone(),
        musicbrainz_release_id: track.musicbrainz_release_id.clone(),
        musicbrainz_recording_id: track.musicbrainz_recording_id.clone(),
        musicbrainz_track_id: track.musicbrainz_track_id.clone(),
        playlist_item_id: track.playlist_item_id.clone(),
        artwork: crate::artwork::track_ref(track),
        credits: credits(track, config),
    }
}

/// Every credit in billing order, whatever shape the row stored. An id belongs
/// to the source that issued it, so one from a row another source owns is left
/// off rather than handed to a frontend that cannot tell the difference.
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
                id: None,
            })
            .collect();
    }
    let active = track.id.service() == config.active_service();
    track
        .credits
        .iter()
        .map(|credit| api::ArtistCredit {
            name: credit.name.clone(),
            id: credit.id.clone().filter(|_| active),
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

    /// An id means nothing to a source that did not issue it, and a frontend
    /// cannot tell which source a row came from -- so the daemon decides.
    #[test]
    fn an_id_from_another_source_is_not_sent() {
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
        assert_eq!(sent[0].name, "Ada");
        assert_eq!(sent[0].id, None);
    }

    #[test]
    fn an_id_from_the_active_source_is_sent() {
        let config = yt_config();
        let own = track(
            TrackId::Server {
                service: config::MusicService::YtMusic,
                item_id: "y-1".into(),
            },
            vec![ArtistCredit::linked("Ada", "UC-ada")],
        );

        let sent = credits(&own, &config);

        assert_eq!(sent[0].id.as_deref(), Some("UC-ada"));
    }

    /// A row stored before the column exists still names its artists, so the
    /// wire always carries a complete list and a frontend needs no fallback.
    #[test]
    fn a_row_without_credits_falls_back_to_the_names() {
        let config = yt_config();
        let bare = track(
            TrackId::Server {
                service: config::MusicService::YtMusic,
                item_id: "y-2".into(),
            },
            Vec::new(),
        );

        let sent = credits(&bare, &config);

        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].name, "Ada");
        assert_eq!(sent[0].id, None);
    }
}
