//! The queue origin as stored: JSON only the daemon reads, so the db never learns the wire shapes.

use api::{QueueContext, TrackFilter, TrackSort};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Stored {
    Album {
        id: String,
    },
    Artist {
        artist: String,
    },
    Genre {
        name: String,
    },
    Playlist {
        id: String,
    },
    Filter {
        search: Option<String>,
        album: Option<String>,
        genre: Option<String>,
        favorite: Option<bool>,
        sort: StoredSort,
        #[serde(default)]
        downloaded: Option<bool>,
        #[serde(default)]
        year_from: Option<u16>,
        #[serde(default)]
        year_to: Option<u16>,
        #[serde(default)]
        reverse: bool,
    },
    Radio {
        station_id: String,
        stream_id: String,
    },
    TrackRadio {
        key: String,
    },
    PlaylistRadio {
        id: String,
    },
}

#[derive(Serialize, Deserialize)]
enum StoredSort {
    Default,
    Title,
    Artist,
    Album,
    DateAdded,
    PlayCount,
    Fields(Vec<config::SortCriterion<config::TrackSortField>>),
}

impl From<&TrackSort> for StoredSort {
    fn from(sort: &TrackSort) -> Self {
        match sort {
            TrackSort::Default => Self::Default,
            TrackSort::Title => Self::Title,
            TrackSort::Artist => Self::Artist,
            TrackSort::Album => Self::Album,
            TrackSort::DateAdded => Self::DateAdded,
            TrackSort::PlayCount => Self::PlayCount,
            TrackSort::Fields(fields) => Self::Fields(fields.clone()),
        }
    }
}

impl From<StoredSort> for TrackSort {
    fn from(sort: StoredSort) -> Self {
        match sort {
            StoredSort::Default => Self::Default,
            StoredSort::Title => Self::Title,
            StoredSort::Artist => Self::Artist,
            StoredSort::Album => Self::Album,
            StoredSort::DateAdded => Self::DateAdded,
            StoredSort::PlayCount => Self::PlayCount,
            StoredSort::Fields(fields) => Self::Fields(fields),
        }
    }
}

/// None for a context that names no container: a raw track list has no origin to keep.
pub(super) fn encode(context: &QueueContext) -> Option<String> {
    let stored = match context {
        QueueContext::Tracks { .. } => return None,
        QueueContext::Album { id } => Stored::Album { id: id.clone() },
        QueueContext::Artist { artist } => Stored::Artist {
            artist: artist.clone(),
        },
        QueueContext::Genre { name } => Stored::Genre { name: name.clone() },
        QueueContext::Playlist { id } => Stored::Playlist { id: id.clone() },
        QueueContext::Filter { filter } => Stored::Filter {
            search: filter.search.clone(),
            album: filter.album.clone(),
            genre: filter.genre.clone(),
            favorite: filter.favorite,
            sort: (&filter.sort).into(),
            downloaded: filter.downloaded,
            year_from: filter.year_from,
            year_to: filter.year_to,
            reverse: filter.reverse,
        },
        QueueContext::Radio {
            station_id,
            stream_id,
        } => Stored::Radio {
            station_id: station_id.clone(),
            stream_id: stream_id.clone(),
        },
        QueueContext::TrackRadio { key } => Stored::TrackRadio { key: key.clone() },
        QueueContext::PlaylistRadio { id } => Stored::PlaylistRadio { id: id.clone() },
    };
    serde_json::to_string(&stored).ok()
}

/// A stored origin the running version cannot read is dropped: the queue is still good without it.
pub(super) fn decode(stored: &str) -> Option<QueueContext> {
    let stored = match serde_json::from_str::<Stored>(stored) {
        Ok(stored) => stored,
        Err(error) => {
            tracing::warn!(%error, "ignoring an unreadable queue origin");
            return None;
        }
    };
    Some(match stored {
        Stored::Album { id } => QueueContext::Album { id },
        Stored::Artist { artist } => QueueContext::Artist { artist },
        Stored::Genre { name } => QueueContext::Genre { name },
        Stored::Playlist { id } => QueueContext::Playlist { id },
        Stored::Filter {
            search,
            album,
            genre,
            favorite,
            sort,
            downloaded,
            year_from,
            year_to,
            reverse,
        } => QueueContext::Filter {
            filter: TrackFilter {
                search,
                album,
                genre,
                favorite,
                sort: sort.into(),
                downloaded,
                year_from,
                year_to,
                reverse,
            },
        },
        Stored::Radio {
            station_id,
            stream_id,
        } => QueueContext::Radio {
            station_id,
            stream_id,
        },
        Stored::TrackRadio { key } => QueueContext::TrackRadio { key },
        Stored::PlaylistRadio { id } => QueueContext::PlaylistRadio { id },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_container_survives_storage() {
        let contexts = [
            QueueContext::Album { id: "a".into() },
            QueueContext::Artist {
                artist: "UC-x".into(),
            },
            QueueContext::Genre { name: "g".into() },
            QueueContext::Playlist { id: "p".into() },
            QueueContext::Filter {
                filter: TrackFilter {
                    search: Some("s".into()),
                    album: Some("a".into()),
                    genre: Some("g".into()),
                    favorite: Some(true),
                    sort: TrackSort::Fields(vec![config::SortCriterion::new(
                        config::TrackSortField::Title,
                        config::SortDirection::Desc,
                    )]),
                    downloaded: Some(false),
                    year_from: Some(1990),
                    year_to: Some(1999),
                    reverse: true,
                },
            },
            QueueContext::Filter {
                filter: TrackFilter::default(),
            },
            QueueContext::Radio {
                station_id: "s".into(),
                stream_id: "st".into(),
            },
            QueueContext::TrackRadio { key: "k".into() },
            QueueContext::PlaylistRadio { id: "p".into() },
        ];
        for context in contexts {
            let stored = encode(&context).expect("a container is stored");
            assert_eq!(decode(&stored), Some(context));
        }
    }

    #[test]
    fn an_origin_stored_before_the_new_filters_still_reads() {
        let stored = r#"{"kind":"filter","search":null,"album":null,"genre":"g","favorite":true,"sort":"Title"}"#;
        assert_eq!(
            decode(stored),
            Some(QueueContext::Filter {
                filter: TrackFilter {
                    genre: Some("g".into()),
                    favorite: Some(true),
                    sort: TrackSort::Title,
                    ..Default::default()
                },
            })
        );
    }

    #[test]
    fn a_raw_track_list_and_garbage_have_no_origin() {
        assert_eq!(
            encode(&QueueContext::Tracks {
                keys: vec!["k".into()]
            }),
            None
        );
        assert_eq!(decode("not json"), None);
        assert_eq!(decode(r#"{"kind":"from_the_future"}"#), None);
    }
}
