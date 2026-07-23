#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackItemRef<'a> {
    Local(&'a str),
    Server {
        service: &'a str,
        item_id: &'a str,
        extra: Option<&'a str>,
    },
    Radio {
        station_id: &'a str,
        stream_id: &'a str,
    },
}

impl<'a> PlaybackItemRef<'a> {
    pub fn parse(value: &'a str) -> Self {
        let mut parts = value.split(':');
        let scheme = parts.next().unwrap_or_default();
        match scheme {
            "radio" => Self::Radio {
                station_id: parts.next().unwrap_or_default(),
                stream_id: parts.next().unwrap_or_default(),
            },
            "jellyfin" | "subsonic" | "custom" | "ytmusic" | "soundcloud" | "spotify" => {
                Self::Server {
                    service: scheme,
                    item_id: parts.next().unwrap_or_default(),
                    extra: parts.next(),
                }
            }
            _ => Self::Local(value),
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
            Self::Server { extra, .. } => extra,
            Self::Local(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedStreamRef<'a> {
    Pending(&'a str),
    SoundCloudHls(&'a str),
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
        } else {
            Self::Direct(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PlaybackItemRef, ResolvedStreamRef};

    #[test]
    fn parses_radio_item_refs() {
        assert_eq!(
            PlaybackItemRef::parse("radio:station:stream"),
            PlaybackItemRef::Radio {
                station_id: "station",
                stream_id: "stream",
            }
        );
    }

    #[test]
    fn parses_server_item_refs() {
        assert_eq!(
            PlaybackItemRef::parse("ytmusic:video_id:extra"),
            PlaybackItemRef::Server {
                service: "ytmusic",
                item_id: "video_id",
                extra: Some("extra"),
            }
        );
    }

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
}
