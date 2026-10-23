//! What the player is on, by the ids a card is keyed by.

use api::TrackInfo;

/// Two albums sharing a title are two albums, so a card is marked by id and never by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Playing {
    track: Option<String>,
    album: Option<String>,
    artists: Vec<String>,
}

impl Playing {
    pub fn of(track: &TrackInfo) -> Self {
        Self {
            track: (!track.key.is_empty()).then(|| track.key.clone()),
            album: (!track.album_id.is_empty()).then(|| track.album_id.clone()),
            artists: track
                .credits
                .iter()
                .filter_map(|credit| credit.key.clone())
                .collect(),
        }
    }

    pub fn track(&self) -> Option<&str> {
        self.track.as_deref()
    }

    pub fn is_track(&self, key: &str) -> bool {
        self.track.as_deref() == Some(key)
    }

    pub fn is_album(&self, id: &str) -> bool {
        self.album.as_deref() == Some(id)
    }

    pub fn is_artist(&self, key: &str) -> bool {
        self.artists.iter().any(|held| held == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> TrackInfo {
        TrackInfo {
            key: "t1".into(),
            artist: "Boards of Canada".into(),
            album: "Geogaddi".into(),
            album_id: "warp-101".into(),
            credits: vec![api::ArtistCredit {
                name: "Boards of Canada".into(),
                key: Some("boc".into()),
            }],
            ..TrackInfo::default()
        }
    }

    #[test]
    fn the_mark_goes_by_id_not_by_name() {
        let playing = Playing::of(&now());
        assert!(playing.is_album("warp-101"));
        assert!(!playing.is_album("Geogaddi"));
        assert!(playing.is_artist("boc"));
        assert!(!playing.is_artist("Boards of Canada"));
        assert!(playing.is_track("t1"));
    }

    #[test]
    fn an_empty_id_marks_nothing() {
        let playing = Playing::of(&TrackInfo::default());
        assert!(!playing.is_album(""));
        assert!(!playing.is_track(""));
        assert_eq!(playing, Playing::default());
    }
}
