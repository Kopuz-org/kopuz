//! Playback classification from domain identities and resolved stream markers.

use std::path::Path;

use reader::TrackId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackItemRef<'a> {
    Local(&'a Path),
    Server {
        item_id: &'a str,
    },
    Radio {
        station_id: &'a str,
        stream_id: &'a str,
    },
}

impl<'a> PlaybackItemRef<'a> {
    pub fn from_id(id: &'a TrackId) -> Self {
        match id {
            TrackId::Server { item_id, .. } => Self::Server { item_id },
            TrackId::Local(path) => match path
                .to_str()
                .and_then(|value| value.strip_prefix("radio:"))
                .and_then(|value| value.split_once(':'))
            {
                Some((station_id, stream_id)) => Self::Radio {
                    station_id,
                    stream_id,
                },
                None => Self::Local(path),
            },
        }
    }

    pub fn is_radio(self) -> bool {
        matches!(self, Self::Radio { .. })
    }

    pub fn is_server(self) -> bool {
        matches!(self, Self::Server { .. })
    }

    pub fn primary_id(self) -> Option<&'a str> {
        match self {
            Self::Server { item_id, .. } => Some(item_id),
            Self::Radio { station_id, .. } => Some(station_id),
            Self::Local(_) => None,
        }
    }

    pub fn stream_id(self) -> Option<&'a str> {
        match self {
            Self::Radio { stream_id, .. } => Some(stream_id),
            Self::Server { .. } | Self::Local(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedStreamRef<'a> {
    Pending(&'a str),
    SoundCloudHls(&'a str),
    /// Apple Music encrypted fMP4. Payload is
    /// `adam_id:storefront:language:base64_media_user_token` — the ref carries
    /// everything the decode factory needs, so it never re-reads the config.
    /// Written by `AppleMusicSource::resolve_stream`.
    AppleMusicFmp4(&'a str),
    Direct(&'a str),
}

impl<'a> ResolvedStreamRef<'a> {
    pub fn pending_marker(item_id: &str) -> String {
        format!("__PENDING:{item_id}")
    }

    pub fn parse(value: &'a str) -> Self {
        if let Some(item_id) = value.strip_prefix("__PENDING:") {
            Self::Pending(item_id)
        } else if let Some(url) = value.strip_prefix("__SC_HLS:") {
            Self::SoundCloudHls(url)
        } else if let Some(payload) = value.strip_prefix("__AM_FMP4:") {
            Self::AppleMusicFmp4(payload)
        } else {
            Self::Direct(value)
        }
    }

    /// Split an [`Self::AppleMusicFmp4`] payload into
    /// `(adam_id, storefront, language, base64_token)`. The token comes last so
    /// its base64 padding can't be mistaken for a field separator.
    pub fn apple_music_parts(payload: &'a str) -> Option<(&'a str, &'a str, &'a str, &'a str)> {
        let mut parts = payload.splitn(4, ':');
        Some((parts.next()?, parts.next()?, parts.next()?, parts.next()?))
    }
}

#[cfg(test)]
mod tests {
    use super::{PlaybackItemRef, ResolvedStreamRef};

    #[test]
    fn parses_stream_markers() {
        assert_eq!(
            ResolvedStreamRef::parse("__PENDING:abc"),
            ResolvedStreamRef::Pending("abc")
        );
        assert_eq!(
            ResolvedStreamRef::parse("__SC_HLS:https://example.invalid/x.m3u8"),
            ResolvedStreamRef::SoundCloudHls("https://example.invalid/x.m3u8")
        );
    }

    #[test]
    fn parses_radio_item_refs() {
        assert_eq!(
            PlaybackItemRef::from_id(&reader::TrackId::Local("radio:station:stream".into())),
            PlaybackItemRef::Radio {
                station_id: "station",
                stream_id: "stream",
            }
        );
    }

    #[test]
    fn server_item_ids_are_opaque_for_every_service() {
        use config::MusicService;
        for service in [
            MusicService::Jellyfin,
            MusicService::Subsonic,
            MusicService::Custom,
            MusicService::YtMusic,
            MusicService::SoundCloud,
            MusicService::AppleMusic,
            MusicService::Spotify,
            MusicService::Nextcloud,
        ] {
            let id = reader::TrackId::Server {
                service,
                item_id: "folder:item:42".into(),
            };
            assert_eq!(
                PlaybackItemRef::from_id(&id),
                PlaybackItemRef::Server {
                    item_id: "folder:item:42"
                }
            );
        }
    }

    #[test]
    fn local_paths_are_not_reclassified_as_services() {
        let id = reader::TrackId::Local("nextcloud:track.flac".into());
        assert_eq!(
            PlaybackItemRef::from_id(&id),
            PlaybackItemRef::Local(std::path::Path::new("nextcloud:track.flac"))
        );
    }
}
