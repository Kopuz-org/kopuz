use super::*;
use crate::*;

pub fn queue_context_to_proto(value: &api::QueueContext) -> QueueContext {
    let kind = match value {
        api::QueueContext::Tracks { keys } => {
            queue_context::Kind::Tracks(queue_context::TrackKeys { keys: keys.clone() })
        }
        api::QueueContext::Album { id } => {
            queue_context::Kind::Album(queue_context::Id { id: id.clone() })
        }
        api::QueueContext::Artist { artist } => queue_context::Kind::ArtistKey(artist.to_string()),
        api::QueueContext::Genre { name } => {
            queue_context::Kind::Genre(queue_context::Name { name: name.clone() })
        }
        api::QueueContext::Playlist { id } => {
            queue_context::Kind::Playlist(queue_context::Id { id: id.clone() })
        }
        api::QueueContext::Filter { filter } => {
            queue_context::Kind::Filter(track_filter_to_proto(filter))
        }
        api::QueueContext::Radio {
            station_id,
            stream_id,
        } => queue_context::Kind::Radio(queue_context::Radio {
            station_id: station_id.clone(),
            stream_id: stream_id.clone(),
        }),
        api::QueueContext::TrackRadio { key } => {
            queue_context::Kind::TrackRadio(queue_context::Key { key: key.clone() })
        }
        api::QueueContext::PlaylistRadio { id } => {
            queue_context::Kind::PlaylistRadio(queue_context::Id { id: id.clone() })
        }
        api::QueueContext::AlbumRadio { id } => {
            queue_context::Kind::AlbumRadio(queue_context::Id { id: id.clone() })
        }
        api::QueueContext::ArtistRadio { artist } => {
            queue_context::Kind::ArtistRadio(queue_context::Key {
                key: artist.clone(),
            })
        }
        api::QueueContext::MixRadio { keys } => {
            queue_context::Kind::MixRadio(queue_context::TrackKeys { keys: keys.clone() })
        }
        api::QueueContext::SonicPath { from, to } => {
            queue_context::Kind::SonicPath(queue_context::SonicPath {
                from: from.clone(),
                to: to.clone(),
            })
        }
        api::QueueContext::Described { query } => queue_context::Kind::Described(query.clone()),
        api::QueueContext::Fingerprint => {
            queue_context::Kind::Fingerprint(queue_context::Fingerprint {})
        }
    };
    QueueContext { kind: Some(kind) }
}

pub fn queue_context_from_proto(value: &QueueContext) -> Option<api::QueueContext> {
    Some(match value.kind.as_ref()? {
        queue_context::Kind::Tracks(tracks) => api::QueueContext::Tracks {
            keys: tracks.keys.clone(),
        },
        queue_context::Kind::Album(id) => api::QueueContext::Album { id: id.id.clone() },
        queue_context::Kind::ArtistKey(key) => api::QueueContext::Artist {
            artist: key.clone(),
        },
        queue_context::Kind::Genre(name) => api::QueueContext::Genre {
            name: name.name.clone(),
        },
        queue_context::Kind::Playlist(id) => api::QueueContext::Playlist { id: id.id.clone() },
        queue_context::Kind::Filter(filter) => api::QueueContext::Filter {
            filter: track_filter_from_proto(filter),
        },
        queue_context::Kind::Radio(radio) => api::QueueContext::Radio {
            station_id: radio.station_id.clone(),
            stream_id: radio.stream_id.clone(),
        },
        queue_context::Kind::TrackRadio(seed) => api::QueueContext::TrackRadio {
            key: seed.key.clone(),
        },
        queue_context::Kind::PlaylistRadio(seed) => api::QueueContext::PlaylistRadio {
            id: seed.id.clone(),
        },
        queue_context::Kind::AlbumRadio(seed) => api::QueueContext::AlbumRadio {
            id: seed.id.clone(),
        },
        queue_context::Kind::ArtistRadio(seed) => api::QueueContext::ArtistRadio {
            artist: seed.key.clone(),
        },
        queue_context::Kind::MixRadio(seed) => api::QueueContext::MixRadio {
            keys: seed.keys.clone(),
        },
        queue_context::Kind::SonicPath(path) => api::QueueContext::SonicPath {
            from: path.from.clone(),
            to: path.to.clone(),
        },
        queue_context::Kind::Described(query) => api::QueueContext::Described {
            query: query.clone(),
        },
        queue_context::Kind::Fingerprint(_) => api::QueueContext::Fingerprint,
    })
}

pub fn mix_basis_to_proto(value: Option<api::MixBasis>) -> MixBasis {
    match value {
        Some(api::MixBasis::Sonic) => MixBasis::Sonic,
        Some(api::MixBasis::Metadata) => MixBasis::Metadata,
        None => MixBasis::Unspecified,
    }
}

pub fn mix_basis_from_proto(value: i32) -> Option<api::MixBasis> {
    match MixBasis::try_from(value) {
        Ok(MixBasis::Sonic) => Some(api::MixBasis::Sonic),
        Ok(MixBasis::Metadata) => Some(api::MixBasis::Metadata),
        Ok(MixBasis::Unspecified) | Err(_) => None,
    }
}

pub fn set_queue_to_proto(value: &api::SetQueueRequest) -> SetQueueRequest {
    SetQueueRequest {
        mode: queue_mode_to_proto(value.mode) as i32,
        context: Some(queue_context_to_proto(&value.context)),
        start_index: value.start_index,
        shuffle: value.shuffle,
    }
}

pub fn set_queue_from_proto(value: &SetQueueRequest) -> Option<api::SetQueueRequest> {
    Some(api::SetQueueRequest {
        mode: queue_mode_from_proto(value.mode),
        context: queue_context_from_proto(value.context.as_ref()?)?,
        start_index: value.start_index,
        shuffle: value.shuffle,
    })
}

pub fn queue_edit_to_proto(value: &api::QueueEdit) -> QueueEditRequest {
    let op = match value {
        api::QueueEdit::Jump { index } => {
            queue_edit_request::Op::Jump(queue_edit_request::Jump { index: *index })
        }
        api::QueueEdit::Move { from, to } => {
            queue_edit_request::Op::Move(queue_edit_request::Move {
                from: *from,
                to: *to,
            })
        }
        api::QueueEdit::Remove { index } => {
            queue_edit_request::Op::Remove(queue_edit_request::Remove { index: *index })
        }
        api::QueueEdit::JumpPhysical { index } => {
            queue_edit_request::Op::JumpPhysical(queue_edit_request::Jump { index: *index })
        }
        api::QueueEdit::Insert { index, keys } => {
            queue_edit_request::Op::Insert(queue_edit_request::Insert {
                index: *index,
                keys: keys.clone(),
            })
        }
        api::QueueEdit::ClearUpcoming => {
            queue_edit_request::Op::ClearUpcoming(queue_edit_request::ClearUpcoming {})
        }
        api::QueueEdit::ReplaceUpcoming { keys } => {
            queue_edit_request::Op::ReplaceUpcoming(queue_edit_request::ReplaceUpcoming {
                keys: keys.clone(),
            })
        }
        api::QueueEdit::ShuffleUpcoming => {
            queue_edit_request::Op::ShuffleUpcoming(queue_edit_request::ShuffleUpcoming {})
        }
    };
    QueueEditRequest { op: Some(op) }
}

pub fn queue_edit_from_proto(value: &QueueEditRequest) -> Option<api::QueueEdit> {
    Some(match value.op.as_ref()? {
        queue_edit_request::Op::Jump(jump) => api::QueueEdit::Jump { index: jump.index },
        queue_edit_request::Op::Move(mv) => api::QueueEdit::Move {
            from: mv.from,
            to: mv.to,
        },
        queue_edit_request::Op::Remove(remove) => api::QueueEdit::Remove {
            index: remove.index,
        },
        queue_edit_request::Op::JumpPhysical(jump) => {
            api::QueueEdit::JumpPhysical { index: jump.index }
        }
        queue_edit_request::Op::Insert(insert) => api::QueueEdit::Insert {
            index: insert.index,
            keys: insert.keys.clone(),
        },
        queue_edit_request::Op::ClearUpcoming(_) => api::QueueEdit::ClearUpcoming,
        queue_edit_request::Op::ReplaceUpcoming(replace) => api::QueueEdit::ReplaceUpcoming {
            keys: replace.keys.clone(),
        },
        queue_edit_request::Op::ShuffleUpcoming(_) => api::QueueEdit::ShuffleUpcoming,
    })
}

pub fn queue_window_to_proto(value: &api::QueueWindow) -> QueueWindow {
    QueueWindow {
        rev: value.rev,
        total: value.total,
        offset: value.offset,
        items: value
            .items
            .iter()
            .map(|item| QueueItem {
                index: item.index,
                track: Some(track_info_to_proto(&item.track)),
            })
            .collect(),
    }
}

pub fn queue_window_from_proto(value: &QueueWindow) -> api::QueueWindow {
    api::QueueWindow {
        rev: value.rev,
        total: value.total,
        offset: value.offset,
        items: value
            .items
            .iter()
            .map(|item| api::QueueItem {
                index: item.index,
                track: item
                    .track
                    .as_ref()
                    .map(track_info_from_proto)
                    .unwrap_or_default(),
            })
            .collect(),
    }
}

pub fn queue_snapshot_to_proto(value: &api::QueueSnapshot) -> QueueSnapshot {
    QueueSnapshot {
        rev: value.rev,
        items: value.items.iter().map(track_info_to_proto).collect(),
        shuffle_order: value.shuffle_order.clone(),
        position: value.position,
        shuffle: value.shuffle,
        context: value.context.as_ref().map(queue_context_to_proto),
    }
}

pub fn queue_snapshot_from_proto(value: &QueueSnapshot) -> api::QueueSnapshot {
    api::QueueSnapshot {
        rev: value.rev,
        items: value.items.iter().map(track_info_from_proto).collect(),
        shuffle_order: value.shuffle_order.clone(),
        position: value.position,
        shuffle: value.shuffle,
        context: value.context.as_ref().and_then(queue_context_from_proto),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        let request = api::SetQueueRequest {
            mode: api::QueueMode::PlayNext,
            context: api::QueueContext::Radio {
                station_id: "s".into(),
                stream_id: "st".into(),
            },
            start_index: Some(2),
            shuffle: Some(false),
        };
        let back = set_queue_from_proto(&set_queue_to_proto(&request)).expect("request survives");
        assert_eq!(request, back);

        let edit = api::QueueEdit::Move { from: 1, to: 3 };
        let back = queue_edit_from_proto(&queue_edit_to_proto(&edit)).expect("edit survives");
        assert_eq!(edit, back);
    }

    #[test]
    fn every_queue_edit_round_trips() {
        let edits = [
            api::QueueEdit::Jump { index: 1 },
            api::QueueEdit::JumpPhysical { index: 2 },
            api::QueueEdit::Move { from: 1, to: 3 },
            api::QueueEdit::Remove { index: 4 },
            api::QueueEdit::Insert {
                index: 2,
                keys: vec!["a".into(), "b".into()],
            },
            api::QueueEdit::ClearUpcoming,
            api::QueueEdit::ReplaceUpcoming {
                keys: vec!["x".into(), "y".into()],
            },
            api::QueueEdit::ReplaceUpcoming { keys: Vec::new() },
            api::QueueEdit::ShuffleUpcoming,
        ];
        for edit in edits {
            let back = queue_edit_from_proto(&queue_edit_to_proto(&edit)).expect("edit survives");
            assert_eq!(edit, back);
        }
    }

    #[test]
    fn a_mix_context_and_its_basis_survive_the_wire() {
        let contexts = [
            api::QueueContext::AlbumRadio { id: "al".into() },
            api::QueueContext::ArtistRadio {
                artist: "ar".into(),
            },
            api::QueueContext::MixRadio {
                keys: vec!["a".into(), "b".into()],
            },
            api::QueueContext::SonicPath {
                from: "a".into(),
                to: "b".into(),
            },
            api::QueueContext::Described {
                query: "calm piano".into(),
            },
            api::QueueContext::Fingerprint,
        ];
        for context in contexts {
            let back = queue_context_from_proto(&queue_context_to_proto(&context));
            assert_eq!(back, Some(context));
        }

        for basis in [
            None,
            Some(api::MixBasis::Sonic),
            Some(api::MixBasis::Metadata),
        ] {
            assert_eq!(
                mix_basis_from_proto(mix_basis_to_proto(basis) as i32),
                basis
            );
        }
    }

    #[test]
    fn a_snapshot_keeps_where_it_was_built_from() {
        let origins = [
            None,
            Some(api::QueueContext::Album { id: "a".into() }),
            Some(api::QueueContext::Artist {
                artist: "UC-x".into(),
            }),
            Some(api::QueueContext::Genre { name: "g".into() }),
            Some(api::QueueContext::Playlist { id: "p".into() }),
            Some(api::QueueContext::Filter {
                filter: api::TrackFilter {
                    favorite: Some(true),
                    ..Default::default()
                },
            }),
            Some(api::QueueContext::Radio {
                station_id: "s".into(),
                stream_id: "st".into(),
            }),
            Some(api::QueueContext::TrackRadio { key: "k".into() }),
            Some(api::QueueContext::PlaylistRadio { id: "p".into() }),
            Some(api::QueueContext::AlbumRadio { id: "al".into() }),
            Some(api::QueueContext::ArtistRadio {
                artist: "ar".into(),
            }),
        ];
        for context in origins {
            let snapshot = api::QueueSnapshot {
                rev: 3,
                context: context.clone(),
                ..Default::default()
            };
            assert_eq!(
                queue_snapshot_from_proto(&queue_snapshot_to_proto(&snapshot)),
                snapshot,
                "{context:?}"
            );
        }
    }
}
