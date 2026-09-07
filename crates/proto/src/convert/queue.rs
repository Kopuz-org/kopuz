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
        api::QueueContext::Artist { name } => {
            queue_context::Kind::Artist(queue_context::Name { name: name.clone() })
        }
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
    };
    QueueContext { kind: Some(kind) }
}

pub fn queue_context_from_proto(value: &QueueContext) -> Option<api::QueueContext> {
    Some(match value.kind.as_ref()? {
        queue_context::Kind::Tracks(tracks) => api::QueueContext::Tracks {
            keys: tracks.keys.clone(),
        },
        queue_context::Kind::Album(id) => api::QueueContext::Album { id: id.id.clone() },
        queue_context::Kind::Artist(name) => api::QueueContext::Artist {
            name: name.name.clone(),
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
    })
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
}
